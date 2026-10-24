//! Actor regressions: no production network, microphone, or provider.
#![allow(clippy::expect_used)]
use super::{fixture, start};
use crate::daemon::turn::said;
use crate::daemon::{HubEvent, Runtime, WorkResult};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::{CapturePhase, Connection, Event, State};
use orbit_store::{ChunkMetadata, Job, JobState};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn status_with_calendar(value: serde_json::Value) -> serde_json::Value {
    json!({"event":"response","id":"status","ok":true,
        "result":{"permissions":{"microphone":"authorized","screen":true,
            "accessibility":false,"calendars":value}}})
}

#[tokio::test]
async fn the_calendar_authorization_reported_by_the_helper_is_kept() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    assert_eq!(runtime.state.capabilities().calendar(), None);
    // The answer is paired with a status the daemon sent. An id it did not
    // send is unpaired and is not read as permissions.
    runtime
        .helper
        .record_pending(&json!({"id": "status", "command": "status"}));
    runtime
        .native_event(status_with_calendar(json!("full_access")))
        .await?;
    // The helper sends this with every status. Dropping it left the calendar
    // permanently reading as never observed, which a person reads as absent.
    assert_eq!(
        runtime.state.capabilities().calendar(),
        Some(orbit_core::calendar::CalendarAuthorization::FullAccess)
    );
    // An unreadable value must not erase a real observation.
    runtime
        .helper
        .record_pending(&json!({"id": "status", "command": "status"}));
    let problem = runtime
        .native_event(status_with_calendar(json!(12345)))
        .await
        .expect_err("a calendar number is not an authorization");
    assert!(problem.to_string().contains("field=calendars type=number"));
    assert_eq!(
        runtime.state.capabilities().calendar(),
        Some(orbit_core::calendar::CalendarAuthorization::FullAccess)
    );
    Ok(())
}

/// A runtime whose outgoing hub frames can be read back.
///
/// The helper's receiver is returned rather than dropped. An earlier version
/// discarded it with `_`, which closes the channel: every send to the helper
/// then failed, the bridge answered "no pude" to everything, and the tests still
/// passed because none of them looked at what reached the helper.
async fn bridge_fixture(
    directory: &std::path::Path,
) -> Result<(
    crate::daemon::Runtime,
    mpsc::Receiver<serde_json::Value>,
    mpsc::Receiver<serde_json::Value>,
)> {
    let (mut runtime, native) = fixture(directory)?;
    let (hub, hub_rx) = mpsc::channel(16);
    runtime.link.outbound.install(hub);
    Ok((runtime, hub_rx, native))
}

/// What macOS grants, set where the daemon reads it: the permissions the
/// helper reports. Orbit once kept two switches of its own here; they are
/// gone, because the macOS permission is the user's decision. Screen
/// Recording is what lets the bridge look, and Accessibility with it is what
/// lets it act. A value set here is what the next request is decided on.
fn grant(
    runtime: &mut crate::daemon::Runtime,
    screen_recording: bool,
    accessibility: bool,
) -> Result<()> {
    let state = |granted| {
        if granted {
            orbit_core::PermissionState::Granted
        } else {
            orbit_core::PermissionState::Denied
        }
    };
    {
        let fixture_value = serde_json::json!(state(screen_recording));
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(state(accessibility));
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    Ok(())
}

async fn ask(runtime: &mut crate::daemon::Runtime, capability: &str) -> Result<()> {
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request",
            "data": {"request_id": "bridge-1", "capability": capability}
        })))
        .await
}

#[tokio::test]
async fn malformed_screenshot_filter_cannot_select_an_unfiltered_window() -> Result<()> {
    for parameters in [
        json!({"application": 5}),
        json!({"title": {}}),
        json!({"application": null}),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
        grant(&mut runtime, true, false)?;
        runtime
            .hub_event(HubEvent::Frame(json!({
                "v":1, "event":"device.request", "data": {
                    "request_id":"capture-1", "capability":"screenshot", "parameters":parameters
                }
            })))
            .await?;
        let arbitrary_window = json!({"structured": {"windows": [{
            "pid": 1234, "window_id": 99, "app_name": "Otra aplicación",
            "title": "Otra ventana", "z_index": 1
        }]}});
        assert!(
            runtime
                .assistant
                .bridge_screenshot_target(&arbitrary_window)
                .is_none(),
            "the invalid filter chose an unrelated window"
        );
        let reply = hub
            .try_recv()
            .context("no refusal for the invalid filter")?;
        assert_eq!(reply["params"]["refusal"]["code"], "invalid_parameter");
        let expected = &reply["params"]["refusal"]["expected"];
        assert_eq!(expected["kind"], "wire_field");
        assert_eq!(expected["expected"], "string");
        assert_eq!(
            expected["field"],
            if parameters.get("application").is_some() {
                "application"
            } else {
                "title"
            }
        );
        assert!(
            helper.try_recv().is_err(),
            "a capture was asked for without a filter"
        );
        assert!(runtime.assistant.bridge_requests.is_empty());
    }
    Ok(())
}

fn diagnostic_fields(directory: &std::path::Path, event: &str) -> Result<Vec<serde_json::Value>> {
    let log = std::fs::read_to_string(directory.join("logs/daemon.log"))?;
    let records = log
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<serde_json::Value>, _>>()?;
    Ok(records
        .into_iter()
        .filter(|record| record["event"] == event)
        .map(|record| record["fields"].clone())
        .collect())
}

#[tokio::test]
async fn unknown_hub_events_preserve_the_live_state() -> Result<()> {
    for protocol in [Some(7), None] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
        runtime.settings.device_id = Some("synthetic-device".into());
        runtime
            .hub_event(HubEvent::Frame(json!({"v":1,"event":"ready",
            "data":{"device_id":"synthetic-device","hub_protocol":protocol}})))
            .await?;
        runtime.state.durable.assistant.active_request = Some("live-turn".into());
        let before = runtime.state.clone();
        for _ in 0..2 {
            runtime
                .hub_event(HubEvent::Frame(
                    json!({"v":1,"event":"future.event","data":{}}),
                ))
                .await?;
            assert_eq!(runtime.state, before);
        }
        assert_eq!(runtime.state.connection(), Connection::Online);
        assert_eq!(
            diagnostic_fields(directory.path(), "hub_event_unrecognized")?,
            vec![json!({"name":"future.event", "daemon_protocol":"1",
                "hub_protocol":protocol.map_or("sin anunciar".into(), |value| value.to_string())})]
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_malformed_delta_does_not_replace_the_active_turn() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime.state.durable.assistant.active_request = Some("turn-1".into());
    {
        let fixture_source: Option<String> = Some("session-1".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime
        .hub_event(HubEvent::Frame(json!({
            "v": 1, "id": "turn-1", "event": "chat.delta",
            "data": {"session_id": "session-1", "text": 42}
        })))
        .await?;
    assert_eq!(
        runtime.state.durable.assistant.active_request.as_deref(),
        Some("turn-1")
    );
    assert!(runtime.state.durable.assistant.turn.is_none());
    assert_eq!(
        diagnostic_fields(directory.path(), "hub_event_invalid")?,
        vec![
            json!({"event":"chat.delta", "field":"text", "actual_type":"integer", "expected_type":"string"})
        ]
    );
    Ok(())
}

#[tokio::test]
async fn malformed_hub_field_logs_types_without_echoing_its_value() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"state.snapshot", "data":{"revision":"PRIVATE TEXT", "sessions":[]}
        })))
        .await?;
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(log.contains("state.snapshot"), "{log}");
    assert!(log.contains("revision"), "{log}");
    assert!(log.contains("string"), "{log}");
    assert!(log.contains("integer"), "{log}");
    assert!(!log.contains("PRIVATE TEXT"), "{log}");
    Ok(())
}

#[tokio::test]
async fn malformed_legacy_snapshot_questions_do_not_disconnect() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.settings.device_id = Some("device-1".into());
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event":"ready","data":{"device_id":"device-1"}}),
        ))
        .await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"state.snapshot", "data":{"revision":1,"sessions":[{
                "session_id":"s1","status":"streaming","pending":[{
                    "request_id":"r1","kind":"clarify","state":"pending",
                    "questions":[{"qid":42,"question":"unexpected"}]
                }]
            }]}
        })))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Online);
    assert_eq!(runtime.assistant.snapshot_revision, Some(1));
    Ok(())
}

#[tokio::test]
async fn unknown_event_names_have_a_memory_ceiling() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    for index in 0..130 {
        runtime
            .hub_event(HubEvent::Frame(json!({
                "event": format!("future.{index}"), "data": {}
            })))
            .await?;
    }
    assert_eq!(runtime.link.unrecognized_events.len(), 128);
    Ok(())
}

#[tokio::test]
async fn chat_status_only_updates_the_selected_session_with_text() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("selected".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.state.set_notice(NoticeCode::AudioGap);
    for (session_id, text) in [
        ("other", "no mostrar"),
        ("selected", ""),
        ("selected", "  "),
    ] {
        runtime
            .hub_event(HubEvent::Frame(json!({
                "event":"chat.status", "data":{"session_id":session_id,"kind":"status","text":text}
            })))
            .await?;
        assert_eq!(runtime.state.notice_code(), Some(NoticeCode::AudioGap));
    }
    runtime.hub_event(HubEvent::Frame(json!({
        "event":"chat.status", "data":{"session_id":"selected","kind":"status","text":"procesando"}
    }))).await?;
    // The hub's progress line is about the conversation: recorded, never
    // its text.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantProgress)
    );
    Ok(())
}

