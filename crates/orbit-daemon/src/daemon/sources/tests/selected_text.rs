use super::*;

const TEXT: &str = "¿Cafe\u{301}? 🪐\r\n\tOriginal \u{202e}exacto";

async fn select_text(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<Value> {
    list(runtime, receiver).await?;
    runtime
        .state
        .views
        .source_inventory
        .as_mut()
        .context("inventory")?
        .selected_text_available = true;
    runtime
        .source_capture_as(
            TOKEN,
            "Nota separada".into(),
            orbit_core::sources::SourceFormat::SelectedText,
        )
        .await?;
    let command = receiver.recv().await.context("text command")?;
    assert_eq!(command["command"], "selected_text");
    Ok(command)
}

fn response(command: &Value) -> Value {
    json!({"event":"response","id":command["id"],"ok":true,
        "result":{"binding":command["window_binding"],"clock_domain":"mach_absolute_time",
        "capture_started_monotonic_ns":2000000,"capture_completed_monotonic_ns":3000000,
        "mime_type":"text/plain; charset=utf-8","source":"native_selected_text","text":TEXT,
        "range":{"location":7,"length":TEXT.encode_utf16().count(),"unit":"utf16_code_units"},
        "ax_binding_quality":"unique_owner_title_bounds",
        "selection_scope":"application_focused_element","sensitivity":"not_certified"}})
}

#[tokio::test]
async fn malformed_text_result_names_event_field_and_type_in_source_refusal() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = select_text(&mut runtime, &mut receiver).await?;
    let mut malformed = response(&command);
    malformed["result"]["text"] = json!(42);
    runtime.native_event(malformed).await?;
    assert!(runtime.state.views.source_receipt.is_none());
    // The notice says the source failed; its detail stays in the source's
    // own view (SIM-21: no sentence in a notice).
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::SourceFailed),
        "{:?}",
        runtime.state.durable.notice
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
        )?,
        json!({"code":"helper_reply_invalid", "field":"text", "expected":{"kind":"wire_type","name":"native_v1/selected_text"}, "received_type":"number", "retry":{"kind":"after_fix"}})
    );
    Ok(())
}

#[tokio::test]
async fn text_is_exact_local_and_single_use_without_image_or_clipboard_fallback() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = select_text(&mut runtime, &mut receiver).await?;
    let source_id = runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("progress")?
        .id
        .clone();
    let event = response(&command);
    runtime.native_event(event.clone()).await?;
    let receipt = runtime
        .state
        .views
        .source_receipt
        .as_ref()
        .context("receipt")?;
    assert_eq!(receipt.intent.id, source_id);
    assert_eq!(receipt.intent.format, SourceFormat::SelectedText);
    assert_eq!(
        receipt.intent.expires_at_ms - receipt.intent.requested_at_ms,
        5000
    );
    assert_eq!(
        receipt.sha256,
        format!("{:x}", Sha256::digest(TEXT.as_bytes()))
    );
    assert_eq!(receipt.bytes, TEXT.len() as u64);
    assert_eq!(receipt.intent.annotation, "Nota separada");
    assert_eq!(receipt.metadata.media.format(), SourceFormat::SelectedText);
    runtime.native_event(event).await?;
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
            .await
            .is_err()
    );
    assert!(
        runtime
            .source_capture_as(TOKEN, String::new(), SourceFormat::SelectedText)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    assert!(!serde_json::to_string(&runtime.state)?.contains("Cafe"));
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    assert_eq!(runtime.fixture_store.diagnostics()?.pending_jobs, 0);
    Ok(())
}

#[tokio::test]
async fn unsupported_text_does_not_consume_inventory_or_request_any_content() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    list(&mut runtime, &mut receiver).await?;
    assert!(
        runtime
            .source_capture_as(
                TOKEN,
                String::new(),
                orbit_core::sources::SourceFormat::SelectedText,
            )
            .await
            .is_err()
    );
    assert!(runtime.state.views.source_inventory.is_some());
    assert!(receiver.try_recv().is_err());
    // A separate explicit image command remains possible, never implicit fallback.
    runtime
        .source_capture_as(TOKEN, String::new(), orbit_core::sources::SourceFormat::Png)
        .await?;
    assert_eq!(
        receiver.recv().await.context("image")?["command"],
        "screenshot"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_text_metadata_or_format_is_terminal_without_retry_buffer() -> Result<()> {
    for tamper in [
        "binding",
        "range",
        "unit",
        "nul",
        "oversize",
        "sensitivity",
        "scope",
        "quality",
        "mime",
        "duration",
        "image",
        "extra",
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let command = select_text(&mut runtime, &mut receiver).await?;
        let mut event = response(&command);
        match tamper {
            "binding" => event["result"]["binding"]["application_pid"] = 789.into(),
            "range" => event["result"]["range"]["length"] = 1.into(),
            "unit" => event["result"]["range"]["unit"] = "characters".into(),
            "nul" => event["result"]["text"] = "\0".into(),
            "oversize" => event["result"]["text"] = "x".repeat(16385).into(),
            "sensitivity" => event["result"]["sensitivity"] = "safe".into(),
            "scope" => event["result"]["selection_scope"] = "system_focused_element".into(),
            "quality" => event["result"]["ax_binding_quality"] = "pid_only".into(),
            "mime" => event["result"]["mime_type"] = "text/html".into(),
            "duration" => {
                event["result"]["capture_completed_monotonic_ns"] = 5_002_000_000u64.into()
            }
            "image" => event = screenshot(&command)?,
            _ => event["result"]["whole_document"] = TEXT.into(),
        }
        runtime.native_event(event).await?;
        assert!(runtime.state.views.source_receipt.is_none(), "{tamper}");
        assert_eq!(
            runtime
                .state
                .views
                .source_request
                .as_ref()
                .context("progress")?
                .stage,
            SourceStage::Failed,
            "{tamper}"
        );
        assert!(receiver.try_recv().is_err());
        assert!(!runtime.state.privacy_stopped());
    }
    // The reverse substitution is also forbidden.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = capture(&mut runtime, &mut receiver).await?;
    runtime.native_event(response(&command)).await?;
    assert!(runtime.state.views.source_receipt.is_none());
    Ok(())
}

#[tokio::test]
async fn refused_or_obsolete_text_never_changes_format_or_stops_unrelated_audio() -> Result<()> {
    for cause in ["refused", "privacy", "generation", "session", "deadline"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let command = select_text(&mut runtime, &mut receiver).await?;
        let mut event = response(&command);
        match cause {
            "refused" => {
                event = json!({"event":"response","id":command["id"],"ok":false,
                "error":{"code":"selected_text_unavailable","message":"Do not echo provider text"}})
            }
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
                let fixture_source: Option<String> = Some("replacement".into());
                let fixture_value = serde_json::json!(fixture_source);
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "session_id",
                    fixture_value,
                );
            }
            _ => {
                runtime
                    .features
                    .sources
                    .operation
                    .as_mut()
                    .context("operation")?
                    .deadline = TEXT_TIMEOUT.starting_at(Instant::now() - Duration::from_secs(6))
            }
        }
        runtime.native_event(event.clone()).await?;
        runtime.native_event(event).await?;
        assert!(runtime.state.views.source_receipt.is_none(), "{cause}");
        assert!(receiver.try_recv().is_err());
        if cause != "privacy" {
            assert!(!runtime.state.privacy_stopped());
        }
    }
    Ok(())
}
