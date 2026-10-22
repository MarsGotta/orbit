//! Real actor + encrypted SQLite + an in-memory MCP peer. No native driver.
#![allow(clippy::expect_used)] // Invalid fixture traffic must fail the test peer.
use super::*;
use crate::daemon::turn::said;
use orbit_core::notice::NoticeCode;

#[tokio::test]
async fn cleanup_notice_is_said_once_and_later_reads_preserve_another_notice() -> Result<()> {
    use orbit_core::topics::control::{EngineCleanup, EngineCleanupOutcome, EngineCleanupReason};

    let mut h = Mac::new(false).await?;
    let mut service = h
        .runtime
        .features
        .computer
        .service
        .take()
        .context("fixture service")?;
    service
        .client_mut()
        .set_startup_cleanup_for_test(vec![EngineCleanup {
        pid: 43_906,
        executable:
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
                .into(),
        reason: EngineCleanupReason::OrphanedFromDaemon,
        outcome: EngineCleanupOutcome::Stopped,
        failure: None,
    }]);
    let generation = h.runtime.state.generation();
    let ticket = h.runtime.begin_computer(
        ComputerStage::Connecting,
        "Conexión sintética",
        generation,
        |_| tokio::spawn(std::future::pending()),
    )?;
    h.runtime
        .computer_event(
            &ticket,
            Ok(ComputerCompletion {
                service,
                result: Ok(ComputerResult::Read(json!({"connected": true}))),
            }),
        )
        .await?;
    // The sweep is said with its count; its PID lines go to daemon.log.
    assert_eq!(h.runtime.state.notice_code(), Some(NoticeCode::EngineSwept));
    let after_connect = h.runtime.state.durable.notice_log.records().len();

    h.runtime.state.set_notice(NoticeCode::AudioGap);
    let after_other_notice = h.runtime.state.durable.notice_log.records().len();
    assert_eq!(after_other_notice, after_connect + 1);
    h.runtime.read_computer_windows()?;
    h.finish().await?;
    h.runtime.read_computer_windows()?;
    h.finish().await?;
    assert_eq!(h.runtime.state.notice_code(), Some(NoticeCode::AudioGap));
    assert_eq!(
        h.runtime.state.durable.notice_log.records().len(),
        after_other_notice
    );
    Ok(())
}

#[tokio::test]
async fn expired_computer_result_retires_authority_with_both_generations() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = crate::daemon::audio::tests::fixture(directory.path())?;
    let started_generation = runtime.state.generation();
    let ticket = runtime.begin_computer(
        ComputerStage::Connecting,
        "Conectando al motor local.",
        started_generation,
        |_| tokio::spawn(std::future::pending()),
    )?;
    {
        let fixture_value = serde_json::json!(runtime.state.generation() + (1));
        crate::daemon::authority_fixture::set(&mut runtime.state, "generation", fixture_value);
    }
    let current_generation = runtime.state.generation();
    runtime
        .computer_event(&ticket, Err(anyhow::anyhow!("synthetic late result")))
        .await?;
    assert!(runtime.features.computer.work.active().is_none());
    let view = runtime
        .features
        .computer
        .view
        .as_ref()
        .context("computer view")?;
    assert_eq!(view.stage, ComputerStage::Cancelled);
    assert!(view.detail.contains(&started_generation.to_string()));
    assert!(view.detail.contains(&current_generation.to_string()));
    Ok(())
}