#[tokio::test]
async fn missing_approval_description_is_not_invented() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime.state.durable.assistant.active_request = Some("turn-1".into());
    {
        let fixture_source: Option<String> = Some("session-1".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime
        .hub_event(HubEvent::Frame(json!({
            "v":1,"id":"turn-1","event":"approval.required",
            "data":{"session_id":"session-1","request_id":"approval-1"}
        })))
        .await?;
    assert!(runtime.state.durable.assistant.pending_approval.is_none());
    assert_eq!(
        runtime.state.durable.assistant.active_request.as_deref(),
        Some("turn-1")
    );
    assert_eq!(
        diagnostic_fields(directory.path(), "hub_event_invalid")?,
        vec![
            json!({"event":"approval.required", "field":"description/question", "actual_type":"missing", "expected_type":"string"})
        ]
    );
    Ok(())
}

#[tokio::test]
async fn remote_look_and_action_name_the_missing_permission_before_touching_the_helper()
-> Result<()> {
    // Nothing in Orbit is switched on or off. What refuses a look is
    // macOS not granting Screen Recording, and the refusal names it.
    for (capability, screen, accessibility, permission) in [
        ("screenshot", false, true, "screen_recording"),
        ("perform_action", true, false, "accessibility"),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
        grant(&mut runtime, screen, accessibility)?;
        ask(&mut runtime, capability).await?;
        // Nothing reached the Mac's helper: the refusal happened before any of it.
        assert!(helper.try_recv().is_err());
        let reply = hub.recv().await.context("the request got no answer")?;
        assert_eq!(reply["method"], "device.reply");
        assert_eq!(reply["params"]["request_id"], "bridge-1");
        assert_eq!(
            reply["params"]["refusal"]["code"], "permission_denied",
            "{reply}"
        );
        assert_eq!(
            reply["params"]["refusal"]["permission"], permission,
            "{reply}"
        );
        // Said and recorded with its code; Deimos tells the person the cause in
        // its reply (the refusal is silent on the Mac's screen).
        assert!(said::has(&runtime.state, NoticeCode::BridgeRefused));
        assert!(said::mentions(&runtime.state, "permission_denied"));
        assert!(runtime.assistant.bridge_requests.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn a_latched_privacy_stop_says_nothing_at_all_to_the_hub() -> Result<()> {
    // Written first expecting a polite refusal, which hung: `hub_event` returns
    // before dispatching anything while privacy is latched, so a stopped Mac is
    // silent to the hub rather than talkative about being stopped. That
    // invariant predates this bridge and is the stronger promise, so the bridge
    // gives way to it. The caller's timeout is what names the answer, and that
    // belongs to the hub.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, true)?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    for capability in ["screenshot", "perform_action", "selected_text", "inventado"] {
        ask(&mut runtime, capability).await?;
    }
    assert!(
        hub.try_recv().is_err(),
        "a Mac with privacy stopped talked to the hub"
    );
    assert!(
        helper.try_recv().is_err(),
        "a Mac with privacy stopped looked at something"
    );
    // And nothing was accepted for service either.
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn an_admitted_request_tells_the_person_what_was_asked_for() -> Result<()> {
    // The point of the bridge is that the user can see it being used while it is
    // being used, not afterwards in a log.
    let directory = tempfile::tempdir()?;
    let (mut runtime, _hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    ask(&mut runtime, "screen_sources").await?;
    // The announcement survives the local capture path, which sets this same
    // field to describe its own progress. Announcing before starting the work
    // meant it was overwritten immediately and the person saw nothing.
    assert!(
        said::mentions(&runtime.state, "screen_sources"),
        "the bridge notice was lost: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        runtime
            .assistant
            .bridge_requests
            .get(crate::daemon::BridgeKind::Screen)
            .map(|ask| ask.id.as_str()),
        Some("bridge-1")
    );
    // And the work actually started: the helper was asked to list windows.
    let sent = helper
        .try_recv()
        .context("nothing was asked of the helper")?;
    assert_eq!(sent["command"], "screen_sources");
    Ok(())
}

#[tokio::test]
async fn a_request_without_an_identity_is_dropped_rather_than_answered() -> Result<()> {
    // There is nowhere to send the answer, and inventing an id would let a
    // malformed frame collect a reply meant for a real request.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, _helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request", "data": {"capability": "screenshot"}
        })))
        .await?;
    assert!(hub.try_recv().is_err());
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

// A window the Mac considers capturable must carry a full binding: the identity
// fields are UUIDs and the clock domain is the one macOS actually reports, so a
// convenient-looking fixture with round numbers is rejected as malformed.
const INSTANCE: &str = "11111111-1111-4111-8111-111111111111";
const INVENTORY: &str = "22222222-2222-4222-8222-222222222222";
const TOKEN: &str = "33333333-3333-4333-8333-333333333333";
const APPLICATION: &str = "44444444-4444-4444-8444-444444444444";
const DESCRIPTOR: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

#[tokio::test]
async fn the_helpers_window_list_comes_back_as_the_answer_to_the_request() -> Result<()> {
    // End to end on this side of the wire: request in, helper asked, helper
    // answers, answer out. Without this the bridge could accept and announce
    // for ever while never actually replying to anything.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    ask(&mut runtime, "screen_sources").await?;
    let sent = helper
        .try_recv()
        .context("nothing was asked of the helper")?;
    assert_eq!(sent["command"], "screen_sources");
    let command_id = sent["id"].as_str().context("no command id")?.to_owned();

    runtime
        .native_event(json!({
            "event": "response",
            "id": command_id,
            "ok": true,
            "result": {
                "native_instance_id": INSTANCE,
                "privacy_generation": runtime.state.generation(),
                "inventory_id": INVENTORY,
                "clock_domain": "mach_absolute_time",
                // Observed must be non-zero and the window no longer than the
                // inventory's own TTL: a zero start reads as "never observed".
                "observed_monotonic_ns": 1_000_000u64,
                "expires_monotonic_ns": 1_000_000u64 + orbit_core::sources::INVENTORY_TTL_NS,
                "selected_text_available": false,
                "windows": [{
                    "binding": {
                        "native_instance_id": INSTANCE,
                        "privacy_generation": runtime.state.generation(),
                        "inventory_id": INVENTORY,
                        "selection_token": TOKEN,
                        "window_id": 11,
                        "application_pid": 4242,
                        "application_instance_id": APPLICATION,
                        "binding_quality": "observed_owner_and_window",
                        "descriptor_hash": DESCRIPTOR
                    },
                    "window_id": 11,
                    "application_pid": 4242,
                    "application_name": "Safari",
                    "bundle_id": "com.apple.Safari",
                    "title": "Correo",
                    "title_truncated": false,
                    "unavailable_reason": null
                }],
                "omitted_count": 0u64
            }
        }))
        .await?;

    let reply = hub.recv().await.context("the request got no answer")?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    let windows = reply["params"]["result"]["windows"]
        .as_array()
        .with_context(|| format!("the reply carries no windows: {reply}"))?;
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0]["application"], "Safari");
    assert_eq!(windows[0]["title"], "Correo");
    // Internal handles stay on the Mac: a caller cannot compose a request out of
    // identifiers it was never given a way to obtain.
    assert!(windows[0].get("window_id").is_none());
    assert!(windows[0].get("application_pid").is_none());
    // The slot is free again, so the next request is not told "ocupado".
    assert!(runtime.assistant.bridge_requests.is_empty());
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request",
            "data": {"request_id": "bridge-2", "capability": "screen_sources"}
        })))
        .await?;
    let again = helper
        .recv()
        .await
        .context("the second request was not served")?;
    assert_eq!(again["command"], "screen_sources");
    let refused = hub.try_recv();
    assert!(
        refused.is_err(),
        "the second request was refused: {refused:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_privacy_stop_releases_the_in_flight_request_and_answers_the_hub() -> Result<()> {
    // The stop is one of the endings of the operation that served
    // the request. Holding the slot meant every later remote request was told
    // "ocupado" until the daemon restarted, so the stop releases it — and the
    // assistant whose request was already admitted gets the refusal rather
    // than silence.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    ask(&mut runtime, "screen_sources").await?;
    assert_eq!(
        runtime
            .assistant
            .bridge_requests
            .get(crate::daemon::BridgeKind::Screen)
            .map(|ask| ask.id.as_str()),
        Some("bridge-1")
    );
    assert!(
        helper.try_recv().is_ok(),
        "the request did not reach the helper"
    );

    runtime.apply(Event::PrivacyStop).await?;

    assert!(runtime.state.privacy_stopped());
    assert!(
        runtime.assistant.bridge_requests.is_empty(),
        "the stop left the bridge busy: {:?}",
        runtime.assistant.bridge_requests
    );
    let reply = hub
        .recv()
        .await
        .context("the request in flight got no answer")?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    assert_eq!(
        reply["params"]["refusal"]["code"], "privacy_stopped",
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn a_hub_disconnect_releases_the_in_flight_request_without_answering() -> Result<()> {
    // There is nobody left to answer once the hub is gone; holding
    // the slot for a request that died with the connection only blocks the
    // next request after a reconnect.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    ask(&mut runtime, "screen_sources").await?;
    assert!(
        helper.try_recv().is_ok(),
        "the request did not reach the helper"
    );

    runtime.hub_event(HubEvent::Disconnected).await?;

    assert!(
        runtime.assistant.bridge_requests.is_empty(),
        "the disconnect left the bridge busy: {:?}",
        runtime.assistant.bridge_requests
    );
    let sent_anyway = hub.try_recv();
    assert!(
        sent_anyway.is_err(),
        "a disconnected hub was answered: {sent_anyway:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_helper_restart_releases_the_in_flight_request_with_its_cause() -> Result<()> {
    // "ready" restarts the helper's world and the enumeration the
    // request was waiting on is cancelled without replay, so the request it
    // served cannot stay open waiting for an answer that will never come.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    ask(&mut runtime, "screen_sources").await?;
    let sent = helper
        .try_recv()
        .context("nothing was asked of the helper")?;
    assert_eq!(sent["command"], "screen_sources");

    runtime.native_event(json!({"event":"ready"})).await?;

    assert!(
        runtime.assistant.bridge_requests.is_empty(),
        "the cancellation left the bridge busy: {:?}",
        runtime.assistant.bridge_requests
    );
    let reply = hub
        .recv()
        .await
        .context("the request in flight got no answer")?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    assert_eq!(
        reply["params"]["refusal"]["code"], "read_interrupted",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["cause"]["kind"], "helper_restarted",
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn websocket_connection_is_not_online_until_exact_device_ready() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.settings.device_id = Some("expected-device".into());
    runtime.hub_event(HubEvent::Connected).await?;
    assert_eq!(runtime.state.connection(), Connection::Connecting);
    runtime
        .hub_event(HubEvent::Frame(json!({"event":"pong", "data":{}})))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Connecting);
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event":"ready", "data":{"device_id":"expected-device"}}),
        ))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Online);
    assert!(!runtime.state.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn another_device_ready_latches_privacy_and_cancels_live_work() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("expected-device".into());
    start(&mut runtime, &mut native).await?;
    let worker = tokio::spawn(std::future::pending::<()>());
    let worker_abort = worker.abort_handle();
    let generation = runtime.state.generation();
    runtime.transfers.worker.attach_test(generation, (), worker);
    let voice = tokio::spawn(std::future::pending::<()>());
    let voice_abort = voice.abort_handle();
    let generation = runtime.state.generation();
    runtime
        .assistant
        .voice_task
        .attach_test(generation, (), voice);
    let generation = runtime.state.generation();
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event":"ready", "data":{"device_id":"other-device"}}),
        ))
        .await?;
    tokio::task::yield_now().await;
    assert_eq!(runtime.state.connection(), Connection::Revoked);
    assert!(runtime.state.privacy_stopped());
    assert!(runtime.state.generation() > generation);
    assert!(worker_abort.is_finished() && voice_abort.is_finished());
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    // A revocation stops capture in the helper without writing its
    // durable latch; the hub is asked again after a restart.
    assert_eq!(
        native.recv().await.context("capture stop absent")?["command"],
        "capture_stop"
    );
    let persisted = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("snapshot absent")?;
    assert!(persisted.privacy_stopped());
    assert_eq!(
        persisted.privacy_stop_by(),
        Some(orbit_core::privacy_stop::StopRequester::Revocation)
    );
    assert_eq!(persisted.connection(), Connection::Revoked);
    Ok(())
}

#[tokio::test]
async fn missing_device_ready_is_not_an_identity_even_without_local_pairing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    assert!(runtime.settings.device_id.is_none());
    runtime.hub_event(HubEvent::Connected).await?;
    runtime
        .hub_event(HubEvent::Frame(json!({"event":"ready", "data":{}})))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Revoked);
    assert!(runtime.state.privacy_stopped());
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::HubDeviceIdentityMissing),
        "{:?}",
        runtime.state.durable.notice
    );
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(log.contains("ready"), "{log}");
    assert!(log.contains("device_id"), "{log}");
    assert!(log.contains("missing"), "{log}");
    assert!(log.contains("string"), "{log}");
    Ok(())
}

#[tokio::test]
async fn unpaired_local_device_does_not_call_a_received_identity_another_device() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    assert!(runtime.settings.device_id.is_none());
    runtime.hub_event(HubEvent::Connected).await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"ready", "data":{"device_id":"synthetic-device"}
        })))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Revoked);
    assert!(runtime.state.privacy_stopped());
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::DevicePairingMismatch),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(said::mentions(&runtime.state, "no_local_identity"));
    Ok(())
}

