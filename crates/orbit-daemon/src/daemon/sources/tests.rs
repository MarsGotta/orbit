use super::*;
use crate::daemon::{HubEvent, audio::tests::fixture};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::protocol::Command;
use tokio::sync::mpsc;

const TOKEN: &str = "00000000-0000-4000-8000-000000000003";
mod metadata_budget;
mod selected_text;

fn inventory() -> Value {
    json!({"native_instance_id":"00000000-0000-4000-8000-000000000001",
        "privacy_generation":2,"inventory_id":"00000000-0000-4000-8000-000000000002",
        "clock_domain":"mach_absolute_time","observed_monotonic_ns":1000000u64,
        "expires_monotonic_ns":30001000000u64,"selected_text_available":false,"omitted_count":0,
        "windows":[{"binding":{"native_instance_id":"00000000-0000-4000-8000-000000000001",
            "privacy_generation":2,"inventory_id":"00000000-0000-4000-8000-000000000002",
            "selection_token":TOKEN,"window_id":123,"application_pid":456,
            "application_instance_id":"00000000-0000-4000-8000-000000000004",
            "binding_quality":"observed_owner_and_window","descriptor_hash":"a".repeat(64)},
            "window_id":123,"application_pid":456,"application_name":"Editor",
            "bundle_id":"example.editor","title":"Nota sintética","title_truncated":false,"unavailable_reason":null}]})
}

fn image() -> Result<Vec<u8>> {
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

fn screenshot(command: &Value) -> Result<Value> {
    Ok(json!({"event":"response","id":command["id"],"ok":true,
        "result":{"binding":command["window_binding"],"clock_domain":"mach_absolute_time",
        "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
        "mime_type":"image/png","width":2,"height":1,"data_base64":STANDARD.encode(image()?),"source":"native_window"}}))
}

async fn list(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<()> {
    runtime.source_windows().await?;
    let request = receiver.recv().await.context("list command")?;
    assert_eq!(request["command"], "screen_sources");
    runtime
        .native_event(json!({"event":"response","id":request["id"],"ok":true,"result":inventory()}))
        .await?;
    Ok(())
}

async fn capture(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<Value> {
    list(runtime, receiver).await?;
    runtime
        .source_capture_as(
            TOKEN,
            "Sólo fuente local.".into(),
            orbit_core::sources::SourceFormat::Png,
        )
        .await?;
    let command = receiver.recv().await.context("capture command")?;
    assert_eq!(command["command"], "screenshot");
    Ok(command)
}

#[tokio::test]
async fn exact_capture_is_local_immutable_and_never_an_audio_upload() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = capture(&mut runtime, &mut receiver).await?;
    let source_id = runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("source progress")?
        .id
        .clone();
    let response = screenshot(&command)?;
    runtime.native_event(response.clone()).await?;
    let receipt = runtime
        .state
        .views
        .source_receipt
        .as_ref()
        .context("receipt")?;
    assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(image()?)));
    assert_eq!(receipt.bytes, image()?.len() as u64);
    assert_eq!(receipt.intent.id, source_id);
    assert_eq!(
        runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("progress")?
            .stage,
        SourceStage::Validated
    );
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    assert_eq!(runtime.fixture_store.diagnostics()?.pending_jobs, 0);
    assert!(runtime.link.outbound.is_none());
    runtime.native_event(response).await?;
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    assert!(!serde_json::to_string(&runtime.state)?.contains("data_base64"));
    Ok(())
}