#[tokio::test]
async fn general_worker_checks_revocation_before_calling_the_tool() -> Result<()> {
    for (revoked, privacy_stopped, changed_generation) in [
        (
            Some(ComputerRevocation::Refusal(
                orbit_core::refusal::Refusal::ReadInterrupted {
                    cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
                },
            )),
            false,
            false,
        ),
        (None, true, false),
        (None, false, true),
    ] {
        let mut h = Mac::new(false).await?;
        h.runtime.call_engine_tool(
            ToolCall {
                tool: "launch_app".into(),
                arguments: json!({"bundle_id":"com.example.fixture"}),
            },
            "fixture",
        )?;
        let authority = h
            .runtime
            .features
            .computer
            .authority
            .as_ref()
            .context("worker authority")?;
        let mut grant = authority.borrow().grant.clone();
        grant.privacy_stopped = privacy_stopped;
        if changed_generation {
            grant.generation = grant.generation.wrapping_add(1);
        }
        authority.send_replace(WorkerAuthority {
            grant,
            revoked: revoked.clone(),
        });
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            h.calls("launch_app"),
            0,
            "revoked={revoked:?} privacy={privacy_stopped} generation={changed_generation}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn other_revocation_answers_remote_action_with_its_cause() -> Result<()> {
    let mut h = Mac::new(true).await?;
    h.start_general_action().await?;
    let (sender, mut frames) = mpsc::channel(4);
    h.runtime.hub.install(sender);
    h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
        id: "remote-revocation".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::PermissionDenied {
            permission: orbit_core::refusal::Permission::Accessibility,
        })
        .await;
    let frame = frames.try_recv().context("Deimos did not get a response")?;
    assert_eq!(frame["params"]["refusal"]["code"], "outcome_unknown");
    assert_eq!(
        frame["params"]["refusal"]["interruption"],
        json!({"code":"permission_denied", "permission":"accessibility", "retry":{"kind":"after_fix"}})
    );
    Ok(())
}

#[tokio::test]
async fn computer_retirement_keeps_non_control_bridge_requests() -> Result<()> {
    for capability in [
        orbit_core::device_bridge::Capability::Screenshot,
        orbit_core::device_bridge::Capability::SelectedText,
        orbit_core::device_bridge::Capability::ScreenSources,
        orbit_core::device_bridge::Capability::CalendarRead,
    ] {
        let mut h = Mac::new(false).await?;
        let (sender, mut frames) = mpsc::channel(4);
        h.runtime.hub.install(sender);
        h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
            id: "unrelated-reading".into(),
            capability,
            application: None,
            title: None,
            full_screen: false,
            source_id: None,
            tool_call: None,
            catalogue_after: 0,
            calendar_window: None,
            calendar_cursor: None,
            files: None,
        });
        h.runtime
            .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
                cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
            })
            .await;
        assert!(frames.try_recv().is_err(), "{capability:?}");
        assert!(
            h.runtime.conversation.bridge_request.is_some(),
            "{capability:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn each_authority_revocation_answers_deimos_with_its_cause() -> Result<()> {
    for change in [
        AuthorityChange::UserStop,
        AuthorityChange::Revoked,
        AuthorityChange::Closing,
        AuthorityChange::Rearm,
        AuthorityChange::RearmWithoutStop,
        AuthorityChange::Interrupt,
        AuthorityChange::HubLink,
        AuthorityChange::HubTransportReady,
        AuthorityChange::HubLinkOffline,
        AuthorityChange::HubDisconnected,
        AuthorityChange::Conversation,
        AuthorityChange::Device,
    ] {
        let mut h = Mac::new(true).await?;
        h.start_general_action().await?;
        let (sender, mut frames) = mpsc::channel(4);
        h.runtime.hub.install(sender);
        h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
            id: "authority-change".into(),
            capability: orbit_core::device_bridge::Capability::PerformAction,
            application: None,
            title: None,
            full_screen: false,
            source_id: None,
            tool_call: None,
            catalogue_after: 0,
            calendar_window: None,
            calendar_cursor: None,
            files: None,
        });
        h.runtime
            .revoke_computer_with_cause(
                super::super::retire::computer_reason(change),
                ComputerRevocation::Authority(change),
            )
            .await;
        let frame = frames.try_recv().context("Deimos did not get a response")?;
        assert_eq!(
            frame["params"]["refusal"]["code"], "outcome_unknown",
            "{change:?}: {frame}"
        );
        let interruption = &frame["params"]["refusal"]["interruption"];
        if matches!(change, AuthorityChange::UserStop | AuthorityChange::Revoked) {
            assert_eq!(interruption["code"], "privacy_stopped");
        } else {
            assert_eq!(interruption["code"], "read_interrupted");
            assert_eq!(interruption["cause"]["kind"], "authority");
            assert_eq!(
                interruption["cause"]["change"],
                serde_json::to_value(change)?
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn engine_refusal_reports_full_and_retired_hub() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let logs = tempfile::tempdir()?;
    said::trace(&mut h.runtime, logs.path());
    let ask = super::super::BridgeAsk {
        id: "engine-refusal".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    h.runtime.hub.install(sender);
    h.runtime.conversation.bridge_request = Some(ask.clone());
    h.runtime.refuse_remote_engine_action("fixture", None).await;
    // Not delivered, with its cause (the full channel) in daemon.log.
    assert_eq!(
        h.runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered),
        "{:?}",
        h.runtime.state.durable.notice
    );
    assert!(said::told(&h.runtime).contains("capacidad 1"));
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());
    h.runtime
        .hub
        .retire(crate::daemon::hub_link::Retirement::NetworkCancelled);
    h.runtime.conversation.bridge_request = Some(ask);
    h.runtime.refuse_remote_engine_action("fixture", None).await;
    assert!(
        said::told(&h.runtime).contains("cancelación de red"),
        "{:?}",
        h.runtime.state.durable.notice
    );
    Ok(())
}
use crate::cua::CuaClient;
use crate::daemon::{Command, Event, audio::tests::fixture};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