#[tokio::test]
async fn turn_frames_without_session_id_do_not_change_selected_session() -> Result<()> {
    for event in ["chat.delta", "chat.completed"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _native) = fixture(directory.path())?;
        let (hub, mut commands) = mpsc::channel(8);
        runtime.link.outbound.install(hub);
        runtime
            .apply(Event::Submit {
                attachments: Vec::new(),
                id: "active-request".into(),
                text: "SYNTHETIC QUESTION".into(),
            })
            .await?;
        let _command = commands.recv().await.context("outbound request absent")?;
        runtime
            .apply(Event::Session {
                id: "selected".into(),
                agent_id: None,
            })
            .await?;
        let before = runtime.state.durable.assistant.turn.clone();
        runtime
            .hub_event(HubEvent::Frame(json!({
                "event": event,
                "id": "active-request",
                "data": {"text": "INJECTED", "status": "completed"}
            })))
            .await?;
        assert_eq!(runtime.state.durable.assistant.turn, before, "{event}");
        assert_eq!(
            runtime.state.durable.assistant.active_request.as_deref(),
            Some("active-request"),
            "{event}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn unrelated_error_ids_cannot_finish_the_active_request() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (hub, mut commands) = mpsc::channel(8);
    runtime.link.outbound.install(hub);
    runtime
        .apply(Event::Submit {
            attachments: Vec::new(),
            id: "active-request".into(),
            text: "SYNTHETIC QUESTION".into(),
        })
        .await?;
    let _ = commands.recv().await.context("outbound request absent")?;
    runtime
        .apply(Event::Session {
            id: "session".into(),
            agent_id: None,
        })
        .await?;
    runtime
        .apply(Event::Delta {
            request_id: "active-request".into(),
            text: "SYNTHETIC PARTIAL".into(),
        })
        .await?;
    runtime.assistant.speak_next = true;
    for stale_id in [
        json!("another-request"),
        json!("completed-request"),
        serde_json::Value::Null,
    ] {
        runtime
            .hub_event(HubEvent::Frame(json!({"event":"error", "id":stale_id,
            "data":{"message":"SYNTHETIC UNRELATED ERROR"}})))
            .await?;
        assert_eq!(
            runtime.state.durable.assistant.active_request.as_deref(),
            Some("active-request")
        );
        assert!(runtime.assistant.speak_next);
        assert!(
            !runtime
                .state
                .durable
                .assistant
                .turn
                .as_ref()
                .and_then(|turn| turn.answer.as_ref())
                .context("partial absent")?
                .complete
        );
    }
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event":"error", "id":"active-request",
        "data":{"message":"SYNTHETIC ACTIVE ERROR"}}),
        ))
        .await?;
    assert!(runtime.state.durable.assistant.active_request.is_none());
    assert!(!runtime.assistant.speak_next);
    // The hub's message is free text: logged, never said.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AssistantFailed)
    );
    assert!(
        !runtime
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .and_then(|turn| turn.answer.as_ref())
            .context("partial absent")?
            .complete
    );
    Ok(())
}

#[tokio::test]
async fn upload_refusal_blocks_the_job_says_it_and_arms_initial_retry_gate() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    runtime.fixture_store.enqueue_job(&Job {
        id: "upload-fixture".into(),
        kind: "chunk_upload".into(),
        state: JobState::Running,
        created_at: 1,
        payload: json!({}),
    })?;
    let generation = runtime.state.generation();
    let ticket = super::pending_work(&mut runtime.transfers.worker, generation, ())?;
    let before = Instant::now();
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket,
            job_id: "upload-fixture".into(),
            result: Err(crate::retry::Refused(403).into()),
        }))
        .await?;
    let after = Instant::now();
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-fixture")?
            .context("job absent")?
            .state,
        JobState::Blocked
    );
    assert!(
        !runtime
            .transfers
            .retry_gate
            .ready(before + Duration::from_millis(1999))
    );
    assert!(
        runtime
            .transfers
            .retry_gate
            .ready(after + Duration::from_secs(2))
    );
    assert!(runtime.transfers.worker.active().is_none());
    // A refusal for good is said with its status, never as a silent retry.
    assert_eq!(runtime.state.notice_code(), Some(NoticeCode::UploadRefused));
    assert!(
        said::told(&runtime).contains("403"),
        "{}",
        said::told(&runtime)
    );
    Ok(())
}

#[tokio::test]
async fn transient_upload_error_remains_retryable() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.fixture_store.enqueue_job(&Job {
        id: "upload-fixture".into(),
        kind: "chunk_upload".into(),
        state: JobState::Running,
        created_at: 1,
        payload: json!({}),
    })?;
    let generation = runtime.state.generation();
    let ticket = super::pending_work(&mut runtime.transfers.worker, generation, ())?;
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket,
            job_id: "upload-fixture".into(),
            result: Err(anyhow::anyhow!("SYNTHETIC TIMEOUT")),
        }))
        .await?;
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-fixture")?
            .context("job absent")?
            .state,
        JobState::RetryWait
    );
    assert!(!runtime.transfers.retry_gate.ready(Instant::now()));
    assert!(runtime.transfers.worker.active().is_none());
    Ok(())
}

/// The capture task's own clock is set longer than the test: the second
/// chunk can only go up because the first one's success scheduled it.
#[tokio::test]
async fn successful_upload_schedules_the_next_chunk_without_a_timer_tick() -> Result<()> {
    use crate::daemon::capture_lane::test_support;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("SYNTHETIC-TOKEN".into());
    let mut receipts = Vec::new();
    for (sequence, chunk_id) in ["a", "b"].into_iter().enumerate() {
        receipts.push(runtime.fixture_store.seal_chunk(
            &ChunkMetadata {
                capture_epoch_id: None,
                id: chunk_id.into(),
                recording_id: "recording".into(),
                track: "mic_local".into(),
                sequence: sequence as u64,
                sample_rate: 8000,
                channels: 1,
                frames: 4,
                monotonic_ns: sequence as u64 * 500_000,
                first_sample: sequence as u64 * 4,
                started_at_ms: 1,
            },
            &crate::native::wav(&[0; 8], 8000, 1)?,
        )?);
    }
    let (mut task, _bridge) =
        test_support::spawn_task_ticking(&runtime, Duration::from_secs(3600))?;
    let (mut first, _) = test_support::accept_put(&listener).await?;
    // A failure before held the gate; the success lifts it.
    runtime.transfers.retry_gate.finish(false, Instant::now());
    test_support::acknowledge(&mut first, "a", &receipts[0].sha256).await?;
    let (mut second, request) = test_support::accept_put(&listener).await?;
    assert!(String::from_utf8_lossy(&request).starts_with("PUT /v1/chunks/b "));
    test_support::job_reaches(&runtime, "upload-a", JobState::Succeeded).await?;
    assert!(runtime.transfers.retry_gate.ready(Instant::now()));
    test_support::acknowledge(&mut second, "b", &receipts[1].sha256).await?;
    test_support::job_reaches(&runtime, "upload-b", JobState::Succeeded).await?;
    task.stop();
    Ok(())
}

#[tokio::test]
async fn a_rearm_typed_on_this_mac_reaches_the_helper_instead_of_being_refused() -> Result<()> {
    // The menu bar item used to be the only accepted way back from a privacy
    // stop, so a person whose menu was stuck had no route at all. This command
    // arrives on the user's own 0600 socket, which is the local presence the
    // latch requires; remote traffic still cannot reach it.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.apply(Event::PrivacyStop).await?;
    assert!(runtime.state.privacy_stopped());
    while receiver.try_recv().is_ok() {}

    runtime
        .command(orbit_core::protocol::Command::Rearm)
        .await?;

    assert_eq!(receiver.try_recv()?["command"], "rearm");
    // The helper owns the durable latch, so the daemon does not rearm itself
    // ahead of it: a helper that refuses must leave both sides stopped.
    assert!(runtime.state.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn the_helpers_own_rearm_event_is_what_clears_the_stop() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    // Set the latch directly: going through PrivacyStop would leave audio
    // draining, and a rearm is refused until that audio is durable, which is a
    // separate rule with its own message.
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }

    runtime.native_event(json!({"event":"local_rearm"})).await?;

    assert!(!runtime.state.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn a_stop_from_the_mac_arrives_naming_its_cause() -> Result<()> {
    // Every stop used to print the same sentence, so an audio route change and
    // a deliberate brake were indistinguishable, and the daemon's own log could
    // not explain afterwards why capture died. An audio route change is not
    // the user's brake, so it cuts what was live and never latches.
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;

    runtime
        .native_event(
            json!({"event":"privacy_stop","reason":"audio_device_changed","privacy_stopped":false}),
        )
        .await?;

    assert!(!runtime.state.privacy_stopped());
    assert!(said::mentions(&runtime.state, "audio_device_changed"));
    Ok(())
}

#[tokio::test]
async fn a_stop_with_no_reason_still_says_so_instead_of_inventing_one() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;

    runtime
        .native_event(json!({"event":"privacy_stop","privacy_stopped":true}))
        .await?;

    assert!(said::mentions(&runtime.state, "unreported"));
    Ok(())
}

#[tokio::test]
async fn a_discontinuity_after_a_stop_does_not_ask_for_yet_another_stop() -> Result<()> {
    // Five stops in eight seconds on the developer's Mac: a gap bailed, the daemon's
    // error path ran fail_closed, fail_closed requested another native stop,
    // and the next tail frame gapped again. Dismantling the audio graph is
    // itself a source of discontinuities, so the teardown cannot be what
    // re-triggers the teardown.
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    // Draining: tail frames are still accepted until the stop is confirmed.
    runtime.audio.lifecycle.full_stop =
        Some(("privacy-1".into(), crate::daemon::audio::FullStop::Privacy));
    runtime.audio.mic_stream = Some("stream-1".into());

    let result = runtime
        .native_event(json!({"event":"gap","source":"mic_local","stream_id":"stream-1"}))
        .await;

    assert!(
        result.is_ok(),
        "a gap during teardown must not error, because an error requests another stop"
    );
    Ok(())
}

/// The run loop's own native branch, so a test that wants the production
/// consequences of a rejected response walks the same path the loop does.
async fn deliver(runtime: &mut crate::daemon::Runtime, event: serde_json::Value) {
    runtime.handle_native_event(event).await;
}

/// How many `privacy_stop` orders are still waiting for the helper.
fn count_privacy_stops(receiver: &mut mpsc::Receiver<serde_json::Value>) -> usize {
    let mut count = 0;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "privacy_stop" {
            count += 1;
        }
    }
    count
}

/// The readback a native call start has to carry before the call goes live,
/// in the shape `call_start_response` verifies.
fn call_readback(audio_start: &serde_json::Value) -> serde_json::Value {
    json!({"processing":"voice_processing","processing_enabled":true,
        "processing_bypassed":false,"agc_enabled":false,"playback_reference":"shared_engine",
        "pipeline_id":"synthetic-pipeline","playback_active":false,
        "playback_observation":"relay_drain","stream_id":audio_start["stream_id"]})
}