#[tokio::test]
async fn native_refusal_and_late_duplicates_are_local_not_global_audio_failure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = capture(&mut runtime, &mut receiver).await?;
    let refused = json!({"event":"response","id":command["id"],"ok":false,
        "error":{"code":"screen_permission_required","message":"untrusted text"}});
    runtime.native_event(refused.clone()).await?;
    runtime.native_event(refused).await?;
    assert!(!runtime.state.privacy_stopped());
    assert!(runtime.state.views.source_receipt.is_none());
    assert!(receiver.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn substituted_binding_or_damaged_png_is_not_a_pending_disk_failure() -> Result<()> {
    for tamper in ["binding", "png", "dimensions", "timestamp"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let command = capture(&mut runtime, &mut receiver).await?;
        let mut response = screenshot(&command)?;
        match tamper {
            "binding" => response["result"]["binding"]["application_pid"] = 789.into(),
            "png" => response["result"]["data_base64"] = STANDARD.encode(b"not a PNG").into(),
            "dimensions" => response["result"]["width"] = u32::MAX.into(),
            _ => response["result"]["capture_completed_monotonic_ns"] = u64::MAX.into(),
        }
        runtime.native_event(response).await?;
        assert!(runtime.state.views.source_receipt.is_none(), "{tamper}");
        assert_eq!(
            runtime
                .state
                .views
                .source_request
                .as_ref()
                .context("progress")?
                .stage,
            SourceStage::Failed
        );
        assert!(!runtime.state.privacy_stopped());
    }
    Ok(())
}

#[tokio::test]
async fn privacy_generation_session_and_deadline_retire_in_flight_capture() -> Result<()> {
    for reason in ["privacy", "generation", "session", "deadline"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let command = capture(&mut runtime, &mut receiver).await?;
        let source_id = runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("progress")?
            .id
            .clone();
        match reason {
            "privacy" => {
                {
                    let fixture_value = serde_json::json!(true);
                    crate::daemon::authority_fixture::set(
                        &mut runtime.state,
                        "privacy_stopped",
                        fixture_value,
                    );
                }
                runtime.cancel_sources().await?;
            }
            "generation" => {
                let fixture_value = serde_json::json!(runtime.state.generation() + (1));
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "generation",
                    fixture_value,
                );
            }
            "session" => {
                let fixture_source: Option<String> = Some("different-session".into());
                let fixture_value = serde_json::json!(fixture_source);
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "session_id",
                    fixture_value,
                );
            }
            _ => {
                runtime
                    .sources
                    .operation
                    .as_mut()
                    .context("operation")?
                    .deadline = NATIVE_TIMEOUT.starting_at(Instant::now() - Duration::from_secs(11))
            }
        }
        runtime.native_event(screenshot(&command)?).await?;
        assert!(runtime.state.views.source_receipt.is_none(), "{reason}");
        assert_eq!(
            runtime
                .state
                .views
                .source_request
                .as_ref()
                .context("progress")?
                .stage,
            SourceStage::Unknown
        );
        assert_eq!(
            runtime
                .sources
                .last_failure
                .as_ref()
                .context("failure")?
                .operation_id,
            source_id
        );
        assert!(receiver.try_recv().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn stale_source_reply_records_its_actual_cause() -> Result<()> {
    for (reason, expected_code, _expected_detail) in [
        ("generation", "generation_changed", "generation_changed"),
        ("session", "conversation_changed", "conversación"),
        ("deadline", "native_timeout", "plazo"),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let command = capture(&mut runtime, &mut receiver).await?;
        let source_id = runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("source progress")?
            .id
            .clone();
        let started_generation = runtime.state.generation();
        match reason {
            "generation" => {
                let fixture_value = serde_json::json!(runtime.state.generation() + (1));
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "generation",
                    fixture_value,
                );
            }
            "session" => {
                let fixture_source: Option<String> = Some("different-session".into());
                let fixture_value = serde_json::json!(fixture_source);
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "session_id",
                    fixture_value,
                );
            }
            "deadline" => {
                runtime
                    .sources
                    .operation
                    .as_mut()
                    .context("operation")?
                    .deadline =
                    NATIVE_TIMEOUT.starting_at(Instant::now() - Duration::from_secs(11));
            }
            _ => unreachable!(),
        }
        runtime.native_event(screenshot(&command)?).await?;
        let stored = runtime.sources.last_failure.as_ref().context("failure")?;
        assert_eq!(stored.operation_id, source_id);
        let refusal = serde_json::to_value(&stored.refusal)?;
        match reason {
            "generation" => {
                assert_eq!(refusal["code"], "read_interrupted");
                assert_eq!(refusal["cause"]["kind"], expected_code);
                assert_eq!(refusal["cause"]["started_generation"], started_generation);
                assert_eq!(
                    refusal["cause"]["current_generation"],
                    runtime.state.generation()
                );
            }
            "session" => {
                assert_eq!(refusal["code"], "read_interrupted");
                assert_eq!(refusal["cause"]["kind"], expected_code);
            }
            "deadline" => {
                assert_eq!(refusal["code"], "timeout");
                assert_eq!(refusal["deadline_ms"], 10_000);
                assert!(
                    refusal["waited_ms"]
                        .as_u64()
                        .is_some_and(|waited| waited >= 11_000)
                );
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[tokio::test]
async fn conversation_and_helper_restart_close_their_own_source_cause() -> Result<()> {
    for (cause, expected) in [
        ("conversation", "conversation_changed"),
        ("helper", "helper_restarted"),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let _command = capture(&mut runtime, &mut receiver).await?;
        let source_id = runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("progress")?
            .id
            .clone();
        if cause == "conversation" {
            runtime
                .command(Command::NewSession { agent_id: None })
                .await?;
        } else {
            runtime.native_event(json!({"event":"ready"})).await?;
        }
        let stored = runtime.sources.last_failure.as_ref().context("failure")?;
        assert_eq!(stored.operation_id, source_id);
        let refusal = serde_json::to_value(&stored.refusal)?;
        assert_eq!(refusal["code"], "read_interrupted");
        assert_eq!(refusal["cause"]["kind"], expected);
        assert_eq!(
            runtime
                .state
                .views
                .source_request
                .as_ref()
                .context("progress")?
                .stage,
            SourceStage::Unknown
        );
        assert_eq!(
            serde_json::from_str::<Value>(
                &runtime
                    .state
                    .views
                    .source_request
                    .as_ref()
                    .context("progress")?
                    .detail
            )?["cause"]["kind"],
            if cause == "conversation" {
                "conversation_changed"
            } else {
                "helper_restarted"
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn privacy_stop_names_the_source_cause_before_a_late_reply() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = capture(&mut runtime, &mut receiver).await?;
    let source_id = runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("progress")?
        .id
        .clone();
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.cancel_sources().await?;
    runtime.native_event(screenshot(&command)?).await?;
    let stored = runtime.sources.last_failure.as_ref().context("failure")?;
    assert_eq!(stored.operation_id, source_id);
    assert_eq!(
        stored.refusal,
        orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None }
    );
    let detail = &runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("progress")?
        .detail;
    assert_eq!(
        serde_json::from_str::<Value>(detail)?["code"],
        "privacy_stopped"
    );
    Ok(())
}

#[tokio::test]
async fn full_queue_fails_before_capture_and_old_inventory_never_rearms() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    list(&mut runtime, &mut receiver).await?;
    while runtime
        .helper
        .sender
        .try_send(json!({"command":"fixture"}))
        .is_ok()
    {}
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
            .await
            .is_err()
    );
    assert!(runtime.state.views.source_inventory.is_none());
    let progress = runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("progress")?;
    assert_eq!(progress.stage, SourceStage::Failed);
    while let Ok(command) = receiver.try_recv() {
        assert_ne!(command["command"], "screenshot");
    }
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn expiring_an_inventory_retires_the_available_ui_state() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    list(&mut runtime, &mut receiver).await?;
    runtime.sources.inventory_deadline = Some(Instant::now() - Duration::from_secs(1));
    runtime.check_source_timeout().await?;
    assert!(runtime.state.views.source_inventory.is_none());
    assert_eq!(
        runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("progress")?
            .stage,
        SourceStage::Cancelled
    );
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn a_helper_timeout_releases_the_remote_request_and_answers_the_hub() -> Result<()> {
    // The enumeration a remote request is waiting on can also end
    // by timeout. That ending must release the bridge's single slot and
    // answer the hub, or every later remote request is refused as busy until
    // the daemon restarts. The deadline is back-dated here because the tick
    // that fires it cannot be fast-forwarded otherwise.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let (hub, mut hub_rx) = mpsc::channel(16);
    runtime.link.outbound.install(hub);
    // What lets the bridge list windows is Screen Recording.
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request",
            "data": {"request_id": "bridge-1", "capability": "screen_sources"}
        })))
        .await?;
    let sent = receiver.recv().await.context("list command")?;
    assert_eq!(sent["command"], "screen_sources");

    runtime
        .sources
        .operation
        .as_mut()
        .context("operation")?
        .deadline = NATIVE_TIMEOUT.starting_at(Instant::now() - Duration::from_secs(11));
    runtime.check_source_timeout().await?;

    assert!(
        runtime.assistant.bridge_requests.is_empty(),
        "the expiry left the bridge busy: {:?}",
        runtime.assistant.bridge_requests
    );
    let reply = hub_rx.recv().await.context("the request got no answer")?;
    assert_eq!(reply["method"], "device.reply");
    assert_eq!(reply["params"]["request_id"], "bridge-1");
    assert_eq!(reply["params"]["refusal"]["code"], "timeout", "{reply}");
    // With the slot free again the next remote request is served, not refused.
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "device.request",
            "data": {"request_id": "bridge-2", "capability": "screen_sources"}
        })))
        .await?;
    let again = receiver
        .recv()
        .await
        .context("the second request was not served")?;
    assert_eq!(again["command"], "screen_sources");
    let refused = hub_rx.try_recv();
    assert!(
        refused.is_err(),
        "the second request was refused: {refused:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_remote_read_refusal_reaches_deimos_without_a_user_failure_notice() -> Result<()> {
    for (capability, code, capture) in [
        ("screen_sources", "screen_permission_required", false),
        ("screenshot", "window_identity_changed", true),
        ("selected_text", "selected_text_identity_changed", true),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            json!(orbit_core::PermissionState::Granted),
        );
        let (hub, mut frames) = mpsc::channel(16);
        runtime.link.outbound.install(hub);
        runtime
            .hub_event(HubEvent::Frame(json!({"event":"device.request",
            "data":{"request_id":"remote-read","capability":capability}})))
            .await?;
        let mut command = receiver.recv().await.context("screen_sources")?;
        assert_eq!(command["command"], "screen_sources");
        if capture {
            let mut listing = inventory();
            listing["selected_text_available"] = json!(true);
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":listing}))
                .await?;
            command = receiver.recv().await.context("capture")?;
            assert_eq!(command["command"], capability);
        }
        runtime
            .native_event(json!({"event":"response","id":command["id"],"ok":false,
            "error":{"code":code,"message":"PRIVATE WINDOW TITLE"}}))
            .await?;
        let reply = frames.recv().await.context("device.reply")?;
        assert_eq!(reply["method"], "device.reply");
        assert_eq!(reply["params"]["request_id"], "remote-read");
        let expected = match code {
            "screen_permission_required" => {
                json!({"code":"permission_denied", "permission":"screen_recording", "retry":{"kind":"after_fix"}})
            }
            "window_identity_changed" => {
                json!({"code":"helper_refused", "helper_code":"window_identity_changed", "retry":{"kind":"after_fix"}})
            }
            "selected_text_identity_changed" => {
                json!({"code":"helper_refused", "helper_code":"selected_text_identity_changed", "retry":{"kind":"after_fix"}})
            }
            other => anyhow::bail!("unexpected helper refusal fixture: {other}"),
        };
        assert_eq!(reply["params"]["refusal"], expected);
        assert!(!reply.to_string().contains("PRIVATE WINDOW TITLE"));
        assert!(runtime.assistant.bridge_requests.is_empty());
        assert!(receiver.try_recv().is_err(), "the read must not be retried");
        let failures: Vec<_> = runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .filter(|notice| notice.code.is_failure())
            .collect();
        assert!(
            failures.is_empty(),
            "Alina saw a failure for Deimos's request: {failures:?}"
        );
        let log =
            std::fs::read_to_string(runtime.report_log.as_ref().context("report log")?.path())?;
        for line in log.lines() {
            let mut value: Value = serde_json::from_str(line)?;
            value.as_object_mut().context("report object")?.remove("ts");
            let report: orbit_core::notice::Report = serde_json::from_value(value)?;
            assert!(
                !report.is_failure(),
                "remote refusal was reported as an Orbit failure: {line}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn source_reads_do_not_exhaust_the_retired_two_hundred_intent_quota() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    for reading in 1..=201 {
        let command = capture(&mut runtime, &mut receiver)
            .await
            .with_context(|| format!("reading {reading} of 201"))?;
        runtime.native_event(screenshot(&command)?).await?;
        assert!(
            runtime.state.views.source_receipt.is_some(),
            "reading {reading}"
        );
    }
    assert!(receiver.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn source_receipt_and_inventory_never_enter_the_durable_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = capture(&mut runtime, &mut receiver).await?;
    runtime.native_event(screenshot(&command)?).await?;
    let durable = serde_json::to_value(runtime.state.durable())?;
    for field in ["source_inventory", "source_request", "source_receipt"] {
        assert!(durable.get(field).is_none(), "durable field {field}");
    }
    let restored: orbit_core::State = runtime.state.durable().into();
    assert!(restored.views.source_receipt.is_none());
    assert!(restored.views.source_request.is_none());
    assert!(restored.views.source_inventory.is_none());
    assert!(runtime.state.views.source_receipt.is_some());
    Ok(())
}

#[tokio::test]
async fn reads_complete_even_when_snapshot_writes_are_refused() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch("CREATE TRIGGER fixture_no_snapshot BEFORE INSERT ON settings WHEN NEW.key='snapshot' BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;
    CREATE TRIGGER fixture_no_snapshot_update BEFORE UPDATE ON settings WHEN NEW.key='snapshot' BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;")?;
    let command = capture(&mut runtime, &mut receiver).await?;
    runtime.native_event(screenshot(&command)?).await?;
    assert!(runtime.state.views.source_receipt.is_some());
    assert!(receiver.try_recv().is_err());
    Ok(())
}