#[path = "capture_upload_tests.rs"]
mod capture_upload_tests;
#[path = "catalogue_tests.rs"]
mod catalogue_tests;
#[path = "engine_action_tests.rs"]
mod engine_action_tests;
#[path = "engine_permission_tests.rs"]
mod engine_permission_tests;
#[path = "screen_capture_tests.rs"]
mod screen_capture_tests;

#[test]
fn retry_countdown_does_not_change_the_control_revision() -> Result<()> {
    use crate::daemon::topics::{TopicBoard, TopicSource};
    use orbit_core::{PermissionState, refusal::EngineCause, topics::TopicId};

    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    runtime.features.computer.engine_failure = Some("fixture failure".into());
    runtime.features.computer.engine_failure_kind = Some(EngineCause::InitializationFailed);
    runtime.features.computer.engine_retry_at = Some(Instant::now() + Duration::from_secs(3_600));

    let mut board = TopicBoard::new("retry-fixture".into());
    let _receiver = board.channels()[&TopicId::Control].subscribe();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1_000,
    });
    let first_revision = board.revisions()[&TopicId::Control];
    // A different remaining duration models the next tick without a clock wait.
    runtime.features.computer.engine_retry_at = Some(Instant::now() + Duration::from_secs(1_800));
    board.state_changed();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 2_000,
    });
    assert_eq!(board.revisions()[&TopicId::Control], first_revision);
    Ok(())
}

#[test]
fn updating_the_cursor_entry_keeps_its_history_position() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _) = fixture(directory.path())?;
    for id in ["old", "cursor", "first"] {
        let entry =
            ControlHistoryEntry::new(id, "click", "Fixture", 1_000, ControlOutcome::Unknown)
                .map_err(anyhow::Error::msg)?;
        runtime.upsert_control_history(entry)?;
    }
    let updated = ControlHistoryEntry::new(
        "cursor",
        "click",
        "Fixture",
        1_000,
        ControlOutcome::Performed,
    )
    .map_err(anyhow::Error::msg)?;
    runtime.upsert_control_history(updated)?;
    let history = runtime.control_history()?;
    assert_eq!(
        history
            .iter()
            .map(|entry| entry.operation_id.as_str())
            .collect::<Vec<_>>(),
        ["first", "cursor", "old"]
    );
    assert_eq!(history[1].outcome, ControlOutcome::Performed);
    Ok(())
}