#[tokio::test]
async fn a_playback_rejection_during_a_live_call_is_a_notice_not_a_privacy_stop() -> Result<()> {
    // The helper legitimately refuses to queue more synthesized audio;
    // that must preserve the call and the person's privacy.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    runtime
        .apply(Event::CallStart {
            id: "synthetic-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert_eq!(
        receiver.recv().await.context("playback stop absent")?["command"],
        "playback_stop"
    );
    let audio_start = receiver.recv().await.context("audio start absent")?;
    runtime
        .native_event(json!({
            "event":"response","id":audio_start["id"],"ok":true,
            "result":call_readback(&audio_start)
        }))
        .await?;
    assert!(runtime.state.durable.voice.call.is_some());
    // A spoken answer in flight: its playback order is already with the helper.
    runtime
        .native_send(
            "playback_clip",
            json!({"playback_id":"synthetic-playback", "mime_type":"audio/wav",
                "provider":"synthetic", "audio_base64":"AA=="}),
        )
        .await?;
    let playback = receiver.recv().await.context("playback command absent")?;

    deliver(
        &mut runtime,
        json!({"event":"response","id":playback["id"],"ok":false,
            "error":{"code":"playback_backpressure","message":"cola de reproducción llena"}}),
    )
    .await;
    tokio::task::yield_now().await;

    assert!(
        !runtime.state.privacy_stopped(),
        "a rejected playback order must not brake privacy"
    );
    assert!(
        runtime.state.durable.voice.call.is_some(),
        "the live call must survive a playback rejection"
    );
    assert!(
        said::mentions(&runtime.state, "playback_backpressure"),
        "the notice must name the helper code: {:?}",
        runtime.state.durable.notice
    );
    assert!(
        said::has(&runtime.state, NoticeCode::CallActive),
        "the notice must be additive, not a replacement: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop"
    );
    Ok(())
}

#[tokio::test]
async fn a_backpressured_permissions_answer_during_a_recording_is_a_notice_not_a_stop() -> Result<()>
{
    // A full helper command queue refuses a permissions order; the meeting
    // that is already being recorded must not pay for it.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime
        .native_send("permissions", json!({"request":["microphone"]}))
        .await?;
    let permissions = receiver
        .recv()
        .await
        .context("permissions command absent")?;

    deliver(
        &mut runtime,
        json!({"event":"response","id":permissions["id"],"ok":false,
            "error":{"code":"native_backpressure","message":"cola de comandos llena"}}),
    )
    .await;

    assert!(
        !runtime.state.privacy_stopped(),
        "a rejected permissions order must not brake privacy"
    );
    assert_eq!(
        runtime.audio.mic_stream.as_deref(),
        Some(stream.as_str()),
        "the recording's stream must survive"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Recording,
        "the meeting recording must continue"
    );
    assert!(
        said::mentions(&runtime.state, "native_backpressure"),
        "the notice must name the helper code: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop"
    );
    Ok(())
}

#[tokio::test]
async fn a_privacy_stopped_rejection_of_a_queued_playback_keeps_the_original_cause() -> Result<()> {
    // The brake wins, and the helper's late refusal of an order queued
    // before it may neither rewrite the cause nor ask for a second stop.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .native_send(
            "playback_clip",
            json!({"playback_id":"synthetic-playback", "mime_type":"audio/wav",
                "provider":"synthetic", "audio_base64":"AA=="}),
        )
        .await?;
    let playback = receiver.recv().await.context("playback command absent")?;

    runtime
        .native_event(json!({"event":"hotkey","action":"privacy_stop","phase":"pressed"}))
        .await?;
    assert!(runtime.state.privacy_stopped());
    assert!(said::mentions(&runtime.state, "hotkey"));
    assert_eq!(
        receiver.recv().await.context("privacy stop absent")?["command"],
        "privacy_stop"
    );

    deliver(
        &mut runtime,
        json!({"event":"response","id":playback["id"],"ok":false,
            "error":{"code":"privacy_stopped","message":"el helper ya estaba frenado"}}),
    )
    .await;

    assert!(
        said::mentions(&runtime.state, "hotkey"),
        "the original cause must survive the late rejection: {:?}",
        runtime.state.durable.notice
    );
    assert!(
        !said::mentions(&runtime.state, "privacy_stopped"),
        "a rejection after the brake must not rewrite the notice: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop a second time"
    );
    Ok(())
}

#[tokio::test]
async fn a_second_entry_into_her_stop_does_not_ask_the_helper_to_stop_again() -> Result<()> {
    // Once the helper confirmed the stop, a later control joining the same
    // stop is a new notice line, not another privacy_stop order.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    let stop = receiver.recv().await.context("privacy stop absent")?;
    assert_eq!(stop["command"], "privacy_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await?;

    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Hotkey)
        .await;

    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "a second entry must not ask the helper to stop again"
    );
    assert!(
        said::mentions(&runtime.state, "order"),
        "the first control must stay visible: {:?}",
        runtime.state.durable.notice_log
    );
    assert!(
        said::mentions(&runtime.state, "hotkey"),
        "the new control must be visible: {:?}",
        runtime.state.durable.notice_log
    );
    Ok(())
}