/// A tool no version of Orbit names, declared only by the fixture engine.
const INVENTED_TOOL: &str = "a_tool_invented_by_a_newer_engine";

/// The `query` that makes the fixture engine answer a tree larger than
/// `MAX_ACTION_RECEIPT_BYTES`.
const ENORMOUS_TREE: &str = "arbol-enorme";

struct Mac {
    runtime: Runtime,
    events: mpsc::Receiver<WorkResult>,
    calls: EngineRecord,
    _native: mpsc::Receiver<Value>,
    _directory: tempfile::TempDir,
}

#[path = "fake_engine.rs"]
mod fake_engine;
use fake_engine::*;

impl Mac {
    async fn new(hang_input: bool) -> Result<Self> {
        Self::with_engine(FakeEngine {
            hang_input,
            ..FakeEngine::default()
        })
        .await
    }

    async fn with_engine(engine: FakeEngine) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, native) = fixture(directory.path())?;
        runtime.settings.device_id = Some("mac-fixture".into());
        // A Mac whose engine is connected: its helper is there and reports that
        // macOS grants both permissions the engine needs. A case about a
        // missing or unknown one, or a missing helper, sets it itself.
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        let (work, events) = mpsc::channel(16);
        runtime.work.work_events = work;
        let calls = engine.record.clone();
        let client = engine.connect().await?;
        runtime.features.computer.service = Some(ActionService::from_client(client));
        Ok(Self {
            runtime,
            events,
            calls,
            _native: native,
            _directory: directory,
        })
    }
}

/// An engine tool runs only when macOS grants the Accessibility it needs.
#[tokio::test]
async fn an_engine_tool_requires_macos_accessibility() -> Result<()> {
    let mut h = Mac::new(false).await?;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Denied);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    let refused = h
        .runtime
        .call_engine_tool(
            ToolCall {
                tool: "launch_app".into(),
                arguments: json!({"bundle_id": "com.apple.calculator"}),
            },
            "fixture",
        )
        .err()
        .context("without Accessibility nothing runs")?
        .to_string();
    assert_eq!(
        serde_json::from_str::<Value>(&refused)?["permission"],
        "accessibility"
    );
    assert_eq!(h.calls("launch_app"), 0);

    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    h.runtime.call_engine_tool(
        ToolCall {
            tool: "launch_app".into(),
            arguments: json!({"bundle_id": "com.apple.calculator"}),
        },
        "fixture",
    )?;
    h.finish().await?;
    assert_eq!(h.calls("launch_app"), 1);
    Ok(())
}

impl Mac {
    async fn finish(&mut self) -> Result<()> {
        let event = tokio::time::timeout(Duration::from_secs(2), self.events.recv())
            .await?
            .context("worker absent")?;
        self.runtime.work_event(event).await
    }

    /// An engine tool in flight: the fixture peer leaves `launch_app`
    /// unanswered when the harness hangs its input.
    async fn start_general_action(&mut self) -> Result<()> {
        self.runtime.call_engine_tool(
            ToolCall {
                tool: "launch_app".into(),
                arguments: json!({"bundle_id": "com.example.fixture"}),
            },
            "fixture",
        )?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while self.calls("launch_app") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    fn calls(&self, tool: &str) -> usize {
        self.calls.calls_of(tool).len()
    }

    async fn ask(&mut self, capability: &str, parameters: Value) -> Result<Value> {
        let request_id = id();
        let (hub, mut frames) = mpsc::channel(16);
        self.runtime.hub.install(hub);
        self.runtime
            .device_request_raw(&json!({"request_id":request_id,
            "capability":capability,"parameters":parameters}))
            .await?;
        let frame = frames.try_recv().context("the Mac must answer")?;
        assert_eq!(frame["method"], "device.reply");
        assert_eq!(frame["params"]["request_id"], request_id);
        Ok(frame)
    }
}

// The uncertainty belongs to the connection it happened on: a later connection
// that is stopped with nothing in flight says «cancelada», not «incierto».

#[tokio::test]
async fn revoked_in_flight_history_is_uncertain_and_queryable() -> Result<()> {
    for stop in [Command::Interrupt, Command::PrivacyStop] {
        let mut h = Mac::new(false).await?;
        let entry = ControlHistoryEntry::new(
            "synthetic-operation",
            "click",
            "Fixture",
            1,
            ControlOutcome::Unknown,
        )
        .map_err(anyhow::Error::msg)?;
        h.runtime.upsert_control_history(entry.clone())?;
        h.runtime.features.computer.active_general = Some(entry);
        h.runtime.command(stop).await?;
        let history = h.runtime.control_history()?;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, ControlOutcome::Unknown);
    }
    Ok(())
}

#[tokio::test]
async fn stopped_tick_revokes_control_that_appeared_after_the_stop() -> Result<()> {
    let mut h = Mac::new(true).await?;
    h.start_general_action().await?;
    assert!(h.runtime.features.computer.work.active().is_some());
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "privacy_stopped",
            fixture_value,
        );
    }
    h.runtime.apply(Event::Tick).await?;
    assert!(h.runtime.features.computer.service.is_none());
    assert!(h.runtime.features.computer.work.active().is_none());
    assert_eq!(
        h.runtime.control_history()?[0].outcome,
        ControlOutcome::Unknown
    );
    Ok(())
}

#[test]
fn ipc_observation_is_bounded_without_truncation() {
    assert!(bounded_observation(&"x".repeat(MAX_OBSERVATION_BYTES)).is_err());
    assert_eq!(
        bounded_observation(&json!({"data":"untrusted"})).expect("small JSON"),
        json!({"data":"untrusted"})
    );
}

#[tokio::test]
async fn close_keeps_granted_engine_off_during_maintenance() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    runtime
        .close_for_exit(
            orbit_core::privacy_stop::StopCause::ListenerStopped,
            "Orbit se está cerrando.",
        )
        .await;

    // Check the decision first: removing the closing latch fails here before
    // the maintenance below could start a real process on the test host.
    assert_eq!(
        engine::decide(engine::Facts {
            observed: runtime.engine_observed(),
            closing: runtime.closing,
            running: false,
            retry_due: true,
        }),
        engine::Decision::Keep
    );
    runtime.maintain_engine().await;
    assert!(runtime.features.computer.service.is_none());
    assert!(runtime.features.computer.work.active().is_none());
    Ok(())
}

async fn remote_reply(h: &mut Mac, capability: &str, parameters: Value) -> Result<Value> {
    let (sender, mut hub) = mpsc::channel(16);
    h.runtime.hub.install(sender);
    h.runtime
        .device_request_raw(&json!({
            "request_id":"engine-start", "capability":capability, "parameters":parameters,
        }))
        .await?;
    let frame = hub.try_recv()?;
    assert_eq!(frame["method"], "device.reply");
    Ok(frame["params"].clone())
}

#[tokio::test]
async fn full_screen_with_window_is_an_invalid_parameter() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let reply = remote_reply(
        &mut h,
        "screenshot",
        json!({"full_screen":true,"application":"Fixture"}),
    )
    .await?;
    assert_eq!(reply["refusal"]["code"], "invalid_parameter");
    assert_eq!(
        reply["refusal"]["expected"],
        json!({"kind":"full_screen_filters", "application":true, "title":false})
    );
    Ok(())
}