#[tokio::test]
async fn an_audio_start_integrity_failure_cuts_the_recording_without_latching() -> Result<()> {
    // A capture order the Mac cannot honour with integrity cuts that
    // recording, says why and asks for no rearm; the helper is told to stop
    // capturing without its latch.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "meeting".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let audio_start = receiver.recv().await.context("start command absent")?;
    assert_eq!(audio_start["command"], "audio_start");

    deliver(
        &mut runtime,
        json!({"event":"response","id":audio_start["id"],"ok":false,
            "error":{"code":"audio_memory_unavailable","message":"sin memoria de audio"}}),
    )
    .await;

    assert!(
        !runtime.state.privacy_stopped(),
        "a rejected capture order cuts the recording, it does not latch"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    assert!(said::mentions(&runtime.state, "audio_memory_unavailable"));
    let mut orders = Vec::new();
    while let Ok(command) = receiver.try_recv() {
        orders.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert_eq!(
        orders.iter().filter(|name| *name == "capture_stop").count(),
        1,
        "the cut reaches the helper once: {orders:?}"
    );
    assert!(
        !orders.iter().any(|name| name == "privacy_stop"),
        "{orders:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_stop_cause_survives_the_wait_line_that_used_to_overwrite_it() -> Result<()> {
    // The incident: fail_closed wrote the
    // cause, then request_privacy_audio_stop replaced the whole notice with its
    // wait line, and every stop read identically. The sequence keeps both.
    // The route change cuts the live recording instead of latching,
    // and the cut's own wait line must not overwrite its cause either.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver).await?;

    runtime
        .native_event(json!({
            "event":"capture_stopped",
            "reason":"audio_device_changed",
            "privacy_stopped":false
        }))
        .await?;

    assert!(!runtime.state.privacy_stopped());
    // The cut says its cause as data (the helper's code), and the
    // wait for the Mac's confirmation is no longer a sentence on screen.
    assert!(said::mentions(&runtime.state, "audio_device_changed"));
    // The sequence holds the cause as its own entry, so a later notice cannot
    // erase what explained the stop.
    runtime.state.set_notice(NoticeCode::StoreProblem);
    assert!(said::mentions(&runtime.state, "audio_device_changed"));
    assert!(said::has(&runtime.state, NoticeCode::StoreProblem));
    Ok(())
}

#[tokio::test]
async fn a_helper_gap_in_a_live_recording_is_declared_and_the_recording_continues() -> Result<()> {
    // A discontinuity costs one declared gap, never the recording.
    // The audit's product decision: a hole is reported, not a reason to brake
    // live capture.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;

    runtime
        .native_event(json!({"event":"gap","source":"mic_local","stream_id":stream.as_str()}))
        .await?;

    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Recording);
    assert_eq!(recording.gaps, 1, "the gap must be declared exactly once");
    assert_eq!(
        runtime.audio.mic_stream.as_deref(),
        Some(stream.as_str()),
        "the capture stream must survive the declared gap"
    );
    assert!(
        !runtime.state.privacy_stopped(),
        "a declared gap must not latch"
    );
    assert!(
        said::has(&runtime.state, NoticeCode::AudioGap),
        "the notice must declare the discontinuity: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop for a declared gap"
    );
    Ok(())
}

#[tokio::test]
async fn a_rejected_clip_during_a_live_call_keeps_the_call_and_records_degradation() -> Result<()> {
    // The helper refused to queue the synthesized answer. The
    // taxonomy routes it as a degradation through the core decision, which
    // records its own notice entry, while the call and privacy stay intact.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    runtime
        .apply(Event::CallStart {
            id: "synthetic-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert_eq!(
        receiver.recv().await.context("playback stop absent")?["command"],
        "playback_stop"
    );
    let audio_start = receiver.recv().await.context("audio start absent")?;
    runtime
        .native_event(json!({
            "event":"response","id":audio_start["id"],"ok":true,
            "result":call_readback(&audio_start)
        }))
        .await?;
    assert!(runtime.state.durable.voice.call.is_some());
    runtime
        .native_send(
            "playback_clip",
            json!({"playback_id":"synthetic-playback", "mime_type":"audio/wav",
                "provider":"synthetic", "audio_base64":"AA=="}),
        )
        .await?;
    let playback = receiver.recv().await.context("playback command absent")?;
    let notices_before = runtime.state.durable.notice_log.records().len();

    deliver(
        &mut runtime,
        json!({"event":"response","id":playback["id"],"ok":false,
            "error":{"code":"invalid_audio","message":"audio no válido"}}),
    )
    .await;
    tokio::task::yield_now().await;

    assert!(
        runtime.state.durable.voice.call.is_some(),
        "the live call must survive a rejected playback"
    );
    assert!(
        said::mentions(&runtime.state, "invalid_audio"),
        "the notice must name the helper code: {:?}",
        runtime.state.durable.notice
    );
    assert!(
        said::has(&runtime.state, NoticeCode::CallActive),
        "the call notice must survive: {:?}",
        runtime.state.durable.notice_log
    );
    assert!(
        runtime.state.durable.notice_log.records().len() > notices_before,
        "a degradation reaches the core decision, which records a new notice entry: {:?}",
        runtime.state.durable.notice_log
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop"
    );
    Ok(())
}

#[tokio::test]
async fn a_permission_refusal_of_audio_start_interrupts_the_recording_without_latching()
-> Result<()> {
    // A refused audio_start leaves the recording Interrupted
    // with its gaps intact, names the missing permission and does not latch:
    // the brake stays reserved for integrity failures and the person's intent.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "meeting".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let audio_start = receiver.recv().await.context("start command absent")?;
    assert_eq!(audio_start["command"], "audio_start");

    deliver(
        &mut runtime,
        json!({"event":"response","id":audio_start["id"],"ok":false,
            "error":{"code":"microphone_permission_required","message":"Falta el permiso de micrófono de macOS"}}),
    )
    .await;

    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert_eq!(recording.gaps, 0, "a permission refusal is not a gap");
    assert!(
        !runtime.state.privacy_stopped(),
        "a permission refusal must not latch privacy"
    );
    assert!(
        said::has(&runtime.state, NoticeCode::MicrophonePermissionMissing),
        "the notice must name the missing permission: {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop for a permission refusal"
    );
    Ok(())
}

#[tokio::test]
async fn a_later_privacy_stop_does_not_relabel_a_refused_audio_start() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "refused-before-stop".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let audio_start = receiver.recv().await.context("start command absent")?;
    deliver(
        &mut runtime,
        json!({"event":"response","id":audio_start["id"],"ok":false,
            "error":{"code":"microphone_permission_required",
                "message":"Falta el permiso de micrófono de macOS"}}),
    )
    .await;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    let stop = receiver.recv().await.context("privacy stop absent")?;
    assert_eq!(stop["command"], "privacy_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-refused-before-stop")?
        .context("closure absent after stop")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "microphone_permission_required"
    );
    Ok(())
}

#[tokio::test]
async fn an_incomplete_drain_with_pending_audio_cuts_without_latching() -> Result<()> {
    // audio_stop answered with
    // audio_drain_incomplete while the recording still holds audio is an
    // integrity failure; it cuts that recording and never latches.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(super::frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop command absent")?;
    assert_eq!(stop["command"], "audio_stop");

    deliver(
        &mut runtime,
        json!({"event":"response","id":stop["id"],"ok":false,
            "error":{"code":"audio_drain_incomplete","message":"El audio no terminó de drenarse"}}),
    )
    .await;

    assert!(
        !runtime.state.privacy_stopped(),
        "an incomplete drain with pending audio cuts, it does not latch"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    let mut orders = Vec::new();
    while let Ok(command) = receiver.try_recv() {
        orders.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert_eq!(
        orders.iter().filter(|name| *name == "capture_stop").count(),
        1,
        "exactly one native capture stop is asked for: {orders:?}"
    );
    assert!(
        !orders.iter().any(|name| name == "privacy_stop"),
        "{orders:?}"
    );
    assert!(
        said::mentions(&runtime.state, "audio_drain_incomplete"),
        "the notice must name the helper code: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn two_entries_into_the_user_stop_keep_one_stop_two_causes_and_one_durable_snapshot()
-> Result<()> {
    // The second entry into an already-latched stop extends the
    // notice sequence, asks the helper for nothing and writes no new durable
    // state.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert_eq!(
        receiver.recv().await.context("privacy stop absent")?["command"],
        "privacy_stop"
    );
    let after_first = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("the first entry must persist the stop")?;
    let generation_after_first = runtime.state.generation();

    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Hotkey)
        .await;

    assert_eq!(
        count_privacy_stops(&mut receiver),
        0,
        "the helper must not be asked to stop a second time"
    );
    assert_eq!(
        runtime.state.generation(),
        generation_after_first,
        "a re-entry is not another privacy-stop decision"
    );
    let after_second = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("the snapshot must survive the second entry")?;
    assert_eq!(
        after_first, after_second,
        "the second entry must not write new durable state"
    );
    let texts = &runtime.state.durable.notice_log;
    assert!(
        said::mentions(&runtime.state, "order"),
        "the first cause must stay visible: {texts:?}"
    );
    assert!(
        said::mentions(&runtime.state, "hotkey"),
        "the new cause must be visible: {texts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn the_user_stop_stops_playback_at_the_helper() -> Result<()> {
    // The user's stop executes the core decision's effects instead of
    // reimplementing a partial teardown, so the helper hears the playback stop
    // too, not only the privacy stop.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;

    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;

    let mut commands = Vec::new();
    while let Ok(command) = receiver.try_recv() {
        commands.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert!(
        commands.iter().any(|command| command == "privacy_stop"),
        "the privacy stop must reach the helper: {commands:?}"
    );
    assert!(
        commands.iter().any(|command| command == "playback_stop"),
        "the core's stop-playback effect must reach the helper: {commands:?}"
    );
    Ok(())
}

const TEXT_TOKEN: &str = "55555555-5555-4555-8555-555555555555";
const TEXT_APPLICATION_INSTANCE: &str = "66666666-6666-4666-8666-666666666666";
const TEXT_DESCRIPTOR: &str = "9876543210abcdef9876543210abcdef9876543210abcdef9876543210abcdef";
const SELECTED_TEXT: &str = "selección sintética 🪐";

fn synthetic_png() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()?
            .write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
    }
    Ok(bytes)
}

fn inventory_with_two_windows(
    command: &serde_json::Value,
    generation: u64,
    selected_text: bool,
) -> serde_json::Value {
    json!({"event":"response","id":command["id"],"ok":true,"result":{
        "native_instance_id": INSTANCE,
        "privacy_generation": generation,
        "inventory_id": INVENTORY,
        "clock_domain": "mach_absolute_time",
        "observed_monotonic_ns": 1_000_000u64,
        "expires_monotonic_ns": 1_000_000u64 + orbit_core::sources::INVENTORY_TTL_NS,
        "selected_text_available": selected_text,
        "windows": [{
            "binding": {
                "native_instance_id": INSTANCE,
                "privacy_generation": generation,
                "inventory_id": INVENTORY,
                "selection_token": TOKEN,
                "window_id": 11,
                "application_pid": 4242,
                "application_instance_id": APPLICATION,
                "binding_quality": "observed_owner_and_window",
                "descriptor_hash": DESCRIPTOR
            },
            "window_id": 11,
            "application_pid": 4242,
            "application_name": "Safari",
            "bundle_id": "com.apple.Safari",
            "title": "Correo",
            "title_truncated": false,
            "unavailable_reason": null
        }, {
            "binding": {
                "native_instance_id": INSTANCE,
                "privacy_generation": generation,
                "inventory_id": INVENTORY,
                "selection_token": TEXT_TOKEN,
                "window_id": 12,
                "application_pid": 5252,
                "application_instance_id": TEXT_APPLICATION_INSTANCE,
                "binding_quality": "observed_owner_and_window",
                "descriptor_hash": TEXT_DESCRIPTOR
            },
            "window_id": 12,
            "application_pid": 5252,
            "application_name": "TextEdit",
            "bundle_id": "com.apple.TextEdit",
            "title": "Presupuesto 2027",
            "title_truncated": false,
            "unavailable_reason": null
        }],
        "omitted_count": 0u64
    }})
}

fn screenshot_answer(command: &serde_json::Value) -> Result<serde_json::Value> {
    Ok(json!({"event":"response","id":command["id"],"ok":true,
        "result":{"binding":command["window_binding"],"clock_domain":"mach_absolute_time",
        "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
        "mime_type":"image/png","width":2,"height":1,
        "data_base64":STANDARD.encode(synthetic_png()?),"source":"native_window"}}))
}

/// The same helper answer, carrying the exact bytes the test wants the helper
/// to have produced (the HTTP upload test feeds the 700 000-byte maximum PNG
/// through it).
fn screenshot_answer_of(command: &serde_json::Value, png: &[u8]) -> serde_json::Value {
    json!({"event":"response","id":command["id"],"ok":true,
        "result":{"binding":command["window_binding"],"clock_domain":"mach_absolute_time",
        "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
        "mime_type":"image/png","width":2,"height":1,
        "data_base64":STANDARD.encode(png),"source":"native_window"}})
}

/// CRC-32 (IEEE, the PNG chunk checksum) for the padding chunk below. Kept
/// local because the daemon only links a CRC library inside orbit-core.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A valid PNG of exactly the helper's ceiling (`MAX_SOURCE_BYTES`, 700 000
/// bytes): the synthetic 2x1 image padded to size with a private ancillary
/// chunk before IEND, so the byte count does not depend on the compressor.
pub(in crate::daemon) fn maximal_png() -> Result<Vec<u8>> {
    let base = synthetic_png()?;
    let target = orbit_core::sources::MAX_SOURCE_BYTES;
    let padding = target
        .checked_sub(base.len() + 12)
        .context("the synthetic PNG leaves no room for padding")?;
    let kind = *b"prIv";
    let mut checksum = Vec::with_capacity(4 + padding);
    checksum.extend_from_slice(&kind);
    checksum.extend(std::iter::repeat_n(0x36u8, padding));
    let mut png = base[..base.len() - 12].to_vec();
    png.extend_from_slice(&(padding as u32).to_be_bytes());
    png.extend_from_slice(&kind);
    png.extend(std::iter::repeat_n(0x36u8, padding));
    png.extend_from_slice(&crc32(&checksum).to_be_bytes());
    png.extend_from_slice(&base[base.len() - 12..]);
    anyhow::ensure!(
        png.len() == target,
        "the maximal PNG does not have the expected size"
    );
    Ok(png)
}

pub(in crate::daemon) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The one HTTP request the fake hub received, as the daemon sent it.
pub(in crate::daemon) struct RecordedPut {
    pub(in crate::daemon) method: String,
    pub(in crate::daemon) path: String,
    headers: std::collections::HashMap<String, String>,
    pub(in crate::daemon) body: Vec<u8>,
}

impl RecordedPut {
    pub(in crate::daemon) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Reads one full HTTP/1.1 request (head plus Content-Length body).
async fn read_put(socket: &mut tokio::net::TcpStream) -> Option<RecordedPut> {
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let mut piece = [0u8; 16384];
    loop {
        let read = socket.read(&mut piece).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&piece[..read]);
        let Some(head_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
        let length: usize = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })?;
        if buffer.len() < head_end + 4 + length {
            continue;
        }
        let mut lines = head.lines();
        let request = lines.next()?;
        let mut parts = request.split(' ');
        let method = parts.next()?.to_owned();
        let path = parts.next()?.to_owned();
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            })
            .collect();
        return Some(RecordedPut {
            method,
            path,
            headers,
            body: buffer[head_end + 4..head_end + 4 + length].to_vec(),
        });
    }
}

/// A synthetic capture hub: records every `PUT /v1/captures/{id}` it receives
/// and, unless it is asked to fail, holds its confirmation until the test
/// releases it. That hold is the proof of ordering: nothing may answer the
/// bridge while the upload's outcome is still unknown.
pub(in crate::daemon) async fn capture_hub(
    reject: bool,
) -> Result<(
    std::net::SocketAddr,
    mpsc::Receiver<RecordedPut>,
    std::sync::Arc<tokio::sync::Notify>,
)> {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (records, records_rx) = mpsc::channel(4);
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let gate = release.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let Some(record) = read_put(&mut socket).await else {
                continue;
            };
            if records.send(record).await.is_err() {
                return;
            }
            if !reject {
                gate.notified().await;
            }
            let response = if reject {
                &b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..]
            } else {
                &b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"durable\":true}"[..]
            };
            let _ = socket.write_all(response).await;
            let _ = socket.shutdown().await;
        }
    });
    Ok((address, records_rx, release))
}

pub(in crate::daemon) async fn capture_hub_at_capacity()
-> Result<(std::net::SocketAddr, mpsc::Receiver<RecordedPut>)> {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (records, records_rx) = mpsc::channel(4);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let Some(record) = read_put(&mut socket).await else {
                continue;
            };
            if records.send(record).await.is_err() {
                return;
            }
            let body = br#"{"code":"too_many","refusal":{"code":"too_many","resource":"capture","count":5,"max":4,"retry":{"kind":"after_fix"}}}"#;
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.write_all(body).await;
            let _ = socket.shutdown().await;
        }
    });
    Ok((address, records_rx))
}

fn selected_text_answer(command: &serde_json::Value) -> serde_json::Value {
    json!({"event":"response","id":command["id"],"ok":true,
        "result":{"binding":command["window_binding"],"clock_domain":"mach_absolute_time",
        "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
        "mime_type":"text/plain; charset=utf-8","source":"native_selected_text",
        "text":SELECTED_TEXT,
        "range":{"location":7,"length":SELECTED_TEXT.encode_utf16().count(),"unit":"utf16_code_units"},
        "ax_binding_quality":"unique_owner_title_bounds",
        "selection_scope":"application_focused_element","sensitivity":"not_certified"}})
}

async fn remote_request(
    runtime: &mut crate::daemon::Runtime,
    id: &str,
    capability: &str,
    parameters: serde_json::Value,
) -> Result<()> {
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request",
            "data": {"request_id": id, "capability": capability, "parameters": parameters}
        })))
        .await
}

async fn helper_command(
    helper: &mut mpsc::Receiver<serde_json::Value>,
    expected: &str,
) -> Result<serde_json::Value> {
    let sent = tokio::time::timeout(Duration::from_secs(2), helper.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the helper never received «{expected}»"))?
        .context("the fake helper disconnected")?;
    assert_eq!(
        sent["command"], expected,
        "orden inesperada al helper: {sent}"
    );
    Ok(sent)
}

async fn hub_reply(hub: &mut mpsc::Receiver<serde_json::Value>) -> Result<serde_json::Value> {
    tokio::time::timeout(Duration::from_secs(2), hub.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the remote request got no answer"))?
        .context("the hub channel closed")
}

fn capture_work_receiver(runtime: &mut crate::daemon::Runtime) -> mpsc::Receiver<WorkResult> {
    let (sender, receiver) = mpsc::channel(16);
    runtime.work_events = sender;
    receiver
}

async fn finish_capture_upload(
    runtime: &mut crate::daemon::Runtime,
    completions: &mut mpsc::Receiver<WorkResult>,
) -> Result<()> {
    // Test hang guard only: the 100 s bound already measured below for the
    // maximal PNG covers sealing and PUT under load. It is not an upload
    // deadline; production applies its own 20 s limit after local sealing.
    let completion = tokio::time::timeout(Duration::from_secs(100), completions.recv())
        .await?
        .context("the upload delivered no WorkResult to the actor")?;
    runtime.work_event(completion).await
}

pub(in crate::daemon) async fn measure_status_rounds(
    runtime: &mut crate::daemon::Runtime,
) -> Result<(u128, u128)> {
    let mut micros = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = Instant::now();
        runtime.command(crate::daemon::Command::Status).await?;
        micros.push(Instant::now().duration_since(started).as_micros());
    }
    micros.sort_unstable();
    Ok((micros[98], micros[99]))
}

#[tokio::test]
async fn bridge_picks_the_foreground_window_a_title_substring_or_refuses_by_name() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    let mut completions = capture_work_receiver(&mut runtime);
    let (address, mut puts, release) = capture_hub(false).await?;
    runtime.settings.hub_url = Some(format!("http://{address}"));
    runtime.link.token = Some("synthetic device credential".to_owned());

    remote_request(&mut runtime, "bridge-1", "screenshot", json!({})).await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    assert_eq!(shot["window_binding"]["selection_token"], TOKEN);
    release.notify_one();
    runtime.native_event(screenshot_answer(&shot)?).await?;
    let _ = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the daemon did not upload the capture by HTTP"))?
        .context("the fake hub shut down")?;
    finish_capture_upload(&mut runtime, &mut completions).await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["result"]["application"], "Safari");
    assert_eq!(reply["params"]["result"]["title"], "Correo");
    assert!(runtime.assistant.bridge_requests.is_empty());

    remote_request(
        &mut runtime,
        "bridge-2",
        "screenshot",
        json!({"title": "Presupuesto"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    assert_eq!(
        shot["window_binding"]["selection_token"], TEXT_TOKEN,
        "the title did not match as a substring: {shot}"
    );
    release.notify_one();
    runtime.native_event(screenshot_answer(&shot)?).await?;
    let _ = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the daemon did not upload the capture by HTTP"))?
        .context("the fake hub shut down")?;
    finish_capture_upload(&mut runtime, &mut completions).await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["result"]["application"], "TextEdit");
    assert!(runtime.assistant.bridge_requests.is_empty());

    remote_request(
        &mut runtime,
        "bridge-3",
        "screenshot",
        json!({"application": "Nadie"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-3");
    assert_eq!(
        reply["params"]["refusal"]["expected"]["reason"], "window_not_found",
        "{reply}"
    );
    assert_eq!(reply["params"]["refusal"]["parameter"], "selection");
    assert!(
        helper.try_recv().is_err(),
        "a capture was asked for without matching any window"
    );
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn bridge_serves_the_selected_text_of_a_named_application() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    // The selected text needs Accessibility besides the screen.
    grant(&mut runtime, true, true)?;
    remote_request(
        &mut runtime,
        "bridge-1",
        "selected_text",
        json!({"application": "com.apple.textedit"}),
    )
    .await?;

    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            true,
        ))
        .await?;
    let text = helper_command(&mut helper, "selected_text").await?;
    assert_eq!(
        text["window_binding"]["selection_token"], TEXT_TOKEN,
        "the selection of another window was read: {text}"
    );
    runtime.native_event(selected_text_answer(&text)).await?;

    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    let result = &reply["params"]["result"];
    assert_eq!(result["application"], "TextEdit");
    assert_eq!(result["title"], "Presupuesto 2027");
    assert_eq!(result["text"], SELECTED_TEXT);
    assert_eq!(
        result.as_object().map(|object| object.len()),
        Some(3),
        "the reply carries fields outside the agreed shape: {result}"
    );

    let receipt = runtime
        .state
        .views
        .source_receipt
        .as_ref()
        .context("no receipt of what Deimos read was kept in memory")?;
    assert_eq!(receipt.intent.annotation, "Deimos");
    assert_eq!(receipt.sha256, sha256_hex(SELECTED_TEXT.as_bytes()));

    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplySent),
        "the final notice must tell sending from receiving: {:?}",
        runtime.state.durable.notice
    );
    let said = serde_json::to_string(&runtime.state.durable.notice_log)?;
    assert!(
        !said.contains(SELECTED_TEXT),
        "the notice shows the content of the selection: {said}"
    );
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn malformed_native_text_result_tells_the_remote_asker_the_field_and_type() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, true)?;
    remote_request(
        &mut runtime,
        "bridge-bad-text",
        "selected_text",
        json!({"application":"com.apple.textedit"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            true,
        ))
        .await?;
    let text = helper_command(&mut helper, "selected_text").await?;
    let mut malformed = selected_text_answer(&text);
    malformed["result"]["text"] = json!(42);
    runtime.native_event(malformed).await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["request_id"], "bridge-bad-text");
    assert_eq!(reply["params"]["refusal"]["field"], "text", "{reply}");
    assert_eq!(
        reply["params"]["refusal"]["received_type"], "number",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["code"], "helper_reply_invalid",
        "{reply}"
    );
    assert_eq!(reply["params"]["refusal"]["field"], "text", "{reply}");
    assert!(runtime.state.views.source_receipt.is_none());
    assert!(helper.try_recv().is_err(), "the invalid read was retried");
    Ok(())
}

#[tokio::test]
async fn screen_read_reports_full_and_retired_hub_without_content() -> Result<()> {
    for retired in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _hub, mut helper) = bridge_fixture(directory.path()).await?;
        grant(&mut runtime, true, true)?;
        let (sender, mut frames) = mpsc::channel(1);
        sender.try_send(json!({"fixture":"occupied"}))?;
        runtime.link.outbound.install(sender);
        if retired {
            runtime
                .link
                .outbound
                .retire(crate::daemon::hub_link::Retirement::PrivacyStop);
        }
        remote_request(
            &mut runtime,
            "screen-read",
            "selected_text",
            json!({"application": "com.apple.textedit"}),
        )
        .await?;
        let list = helper_command(&mut helper, "screen_sources").await?;
        runtime
            .native_event(inventory_with_two_windows(
                &list,
                runtime.state.generation(),
                true,
            ))
            .await?;
        let text = helper_command(&mut helper, "selected_text").await?;
        runtime.native_event(selected_text_answer(&text)).await?;
        // Not delivered is said; its cause (the stop, the full
        // channel) stays in daemon.log.
        let notice = &runtime.state.durable.notice;
        assert_eq!(
            runtime.state.notice_code(),
            Some(NoticeCode::BridgeReplyUndelivered),
            "{notice:?}"
        );
        if !retired {
            assert_eq!(frames.try_recv()?["fixture"], "occupied");
            assert!(frames.try_recv().is_err());
        }
        let said = serde_json::to_string(&runtime.state.durable.notice_log)?;
        assert!(!said.contains(SELECTED_TEXT), "{said}");
    }
    Ok(())
}

#[tokio::test]
async fn bridge_uploads_a_maximal_capture_by_http_before_replying_by_websocket() -> Result<()> {
    // The split: the bytes go by HTTP, the control by WebSocket. With the
    // helper's ceiling PNG (700 000 bytes) the «device.reply» frame with
    // base64 measured 933 542 bytes and the hub cuts at 128 KiB, so the
    // reply can only carry the identifier of what was uploaded.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    let mut completions = capture_work_receiver(&mut runtime);
    let png = maximal_png()?;
    orbit_core::sources::validate_png(&png, 2, 1)
        .context("the synthetic ceiling PNG is not a valid image")?;
    let (address, mut puts, release) = capture_hub(false).await?;
    let token = "synthetic device credential";
    runtime.settings.hub_url = Some(format!("http://{address}"));
    runtime.link.token = Some(token.to_owned());

    remote_request(
        &mut runtime,
        "bridge-1",
        "screenshot",
        json!({"application": "TextEdit"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    assert_eq!(shot["window_binding"]["selection_token"], TEXT_TOKEN);
    assert_eq!(shot["window_binding"]["window_id"], 12);
    let answer = screenshot_answer_of(&shot, &png);

    // The handler frees the actor before the hub confirms the PUT. The
    // remote reply keeps waiting for the receipt of the same maximal PNG.
    runtime.native_event(answer).await?;
    // A hang guard, not an upload budget: 100 s is over 2× the worst full run
    // measured (48.4 s, 10 runs with 16 spinners and 8 writers on the test's
    // CPUs).
    const PUT_HANG_GUARD: Duration = Duration::from_secs(100);
    let upload_wait_started = Instant::now();
    let record = tokio::time::timeout(PUT_HANG_GUARD, puts.recv())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "no capture PUT was observed after {:.1} s (hang guard: {} s)",
                Instant::now()
                    .duration_since(upload_wait_started)
                    .as_secs_f64(),
                PUT_HANG_GUARD.as_secs()
            )
        })?
        .context("the fake hub shut down")?;
    assert!(
        hub.try_recv().is_err(),
        "device.reply arrived before the upload was confirmed"
    );
    release.notify_one();
    finish_capture_upload(&mut runtime, &mut completions).await?;

    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    let result = &reply["params"]["result"];
    let digest = sha256_hex(&png);
    assert_eq!(
        result["sha256"], digest,
        "the sha256 is not that of the uploaded bytes"
    );
    assert_eq!(result["application"], "TextEdit");
    assert_eq!(result["title"], "Presupuesto 2027");
    assert_eq!(result["width"], 2);
    assert_eq!(result["height"], 1);
    assert_eq!(result["mime_type"], "image/png");
    assert_eq!(
        result.as_object().map(|object| object.len()),
        Some(7),
        "the reply carries fields outside the agreed shape: {result}"
    );
    let capture_id = result["capture_id"]
        .as_str()
        .context("the reply carries no capture_id")?
        .to_owned();
    assert!(
        !capture_id.is_empty(),
        "the capture_id of the PUT and the reply cannot be empty"
    );
    assert!(
        result.get("data_base64").is_none(),
        "the bridge reply carries the PNG bytes: {result}"
    );
    let wire = serde_json::to_vec(&reply)?;
    assert!(
        !String::from_utf8_lossy(&wire).contains(&STANDARD.encode(&png)),
        "the reply frame contains the whole PNG"
    );
    assert!(
        wire.len() < crate::hub::MAX_OUTGOING_FRAME_BYTES,
        "the {}-byte frame does not fit the hub WebSocket maximum ({} bytes)",
        wire.len(),
        crate::hub::MAX_OUTGOING_FRAME_BYTES
    );

    assert_eq!(
        record.method, "PUT",
        "it was not a PUT upload: {:?}",
        record.path
    );
    assert_eq!(record.path, format!("/v1/captures/{capture_id}"));
    assert_eq!(
        record.header("authorization"),
        Some(format!("Bearer {token}").as_str()),
        "the upload does not carry the device pairing"
    );
    assert_eq!(
        record.header("x-orbit-request"),
        Some("bridge-1"),
        "the upload does not carry the bridge request_id"
    );
    assert_eq!(
        record.header("x-orbit-sha256"),
        Some(digest.as_str()),
        "the sha256 header is not the digest of the body"
    );
    assert_eq!(
        record.header("content-type"),
        Some("image/png"),
        "the upload does not declare the image type"
    );
    assert_eq!(
        record.body, png,
        "the helper's exact bytes were not uploaded"
    );
    let receipt = runtime
        .state
        .views
        .source_receipt
        .as_ref()
        .context("no receipt of what Deimos saw was kept in memory")?;
    assert_eq!(receipt.intent.annotation, "Deimos");
    assert_eq!(receipt.sha256, digest);
    assert_eq!(receipt.bytes, png.len() as u64);
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplySent)
    );
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn helper_capture_returns_to_actor_while_upload_is_held() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    let stream = start(&mut runtime, &mut helper).await?;
    let idle_status = measure_status_rounds(&mut runtime).await?;
    let (work, mut completions) = mpsc::channel(16);
    runtime.work_events = work;
    let (address, mut puts, release) = capture_hub(false).await?;
    runtime.settings.hub_url = Some(format!("http://{address}"));
    runtime.link.token = Some("synthetic device credential".into());
    remote_request(&mut runtime, "held-upload-capture", "screenshot", json!({})).await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    let mut answer = Box::pin(runtime.native_event(screenshot_answer(&shot)?));
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), &mut answer)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the helper answer held the actor {:.3} s while the upload is still held",
                Instant::now().duration_since(started).as_secs_f64()
            )
        })??;
    drop(answer);
    let put = tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("the fake hub closed before the PUT")?;
    assert_eq!(put.method, "PUT");
    for sequence in 0..30 {
        runtime
            .native_event(super::frame(&stream, sequence, 100))
            .await?;
    }
    let held_status = measure_status_rounds(&mut runtime).await?;
    eprintln!(
        "helper_capture_status_us idle_p99={} idle_worst={} held_p99={} held_worst={} n=100",
        idle_status.0, idle_status.1, held_status.0, held_status.1
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("no recording")?
            .gaps,
        0,
        "the capture introduced gaps in the helper's 30 frames"
    );
    assert!(
        hub.try_recv().is_err(),
        "Deimos was answered before the PUT was confirmed"
    );
    // A later local capture may replace the single volatile receipt before
    // the held upload finishes. The bridge already owns the validated metadata.
    runtime.state.views.source_receipt = None;
    release.notify_one();
    let completion = tokio::time::timeout(Duration::from_secs(5), completions.recv())
        .await?
        .context("the upload result did not arrive")?;
    runtime.work_event(completion).await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["request_id"], "held-upload-capture");
    assert!(reply["params"]["result"]["capture_id"].is_string());
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