#[tokio::test]
async fn unknown_capture_filter_is_refused_before_window_selection() -> Result<()> {
    for capability in ["screenshot", "selected_text"] {
        let mut h = Mac::new(false).await?;
        let reply = remote_reply(&mut h, capability, json!({"window":"Safari"})).await?;
        assert_eq!(reply["refusal"]["code"], "invalid_parameter");
        let expected = &reply["refusal"]["expected"];
        assert_eq!(expected["kind"], "selection_filters");
        assert_eq!(expected["unknown"], "window");
        let allowed = if capability == "screenshot" {
            json!(["application", "title", "full_screen"])
        } else {
            json!(["application", "title"])
        };
        assert_eq!(expected["allowed"], allowed);
        assert!(h.runtime.conversation.bridge_request.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn closed_native_source_channel_names_the_helper() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h._native.close();
    let reply = remote_reply(&mut h, "selected_text", json!({})).await?;
    assert_eq!(reply["refusal"]["code"], "helper_not_running");
    assert!(reply["refusal"]["retry"]["kind"].is_string());
    Ok(())
}

#[tokio::test]
async fn pending_source_names_busy_instead_of_engine_failure() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h.runtime.source_windows().await?;
    let reply = remote_reply(&mut h, "selected_text", json!({})).await?;
    assert_eq!(reply["refusal"]["code"], "busy");
    Ok(())
}

#[tokio::test]
async fn busy_engine_does_not_claim_to_have_failed_starting() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let generation = h.runtime.state.generation();
    h.runtime.features.computer.work.attach_test(
        generation,
        (),
        tokio::spawn(async {
            std::future::pending::<()>().await;
        }),
    );
    let reply = remote_reply(
        &mut h,
        "perform_action",
        json!({"action":{"tool":"launch_app","arguments":{"bundle_id":"com.example.app"}}}),
    )
    .await?;
    assert_eq!(reply["refusal"]["code"], "busy");
    if let Some(task) = h.runtime.features.computer.work.take() {
        task.abort();
    }
    Ok(())
}

#[tokio::test]
async fn undeclared_tool_names_that_tool_as_the_invalid_choice() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let reply = remote_reply(
        &mut h,
        "perform_action",
        json!({"action":{"tool":"undeclared_tool","arguments":{}}}),
    )
    .await?;
    assert_eq!(reply["refusal"]["code"], "invalid_parameter");
    assert_eq!(
        reply["refusal"]["expected"],
        json!({"kind":"declared_tool", "received":"undeclared_tool"})
    );
    Ok(())
}

#[tokio::test]
async fn history_storage_failure_is_named_before_input() -> Result<()> {
    let mut h = Mac::new(false).await?;
    std::fs::create_dir(h.runtime.paths.root.join("computer-control.json"))?;
    let reply = remote_reply(
        &mut h,
        "perform_action",
        json!({"action":{"tool":"launch_app","arguments":{"bundle_id":"com.example.app"}}}),
    )
    .await?;
    assert_eq!(reply["refusal"]["code"], "storage_failed");
    assert_eq!(reply["refusal"]["operation"], "computer_action");
    assert_eq!(h.calls("launch_app"), 0);
    Ok(())
}

#[tokio::test]
async fn failed_engine_keeps_its_retry_number_without_its_system_error() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h.runtime.features.computer.service = None;
    h.runtime.features.computer.engine_failure = Some("/Users/sample/private/engine".into());
    h.runtime.features.computer.engine_retry_at = Some(Instant::now() + Duration::from_secs(3600));
    let before = match h.runtime.engine_status() {
        engine::Status::Failed { retry_in_ms, .. } => retry_in_ms,
        status => anyhow::bail!("expected a failed engine before request: {status:?}"),
    };
    let reply = remote_reply(&mut h, "control_tools", json!({})).await?;
    let after = match h.runtime.engine_status() {
        engine::Status::Failed { retry_in_ms, .. } => retry_in_ms,
        status => anyhow::bail!("expected a failed engine after request: {status:?}"),
    };
    assert_eq!(reply["refusal"]["code"], "engine_unavailable");
    assert_eq!(reply["refusal"]["cause"], "initialization_failed");
    let reported = reply["refusal"]["retry_in_ms"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing retry_in_ms: {reply}"))?;
    assert!(
        after <= reported && reported <= before,
        "{after} <= {reported} <= {before}"
    );
    assert!(!reply.to_string().contains("/Users/"));
    Ok(())
}