fn attach_pending_upload(
    runtime: &mut Runtime,
    capture_id: &str,
) -> Result<(
    crate::daemon::ticketed::WorkTicket,
    tokio::task::AbortHandle,
)> {
    let mut work = crate::daemon::ticketed::Ticketed::logical();
    let ticket = work.replace_logical(
        runtime.state.generation(),
        runtime.turn.op.clone(),
        (),
        |_| tokio::spawn(std::future::pending::<()>()),
    );
    let abort = work.task_handle().context("upload worker")?.abort_handle();
    runtime.sources.bridge_uploads.insert(
        crate::daemon::BridgeKind::Screen,
        crate::daemon::sources::BridgeUpload {
            capture_id: capture_id.into(),
            screenshot: None,
            work,
        },
    );
    Ok((ticket, abort))
}

#[tokio::test]
async fn computer_revocation_keeps_an_unrelated_capture_and_its_upload() -> Result<()> {
    use crate::daemon::bridge::{CaptureUploadOutcome, PendingCapture};
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, _helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    remote_request(&mut runtime, "unrelated-capture", "screenshot", json!({})).await?;
    let (ticket, abort) = attach_pending_upload(&mut runtime, "unrelated-source")?;

    runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    assert!(
        hub.try_recv().is_err(),
        "computer retirement answered a capture"
    );
    assert!(!runtime.assistant.bridge_requests.is_empty());
    assert!(
        !abort.is_finished(),
        "computer retirement aborted the capture upload"
    );

    runtime
        .work_event(WorkResult::BridgeCaptureUpload {
            ticket,
            request_id: "unrelated-capture".into(),
            capture: PendingCapture::Engine {
                capture_id: "unrelated-source".into(),
                application: "Fixture".into(),
                title: "Fixture".into(),
                width: 1,
                height: 1,
                mime_type: "image/png".into(),
            },
            outcome: CaptureUploadOutcome::Uploaded("synthetic-sha256".into()),
        })
        .await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["request_id"], "unrelated-capture");
    assert_eq!(reply["params"]["result"]["capture_id"], "unrelated-source");
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn privacy_and_cancellation_abort_upload_and_answer_only_once() -> Result<()> {
    use crate::daemon::bridge::{CaptureUploadOutcome, PendingCapture};
    for privacy in [true, false] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut hub, _helper) = bridge_fixture(directory.path()).await?;
        grant(&mut runtime, true, false)?;
        remote_request(&mut runtime, "stopped-upload", "screenshot", json!({})).await?;
        let (ticket, abort) = attach_pending_upload(&mut runtime, "late-source")?;
        if privacy {
            runtime.apply(Event::PrivacyStop).await?;
        } else {
            runtime.release_bridge_requests(crate::daemon::BridgeReleaseReason::Cancelled);
        }
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
        let reply = hub_reply(&mut hub).await?;
        assert_eq!(reply["params"]["request_id"], "stopped-upload");
        if privacy {
            assert_eq!(reply["params"]["refusal"]["code"], "privacy_stopped");
        } else {
            assert_eq!(reply["params"]["refusal"]["code"], "read_interrupted");
            assert_eq!(reply["params"]["refusal"]["cause"]["kind"], "cancelled");
        }
        runtime
            .work_event(WorkResult::BridgeCaptureUpload {
                ticket,
                request_id: "stopped-upload".into(),
                capture: PendingCapture::Helper {
                    source_id: "late-source".into(),
                },
                outcome: CaptureUploadOutcome::Uploaded("late-digest".into()),
            })
            .await?;
        assert!(
            hub.try_recv().is_err(),
            "a late result answered a second time"
        );
    }
    Ok(())
}

#[tokio::test]
async fn upload_timeout_names_wait_and_deadline_and_late_result_is_ignored() -> Result<()> {
    use crate::daemon::bridge::{CaptureUploadOutcome, PendingCapture};
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, _helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    remote_request(&mut runtime, "timed-out-upload", "screenshot", json!({})).await?;
    let (ticket, abort) = attach_pending_upload(&mut runtime, "source")?;
    runtime
        .work_event(WorkResult::BridgeCaptureUpload {
            ticket: ticket.clone(),
            request_id: "timed-out-upload".into(),
            capture: PendingCapture::Helper {
                source_id: "source".into(),
            },
            outcome: CaptureUploadOutcome::TimedOut { waited_ms: 20_003 },
        })
        .await?;
    abort.abort();
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["params"]["refusal"]["code"], "timeout");
    assert_eq!(reply["params"]["refusal"]["waited_ms"], 20_003);
    assert_eq!(reply["params"]["refusal"]["deadline_ms"], 20_000);
    // The numbers are in the reply Deimos reads; the Mac records the code.
    assert!(said::mentions(&runtime.state, "capture_upload_timeout"));
    assert!(runtime.assistant.bridge_requests.is_empty());
    runtime
        .work_event(WorkResult::BridgeCaptureUpload {
            ticket,
            request_id: "timed-out-upload".into(),
            capture: PendingCapture::Helper {
                source_id: "source".into(),
            },
            outcome: CaptureUploadOutcome::Uploaded("late-digest".into()),
        })
        .await?;
    assert!(
        hub.try_recv().is_err(),
        "a late result answered a second time"
    );
    Ok(())
}