#[tokio::test]
async fn real_engine_failure_handler_never_replies_with_system_text() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let (sender, mut hub) = mpsc::channel(16);
    h.runtime.hub.install(sender);
    h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
        id: "engine-error".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    h.runtime
        .computer_failed(&anyhow::anyhow!(
            "engine failed at /Users/sample/private/key and $HOME"
        ))
        .await;
    let frame = hub.try_recv()?;
    assert_eq!(frame["params"]["refusal"]["code"], "engine_unavailable");
    assert!(!frame.to_string().contains("/Users/"));
    assert!(!frame.to_string().contains("$HOME"));
    let said = serde_json::to_string(&h.runtime.state.durable.notice_log)?;
    assert!(!said.contains("/Users/"));
    assert!(!said.contains("$HOME"));
    Ok(())
}

#[tokio::test]
async fn failed_active_action_replies_with_uncertain_outcome() -> Result<()> {
    {
        let mut h = Mac::new(false).await?;
        let (sender, mut hub) = mpsc::channel(16);
        h.runtime.hub.install(sender);
        let operation_id = "22222222-2222-4222-8222-222222222222";
        let entry =
            ControlHistoryEntry::new(operation_id, "click", "Fixture", 1, ControlOutcome::Unknown)
                .map_err(anyhow::Error::msg)?;
        h.runtime.upsert_control_history(entry.clone())?;
        h.runtime.features.computer.active_general = Some(entry);
        h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
            id: "11111111-1111-4111-8111-111111111111".into(),
            capability: orbit_core::device_bridge::Capability::PerformAction,
            application: None,
            title: None,
            full_screen: false,
            source_id: None,
            tool_call: None,
            catalogue_after: 0,
            calendar_window: None,
            calendar_cursor: None,
            files: None,
        });
        h.runtime
            .computer_failed(&anyhow::anyhow!("engine lost its receipt"))
            .await;
        let mut frame = hub.try_recv()?;
        // Every request frame names its op at the top level; the
        // rest of the reply is the contract fixture as it was.
        let op = frame
            .as_object_mut()
            .and_then(|object| object.remove("op"))
            .context("the reply names its op")?;
        assert!(
            op.as_str()
                .and_then(orbit_core::notice::Op::parse)
                .is_some(),
            "{op}"
        );
        assert_eq!(frame["method"], "device.reply");
        assert_eq!(
            frame["params"]["request_id"],
            "11111111-1111-4111-8111-111111111111"
        );
        let expected: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/fixtures/active-action-outcome-unknown-reply.json"
        )))?;
        assert_eq!(frame, expected);
        assert_eq!(frame["params"]["refusal"]["code"], "outcome_unknown");
        assert_eq!(frame["params"]["refusal"]["operation_id"], operation_id);
        assert_eq!(frame["params"]["refusal"]["retry"]["kind"], "check_outcome");
        assert_eq!(
            h.runtime.control_history()?[0].outcome,
            ControlOutcome::Unknown
        );
        assert!(h.runtime.conversation.bridge_request.is_none());
        assert!(hub.try_recv().is_err(), "the action got another reply");
    }
    Ok(())
}

/// The real test's first whole-window read came back `outcome_unknown`
/// with nothing in daemon.log or the report log about why. An engine failure
/// leaves its cause in daemon.log and a typed `engine_failed` line, with the
/// receipt's numbers when that was the cause, under the turn's op.
#[tokio::test]
async fn an_engine_failure_leaves_its_cause_and_a_report_line() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let root = h._directory.path().to_path_buf();
    h.runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(&root));
    h.runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(&root));
    let (sender, mut frames) = mpsc::channel(4);
    h.runtime.hub.install(sender);
    let call = ToolCall {
        tool: "get_window_state".into(),
        arguments: json!({"pid": 11, "window_id": 22, "query": ENORMOUS_TREE}),
    };
    h.runtime.conversation.bridge_request = Some(super::super::BridgeAsk {
        id: "whole-window-read".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: Some(call.clone()),
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    h.runtime.begin_turn(
        orbit_core::notice::Op::parse("op-engine-0001").context("op")?,
        orbit_core::notice::ReportSource::Daemon,
    );
    h.runtime
        .call_engine_tool(call, "fixture")
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    h.finish().await?;

    // Deimos is answered as before: the attempt's outcome is not known.
    let frame = frames.try_recv().context("Deimos did not get a response")?;
    assert_eq!(frame["params"]["refusal"]["code"], "outcome_unknown");

    let daemon_log = std::fs::read_to_string(root.join("logs/daemon.log"))?;
    let line = daemon_log
        .lines()
        .find(|line| line.contains("engine_failed"))
        .context("daemon.log says nothing about the engine failure")?;
    let max = orbit_core::limits::MAX_ACTION_RECEIPT_BYTES.to_string();
    assert!(line.contains("Recibo de acción de"), "{line}");
    assert!(line.contains(&max), "{line}");
    assert!(line.contains("op-engine-0001"), "{line}");
    assert!(line.contains("whole-window-read"), "{line}");

    let report = std::fs::read_to_string(root.join("logs/report-daemon.log"))?;
    let row: Value = report
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|row| row["code"] == "engine_failed")
        .context("no engine_failed line in the report log")?;
    assert_eq!(row["op"], "op-engine-0001");
    assert_eq!(row["class"], "silent");
    assert_eq!(row["data"]["cause"], "unhealthy");
    assert_eq!(row["data"]["code"], "receipt_too_large");
    assert_eq!(
        row["data"]["max_bytes"],
        orbit_core::limits::MAX_ACTION_RECEIPT_BYTES
    );
    assert!(
        row["data"]["bytes"].as_u64().context("bytes")?
            > orbit_core::limits::MAX_ACTION_RECEIPT_BYTES as u64
    );
    assert!(orbit_core::notice::NoticeCode::EngineFailed.is_failure());
    Ok(())
}

/// An engine that never comes up says so too, with the cause in daemon.log.
#[tokio::test]
async fn an_engine_that_cannot_start_leaves_its_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
    runtime.engine_unavailable(
        orbit_core::refusal::EngineCause::InitializationFailed,
        &anyhow::anyhow!("could not launch cua-driver: ENOENT"),
        &[],
    );
    let daemon_log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(
        daemon_log
            .lines()
            .any(|line| line.contains("engine_failed") && line.contains("ENOENT")),
        "{daemon_log}"
    );
    let report = std::fs::read_to_string(directory.path().join("logs/report-daemon.log"))?;
    assert!(
        report.contains(r#""code":"engine_failed""#)
            && report.contains(r#""cause":"initialization_failed""#)
            && report.contains(r#""code":"engine_error""#),
        "{report}"
    );
    Ok(())
}

fn permission_reason(runtime: &Runtime) -> String {
    match runtime.engine_status() {
        engine::Status::PermissionMissing { permissions } => {
            orbit_core::refusal::Refusal::PermissionDenied {
                permission: orbit_core::refusal::missing_permission_name(permissions),
            }
            .to_string()
        }
        engine::Status::PermissionUnknown { permissions } => {
            orbit_core::refusal::Refusal::PermissionUnknown {
                permission: orbit_core::refusal::missing_permission_name(permissions),
            }
            .to_string()
        }
        other => panic!("expected a permission refusal, got {other:?}"),
    }
}

#[path = "engine_exit_tests.rs"]
mod engine_exit_tests;