#[tokio::test]
async fn old_generation_upload_cannot_answer_the_current_request() -> Result<()> {
    use crate::daemon::bridge::{CaptureUploadOutcome, PendingCapture};
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, _helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    remote_request(
        &mut runtime,
        "old-generation-upload",
        "screenshot",
        json!({}),
    )
    .await?;
    let old_generation = runtime.state.generation();
    {
        let fixture_value = serde_json::json!(old_generation.wrapping_add(1));
        crate::daemon::authority_fixture::set(&mut runtime.state, "generation", fixture_value);
    }
    let (ticket, abort) = attach_pending_upload(&mut runtime, "new-source")?;
    runtime
        .work_event(WorkResult::BridgeCaptureUpload {
            ticket: crate::daemon::ticketed::WorkTicket::test(old_generation),
            request_id: "old-generation-upload".into(),
            capture: PendingCapture::Helper {
                source_id: "old-source".into(),
            },
            outcome: CaptureUploadOutcome::Uploaded("old-digest".into()),
        })
        .await?;
    abort.abort();
    assert!(
        hub.try_recv().is_err(),
        "an old generation answered the hub"
    );
    runtime
        .work_event(WorkResult::BridgeCaptureUpload {
            ticket,
            request_id: "old-generation-upload".into(),
            capture: PendingCapture::Helper {
                source_id: "old-source".into(),
            },
            outcome: CaptureUploadOutcome::Uploaded("old-digest".into()),
        })
        .await?;
    assert!(
        hub.try_recv().is_err(),
        "a replaced upload answered the hub"
    );
    assert!(!runtime.assistant.bridge_requests.is_empty());
    runtime.release_bridge_requests(crate::daemon::BridgeReleaseReason::Cancelled);
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(
        reply["params"]["refusal"]["code"], "read_interrupted",
        "{reply}"
    );
    assert_eq!(
        reply["params"]["refusal"]["cause"]["kind"], "cancelled",
        "{reply}"
    );
    assert!(hub.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn bridge_refuses_by_name_when_the_capture_upload_fails() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    let mut completions = capture_work_receiver(&mut runtime);
    let png = maximal_png()?;
    let (address, mut puts, _release) = capture_hub(true).await?;
    runtime.settings.hub_url = Some(format!("http://{address}"));
    runtime.link.token = Some("synthetic device credential".to_owned());

    remote_request(
        &mut runtime,
        "bridge-1",
        "screenshot",
        json!({"application": "TextEdit"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    runtime
        .native_event(screenshot_answer_of(&shot, &png))
        .await?;

    let record = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the daemon did not try to upload the capture by HTTP"))?
        .context("the fake hub shut down")?;
    assert!(
        record.path.starts_with("/v1/captures/"),
        "the upload did not use the agreed route: {}",
        record.path
    );

    finish_capture_upload(&mut runtime, &mut completions).await?;

    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    assert_eq!(
        reply["params"]["refusal"]["code"], "upstream_failed",
        "{reply}"
    );
    let wire = serde_json::to_vec(&reply)?;
    assert!(
        !String::from_utf8_lossy(&wire).contains("capture_id"),
        "the refusal leaked a capture_id: {reply}"
    );
    assert!(
        runtime.assistant.bridge_requests.is_empty(),
        "the upload failure left the slot occupied"
    );
    Ok(())
}

#[tokio::test]
async fn helper_capture_capacity_reaches_device_reply_with_its_counts() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
    grant(&mut runtime, true, false)?;
    let mut completions = capture_work_receiver(&mut runtime);
    let png = maximal_png()?;
    let (address, mut puts) = capture_hub_at_capacity().await?;
    runtime.settings.hub_url = Some(format!("http://{address}"));
    runtime.link.token = Some("synthetic device credential".to_owned());

    remote_request(
        &mut runtime,
        "bridge-capacity",
        "screenshot",
        json!({"application": "TextEdit"}),
    )
    .await?;
    let list = helper_command(&mut helper, "screen_sources").await?;
    runtime
        .native_event(inventory_with_two_windows(
            &list,
            runtime.state.generation(),
            false,
        ))
        .await?;
    let shot = helper_command(&mut helper, "screenshot").await?;
    runtime
        .native_event(screenshot_answer_of(&shot, &png))
        .await?;

    let put = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await?
        .context("the helper did not upload the capture")?;
    assert!(put.path.starts_with("/v1/captures/"));
    finish_capture_upload(&mut runtime, &mut completions).await?;
    let reply = hub_reply(&mut hub).await?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-capacity");
    assert_eq!(reply["params"]["refusal"]["code"], "too_many");
    assert_eq!(reply["params"]["refusal"]["resource"], "capture");
    assert_eq!(reply["params"]["refusal"]["count"], 5);
    assert_eq!(reply["params"]["refusal"]["max"], 4);
    assert!(runtime.sources.bridge_uploads.is_empty());
    assert!(runtime.assistant.bridge_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn bridge_answers_a_helper_capture_failure_with_the_helpers_code() -> Result<()> {
    for helper_code in [
        "privacy_stopped",
        "window_identity_changed",
        "screenshot_busy",
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut hub, mut helper) = bridge_fixture(directory.path()).await?;
        grant(&mut runtime, true, false)?;
        remote_request(
            &mut runtime,
            "bridge-1",
            "screenshot",
            json!({"application": "TextEdit"}),
        )
        .await?;
        let list = helper_command(&mut helper, "screen_sources").await?;
        runtime
            .native_event(inventory_with_two_windows(
                &list,
                runtime.state.generation(),
                false,
            ))
            .await?;
        let shot = helper_command(&mut helper, "screenshot").await?;
        runtime
            .native_event(json!({
                "event": "response", "id": shot["id"], "ok": false,
                "error": {"code": helper_code, "message": "untrusted text"}
            }))
            .await?;

        let failure = runtime
            .sources
            .last_failure
            .as_ref()
            .context("source failure")?;
        let typed = serde_json::to_value(&failure.refusal)?;
        if helper_code == "privacy_stopped" {
            assert_eq!(typed["code"], helper_code);
        } else {
            assert_eq!(typed["code"], "helper_refused");
            assert_eq!(typed["helper_code"], helper_code);
        }
        assert!(!typed.to_string().contains("untrusted text"));
        let reply = hub_reply(&mut hub).await?;
        assert_eq!(reply["method"], "device.reply");
        assert_eq!(reply["params"]["request_id"], "bridge-1");
        if helper_code == "privacy_stopped" {
            assert_eq!(reply["params"]["refusal"]["code"], helper_code, "{reply}");
        } else {
            assert_eq!(
                reply["params"]["refusal"]["code"], "helper_refused",
                "{reply}"
            );
            assert_eq!(
                reply["params"]["refusal"]["helper_code"], helper_code,
                "{reply}"
            );
        }
        assert_ne!(reply["params"]["refusal"]["text"], "native_refused");
        assert!(
            runtime.assistant.bridge_requests.is_empty(),
            "the helper failure left the slot occupied"
        );
        assert!(
            helper.try_recv().is_err(),
            "the capture was retried after the helper failure"
        );
        assert!(runtime.state.views.source_receipt.is_none());
    }
    Ok(())
}

/// The app's source dropdown reads the helper's application
/// inventory through the daemon. Refreshing asks the helper for
/// `application_sources` and publishes the rows it answered; when the helper
/// leaves, the published inventory empties.
#[tokio::test]
async fn audio_sources_refresh_asks_the_helper_and_clears_when_it_leaves() -> Result<()> {
    use orbit_core::protocol::Command;
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.command(Command::AudioSourcesRefresh).await?;
    // The inventory asks in reads of eight; Chrome is answered by the read
    // that names it, wherever the list puts it (call apps made it longer).
    let mut unanswered = Vec::new();
    let request = loop {
        let request = helper.recv().await.context("inventory request absent")?;
        assert_eq!(request["command"], "application_sources");
        if request["bundle_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == "com.google.Chrome"))
        {
            break request;
        }
        unanswered.push(request);
    };
    runtime
        .native_event(json!({"event":"response","id":request["id"],"ok":true,
        "result":{
            "native_instance_id":"00000000-0000-0000-0000-000000000001",
            "privacy_generation":0,"clock_domain":"mach_absolute_time",
            "observed_monotonic_ns":1_000,"completed_monotonic_ns":2_000,"omitted_count":0,
            "applications":[{
                "application_pid":321,"application_name":"Chrome","bundle_id":"com.google.Chrome",
                "unavailable_reason":null,
                "binding":{
                    "native_instance_id":"00000000-0000-0000-0000-000000000001",
                    "privacy_generation":0,
                    "application_pid":321,
                    "application_instance_id":"00000000-0000-0000-0000-000000000002",
                    "identity_token":"00000000-0000-0000-0000-000000000003",
                    "identity":{"bundle_id":"com.google.Chrome",
                        "bundle_path":"/Applications/Google Chrome.app",
                        "designated_requirement_sha256":"a".repeat(64)},
                    "clock_domain":"mach_absolute_time",
                    "observed_monotonic_ns":1_000,"expires_monotonic_ns":15_000_000_000_u64}
            }]}}))
        .await?;
    while let Ok(command) = helper.try_recv() {
        unanswered.push(command);
    }
    for request in unanswered {
        assert_eq!(request["command"], "application_sources");
        runtime
            .native_event(json!({"event":"response","id":request["id"],"ok":true,
            "result": {"native_instance_id":"00000000-0000-0000-0000-000000000001",
                "privacy_generation":0,"clock_domain":"mach_absolute_time",
                "observed_monotonic_ns":1_000,"completed_monotonic_ns":2_000,
                "omitted_count":0,"applications":[]}}))
            .await?;
    }
    let orbit_core::topics::TopicData::Recording(orbit_core::topics::Knowledge::Present {
        value,
        ..
    }) = crate::daemon::topic(&runtime, orbit_core::topics::TopicId::Recording)
    else {
        panic!("recording topic absent")
    };
    let orbit_core::topics::Knowledge::Present {
        value: applications,
        ..
    } = value.applications
    else {
        panic!("inventory absent")
    };
    assert_eq!(
        applications,
        vec![orbit_core::topics::recording::Application {
            pid: 321,
            name: "Chrome".into()
        }]
    );
    let published = runtime.state.views.audio_sources.clone();
    assert_eq!(
        published.len(),
        1,
        "the helper's inventory was not published: {published:?}"
    );
    assert_eq!(
        published[0],
        orbit_core::AudioSource {
            application_pid: 321,
            application_name: "Chrome".into(),
            bundle_id: "com.google.Chrome".into(),
        }
    );

    // The helper leaving takes the inventory with it.
    runtime
        .native_event(json!({"event":"helper_disconnected"}))
        .await?;
    assert!(
        matches!(crate::daemon::topic(&runtime, orbit_core::topics::TopicId::Recording), orbit_core::topics::TopicData::Recording(orbit_core::topics::Knowledge::Present { value, .. }) if matches!(value.applications, orbit_core::topics::Knowledge::Absent { refusal: orbit_core::refusal::Refusal::HelperNotRunning })),
        "a disconnected helper left applications published"
    );
    Ok(())
}

#[tokio::test]
async fn audio_drain_incomplete_without_bridge_command_cuts_capture() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(super::frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop command absent")?;
    assert_eq!(stop["command"], "audio_stop");
    // The generic index evicted the stop (without this the name came from
    // `pending_native` and the test proved nothing, as a review found): only
    // the capture registry can name it now.
    runtime.helper.pending.clear();

    deliver(
        &mut runtime,
        json!({"v":1, "event":"response","id":stop["id"],"ok":false,
            "error":{"code":"audio_drain_incomplete","message":"El audio no terminó de drenarse"}}),
    )
    .await;

    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    assert!(
        said::mentions(&runtime.state, "audio_drain_incomplete"),
        "the notice must name the helper code: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn bridge_tagged_clip_rejection_names_the_cause_and_keeps_the_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    runtime
        .apply(Event::CallStart {
            id: "synthetic-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert_eq!(
        receiver.recv().await.context("playback stop absent")?["command"],
        "playback_stop"
    );
    let audio_start = receiver.recv().await.context("audio start absent")?;
    runtime
        .native_event(json!({"event":"response","id":audio_start["id"],"ok":true,
            "result":call_readback(&audio_start)}))
        .await?;
    // Production path: speech.rs writes playback_clip straight to the bridge, so
    // the daemon index never saw it; only the bridge tag names it.
    deliver(
        &mut runtime,
        json!({"event":"response","id":"speech-playback-1","_bridge_command":"playback_clip","ok":false,
            "error":{"code":"playback_backpressure","message":"cola de reproducción llena"}}),
    )
    .await;
    tokio::task::yield_now().await;
    assert!(!runtime.state.privacy_stopped());
    assert!(runtime.state.durable.voice.call.is_some());
    assert!(
        said::mentions(&runtime.state, "playback_backpressure"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

fn ready_with_storage(nearly_full: bool) -> HubEvent {
    HubEvent::Frame(json!({
        "event":"ready", "data":{"device_id":"synthetic-device", "storage":{
            "used_bytes": 82_000_000_000_u64, "max_bytes": 100_000_000_000_u64,
            "remaining_bytes": 18_000_000_000_u64, "limit": "quota", "nearly_full": nearly_full
        }}
    }))
}

fn storage_notices(runtime: &Runtime) -> usize {
    runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .filter(|notice| notice.code == NoticeCode::HubStorageNearlyFull)
        .count()
}

#[tokio::test]
async fn a_nearly_full_hub_is_said_once_with_its_numbers_not_at_every_reconnection() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("synthetic-device".into());
    runtime.hub_event(HubEvent::Connected).await?;
    runtime.hub_event(ready_with_storage(true)).await?;
    assert_eq!(runtime.state.connection(), Connection::Online);
    assert_eq!(
        storage_notices(&runtime),
        1,
        "{:?}",
        said::codes(&runtime.state)
    );
    let told = said::told(&runtime);
    for fragment in [
        "\"bytes\":82000000000",
        "\"max_bytes\":100000000000",
        "\"remaining\":18000000000",
        "\"cause\":\"quota\"",
    ] {
        assert!(told.contains(fragment), "{fragment} missing from {told}");
    }
    runtime.hub_event(HubEvent::Disconnected).await?;
    runtime.hub_event(HubEvent::Connected).await?;
    runtime.hub_event(ready_with_storage(true)).await?;
    assert_eq!(storage_notices(&runtime), 1);
    Ok(())
}

#[tokio::test]
async fn the_hub_saying_it_became_nearly_full_reaches_the_person() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("synthetic-device".into());
    runtime.hub_event(HubEvent::Connected).await?;
    runtime.hub_event(ready_with_storage(false)).await?;
    assert_eq!(storage_notices(&runtime), 0);
    let status = |nearly_full: bool| {
        HubEvent::Frame(json!({"event":"storage.status", "data":{
            "used_bytes": 80, "max_bytes": 100, "remaining_bytes": 20,
            "limit": "disk", "nearly_full": nearly_full
        }}))
    };
    runtime.hub_event(status(true)).await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::HubStorageNearlyFull)
    );
    // Room freed, then full again: said again.
    runtime.hub_event(status(false)).await?;
    runtime.hub_event(status(true)).await?;
    assert_eq!(storage_notices(&runtime), 2);
    Ok(())
}

#[tokio::test]
async fn a_storage_the_mac_cannot_read_does_not_cost_the_link() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.settings.device_id = Some("synthetic-device".into());
    runtime.hub_event(HubEvent::Connected).await?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event":"ready", "data":{"device_id":"synthetic-device", "storage":{"used_bytes":"mucho"}}
        })))
        .await?;
    assert_eq!(runtime.state.connection(), Connection::Online);
    assert_eq!(storage_notices(&runtime), 0);
    Ok(())
}
