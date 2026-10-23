use super::*;
use crate::daemon::turn::said;
use orbit_core::notice::NoticeCode;

/// No current notice of `code`.
fn not_current(runtime: &Runtime, code: NoticeCode) -> bool {
    runtime.state.notice_code() != Some(code)
}

fn next_capture_command(receiver: &mut mpsc::Receiver<Value>) -> Result<Option<Value>> {
    loop {
        match receiver.try_recv() {
            Ok(command)
                if matches!(
                    command["command"].as_str(),
                    Some("playback_stop" | "status")
                ) => {}
            Ok(command) => return Ok(Some(command)),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                anyhow::bail!("synthetic helper command channel closed")
            }
        }
    }
}

async fn retired_system_start(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<Value> {
    runtime
        .apply(Event::StartMeeting {
            id: "old-recording".into(),
            title: "Old synthetic recording".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let microphone = receive_helper_command(receiver, "audio_start").await?;
    let remote = receive_helper_command(receiver, "audio_start").await?;
    assert_eq!(microphone["source"], "mic_local");
    assert_eq!(remote["source"], "remote_app");
    runtime
        // A cut that ends the capture (the Mac going to sleep): the empty
        // recording closes at once and the call that follows is admitted.
        .cut_capture(
            orbit_core::privacy_stop::StopCause::HelperCaptureStop,
            "session_inactive",
            "Corte sintético anterior",
        )
        .await;
    let stop = receiver.try_recv().context("capture_stop absent")?;
    assert_eq!(stop["command"], "capture_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(runtime).await?;
    let remote_id = remote["id"].as_str().context("old remote id absent")?;
    assert!(
        !runtime
            .capture
            .pending_starts
            .iter()
            .any(|id| id == remote_id)
    );
    Ok(remote)
}

#[tokio::test]
async fn retired_success_stops_only_its_stream_and_waits_for_receipt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    admit_call_after_empty_capture(&mut runtime, "new-call").await?;
    let call_start = loop {
        let command = receiver.try_recv().context("new call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    let call_stream = call_start["stream_id"]
        .as_str()
        .context("call stream absent")?;
    let old_stream = remote["stream_id"].as_str().context("old stream absent")?;
    assert_ne!(old_stream, call_stream);

    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":old_stream,"source":"remote_app"}}))
        .await?;
    let retired_stop = receiver
        .try_recv()
        .context("retired stream audio_stop absent")?;
    assert_eq!(retired_stop["command"], "audio_stop");
    assert_eq!(retired_stop["stream_id"], old_stream);
    let stop_id = retired_stop["id"]
        .as_str()
        .context("retired stop id absent")?;
    assert_eq!(
        runtime
            .capture
            .retired_stops
            .get(stop_id)
            .map(|stop| (stop.stream.as_str(), stop.sent)),
        Some((old_stream, true))
    );
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(call_stream));
    assert_eq!(
        runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("new-call")
    );
    runtime
        .native_event(json!({"event":"response","id":retired_stop["id"],"ok":true,
            "result":{"stream_id":old_stream,"drain_complete":true}}))
        .await?;
    assert!(!runtime.capture.retired_stops.contains_key(stop_id));

    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(call_stream));
    assert_eq!(
        runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("new-call")
    );
    assert!(
        receiver.try_recv().is_err(),
        "receipt triggered an unrelated helper order"
    );
    Ok(())
}

async fn retired_integrity_code_does_not_cut_a_new_recording(code: &str) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    let old_remote = retired_system_start(&mut runtime, &mut receiver).await?;
    runtime
        .apply(Event::StartMeeting {
            id: "next-recording".into(),
            title: "New synthetic recording".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let new_microphone = loop {
        let command = receiver.try_recv().context("new microphone start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    let new_remote = receiver.try_recv().context("new remote start absent")?;
    assert_eq!(new_remote["command"], "audio_start");
    let new_mic_stream = new_microphone["stream_id"]
        .as_str()
        .context("new mic absent")?;
    let new_remote_stream = new_remote["stream_id"]
        .as_str()
        .context("new remote absent")?;
    runtime
        .native_event(json!({"event":"response","id":old_remote["id"],"ok":false,
                "error":{"code":code,"message":"Fallo sintético anterior"}}))
        .await?;
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        Some(new_mic_stream),
        "{code}"
    );
    assert_eq!(
        runtime.capture.remote_stream.as_deref(),
        Some(new_remote_stream),
        "{code}"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Starting),
        "{code}"
    );
    assert!(runtime.capture.interruption.is_none(), "{code}");
    assert!(receiver.try_recv().is_err(), "{code} cut the new recording");

    Ok(())
}

#[tokio::test]
async fn retired_integrity_refusals_do_not_cut_a_new_recording() -> Result<()> {
    for code in [
        "future_audio_integrity_failure",
        "system_audio_clock_unavailable",
        "application_binding_changed",
    ] {
        retired_integrity_code_does_not_cut_a_new_recording(code).await?;
    }
    Ok(())
}

#[tokio::test]
async fn voice_first_then_system_recording_refused_keeps_voice_source() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.apply(Event::VoiceStart).await?;
    let microphone = loop {
        let command = receiver.try_recv().context("voice audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    runtime
        .native_event(json!({"event":"response","id":microphone["id"],"ok":true,
            "result":{"stream_id":microphone["stream_id"]}}))
        .await?;
    let stream = microphone["stream_id"]
        .as_str()
        .context("voice stream absent")?;
    runtime.native_event(frame(stream, 800)).await?;
    let pcm_before = runtime
        .capture
        .buffer
        .voice
        .as_ref()
        .context("voice buffer absent")?
        .pcm
        .clone();
    assert_eq!(pcm_before.len(), 1_600);
    runtime
        .apply(Event::StartMeeting {
            id: "system-after-voice".into(),
            title: "Synthetic".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    // The meeting takes the note's microphone, then opens the system's audio.
    let taken = receive_helper_command(&mut receiver, "audio_purpose").await?;
    assert_eq!(taken["stream_id"], microphone["stream_id"]);
    assert_eq!(taken["purpose"], "meeting");
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    assert_eq!(remote["source"], "remote_app");
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"screen_permission_required", "message":"Pantalla denegada"}}))
        .await?;
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("voice PCM discarded")?
            .pcm,
        pcm_before
    );
    let stop = receiver.try_recv().context("capture_stop absent")?;
    assert_eq!(stop["command"], "capture_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let manifest = runtime
        .fixture_store
        .list_jobs(20)?
        .into_iter()
        .find(|job| {
            job.kind == "recording_manifest" && job.payload["recording_kind"] == "voice_turn"
        })
        .context("cancelled voice source manifest absent")?;
    assert_eq!(manifest.payload["voice_state"], "cancelled");
    assert_eq!(manifest.payload["upload_allowed"], false);
    let recording_id = manifest.payload["recording_id"]
        .as_str()
        .context("voice recording id absent")?;
    let chunks = runtime.fixture_store.list_chunks(recording_id, None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, 800);
    let wav = runtime.fixture_store.read_chunk(&chunks[0].metadata.id)?;
    assert_eq!(wav.len(), pcm_before.len() + 44);
    assert_eq!(&wav[44..], pcm_before.as_slice());
    Ok(())
}
async fn retired_stop_with_call(
    receipt: impl Fn(&str, &str) -> Value,
) -> Result<(
    Runtime,
    mpsc::Receiver<Value>,
    tempfile::TempDir,
    String,
    String,
)> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    admit_call_after_empty_capture(&mut runtime, "new-call").await?;
    let call_start = loop {
        let command = receiver.try_recv().context("new call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    let call_stream = call_start["stream_id"]
        .as_str()
        .context("call stream")?
        .to_owned();
    let old_stream = remote["stream_id"]
        .as_str()
        .context("old stream")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":old_stream,"source":"remote_app"}}))
        .await?;
    let retired_stop = receiver.try_recv().context("retired stop absent")?;
    assert_eq!(retired_stop["command"], "audio_stop");
    let stop_id = retired_stop["id"].as_str().context("id")?.to_owned();
    let event = receipt(&stop_id, &call_stream);
    runtime.native_event(event).await?;
    assert!(!runtime.capture.retired_stops.contains_key(&stop_id));
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        Some(call_stream.as_str())
    );
    assert_eq!(
        runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("new-call")
    );
    let call_start_id = call_start["id"].as_str().context("call start id absent")?;
    assert!(
        runtime
            .capture
            .pending_starts
            .iter()
            .any(|id| id == call_start_id)
    );
    assert!(
        receiver.try_recv().is_err(),
        "receipt triggered a helper order"
    );
    Ok((runtime, receiver, directory, call_stream, old_stream))
}

#[tokio::test]
async fn retired_stop_failure_leaves_call_and_names_code() -> Result<()> {
    let (runtime, _rx, _dir, _call, _old) = retired_stop_with_call(|id, _| {
        json!({"event":"response","id":id,"ok":false,
            "error":{"code":"audio_drain_incomplete","message":"x"}})
    })
    .await?;
    assert!(
        said::mentions(&runtime.state, "audio_drain_incomplete"),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(runtime.capture.interruption.is_none());
    Ok(())
}

#[tokio::test]
async fn retired_stop_without_receipt_blocks_nothing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    let old_stream = remote["stream_id"]
        .as_str()
        .context("old stream")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":old_stream,"source":"remote_app"}}))
        .await?;
    let retired_stop = loop {
        let command = receiver.try_recv().context("retired stop absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            _ => break command,
        }
    };
    assert_eq!(retired_stop["command"], "audio_stop");
    assert!(!runtime.audio_is_draining());
    runtime
        .apply(Event::StartMeeting {
            id: "after-unanswered".into(),
            title: "Synthetic".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let mut starts = Vec::new();
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "audio_start" {
            starts.push(command);
        }
    }
    assert_eq!(
        starts.len(),
        2,
        "new recording blocked by an unanswered retired stop"
    );
    // Its mic ACK proceeds normally.
    for start in &starts {
        runtime
            .native_event(json!({"event":"response","id":start["id"],"ok":true,
                "result":{"stream_id":start["stream_id"],"source":start["source"]}}))
            .await?;
    }
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Recording)
    );
    assert!(
        receiver.try_recv().is_err(),
        "live ACK was treated as retired"
    );
    Ok(())
}

#[tokio::test]
async fn live_system_clock_failure_still_contains() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "live".into(),
            title: "Synthetic".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let _microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"system_audio_clock_unavailable","message":"x"}}))
        .await?;
    assert!(
        said::mentions(&runtime.state, "system_audio_clock_unavailable"),
        "{:?}",
        runtime.state.durable.notice
    );
    let stop = receiver
        .try_recv()
        .context("capture_stop absent: live integrity not contained")?;
    assert_eq!(stop["command"], "capture_stop");
    Ok(())
}

#[tokio::test]
async fn privacy_stop_during_starting_names_no_other_track() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "privacy-during-start".into(),
            title: "Synthetic".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    let mut stop = None;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "privacy_stop" {
            stop = Some(command);
        }
    }
    let stop = stop.context("privacy_stop absent")?;
    let stopped_after_stop = said::has(&runtime.state, NoticeCode::PrivacyStopped);
    runtime
        .native_event(json!({"event":"response","id":microphone["id"],"ok":false,
            "error":{"code":"operation_cancelled_by_stop","message":"x"}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"privacy_stopped","message":"x"}}))
        .await?;
    let stopped_before_receipt = said::has(&runtime.state, NoticeCode::PrivacyStopped);
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true,"privacy_stopped":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(stopped_after_stop);
    assert!(stopped_before_receipt);
    assert!(
        !said::has(&runtime.state, NoticeCode::OtherTrackStopped),
        "{:?}",
        runtime.state.durable.notice_log
    );
    Ok(())
}

#[tokio::test]
async fn retired_ack_during_full_stop_sends_nothing_more() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "ack-during-stop".into(),
            title: "Synthetic".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":microphone["id"],"ok":false,
            "error":{"code":"microphone_permission_required","message":"x"}}))
        .await?;
    let stop = receiver.try_recv().context("capture_stop absent")?;
    assert_eq!(stop["command"], "capture_stop");
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":remote["stream_id"],"source":"remote_app"}}))
        .await?;
    assert!(
        receiver.try_recv().is_err(),
        "retired ACK during the full stop sent another order"
    );
    Ok(())
}

#[tokio::test]
async fn call_start_after_stop_receipt_does_not_touch_next_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
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
            id: "older-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    let start = loop {
        let command = receiver
            .try_recv()
            .context("older call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("older call audio_stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stop["stream_id"],"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    admit_call_after_empty_capture(&mut runtime, "newer-call").await?;
    let newer = loop {
        let command = receiver
            .try_recv()
            .context("newer call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    // The generic command-name index is diagnostic and may evict this id.
    runtime.pending_native.clear();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":start["stream_id"],"processing":"voice_processing",
                "processing_enabled":true,"processing_bypassed":false,
                "agc_enabled":false,"playback_reference":"shared_engine",
                "pipeline_id":"older-pipeline"}}))
        .await?;
    let late_stop = next_capture_command(&mut receiver)?.context("older stream stop absent")?;
    assert_eq!(late_stop["command"], "audio_stop");
    assert_eq!(late_stop["stream_id"], start["stream_id"]);
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        newer["stream_id"].as_str()
    );
    assert_eq!(
        runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("newer-call")
    );
    Ok(())
}

#[tokio::test]
async fn retired_call_start_registry_refuses_a_sixty_fifth_unknown_start() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    for index in 0..64 {
        runtime.capture.retired_call_starts.insert(
            format!("retired-call-request-{index}"),
            format!("retired-call-stream-{index}"),
        );
    }
    let failure = match runtime
        .apply(Event::CallStart {
            id: "over-capacity".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await
    {
        Ok(()) => anyhow::bail!("a 65th unanswered call start was sent"),
        Err(failure) => failure,
    };
    assert!(
        failure
            .to_string()
            .contains("retired_call_start_capacity_exhausted")
    );
    assert!(failure.to_string().contains("64"));
    while let Some(command) = next_capture_command(&mut receiver)? {
        assert_ne!(command["command"], "audio_start");
    }
    Ok(())
}

#[tokio::test]
async fn helper_restart_retires_old_stop_and_start_debts() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .capture
        .retired_call_starts
        .insert("old-call-start".into(), "old-call-stream".into());
    runtime.capture.retired_stops.insert(
        "old-stop".into(),
        crate::daemon::audio::RetiredStreamStop {
            stream: "old-stream".into(),
            sent: true,
            may_finish_call: true,
        },
    );
    runtime.native_event(json!({"event":"ready"})).await?;
    assert!(runtime.capture.retired_call_starts.is_empty());
    assert!(runtime.capture.retired_stops.is_empty());

    Ok(())
}

#[tokio::test]
async fn helper_loss_retires_old_call_start_debts() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .capture
        .retired_call_starts
        .insert("old-call-start".into(), "old-call-stream".into());
    {
        let fixture_value = serde_json::json!(false);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.retry_retired_stream_stops().await;
    assert!(runtime.capture.retired_call_starts.is_empty());

    Ok(())
}

async fn closing_call_with_unanswered_start()
-> Result<(Runtime, mpsc::Receiver<Value>, tempfile::TempDir, Value)> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
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
            id: "unanswered-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    let start = next_capture_command(&mut receiver)?.context("call audio_start absent")?;
    assert_eq!(start["command"], "audio_start");
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first audio_stop absent")?;
    assert_eq!(first_stop["command"], "audio_stop");
    runtime
        .native_event(json!({"event":"response","id":first_stop["id"],"ok":false,
            "error":{"code":"stream_not_found","message":"Synthetic stop overtook start"}}))
        .await?;
    Ok((runtime, receiver, directory, start))
}

#[tokio::test]
async fn unreadable_late_call_start_stops_its_commanded_stream() -> Result<()> {
    for result in [None, Some(json!({"source":"mic_local"}))] {
        let (mut runtime, mut receiver, _directory, start) =
            closing_call_with_unanswered_start().await?;
        let mut event = json!({"event":"response","id":start["id"],"ok":true});
        if let Some(result) = result {
            event["result"] = result;
        }
        runtime.native_event(event).await?;
        let late_stop = next_capture_command(&mut receiver)?.context("late audio_stop absent")?;
        assert_eq!(late_stop["command"], "audio_stop");
        assert_eq!(late_stop["stream_id"], start["stream_id"]);
        runtime
            .native_event(json!({"event":"response","id":late_stop["id"],"ok":true,
                "result":{"stream_id":start["stream_id"],"drain_complete":true}}))
            .await?;
        assert!(runtime.capture.mic_stream.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn late_call_start_naming_another_stream_cuts_uncertain_capture() -> Result<()> {
    let (mut runtime, mut receiver, _directory, start) =
        closing_call_with_unanswered_start().await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":"different-stream","source":"mic_local"}}))
        .await?;
    let mut capture_stop = None;
    while let Some(command) = next_capture_command(&mut receiver)? {
        match command["command"].as_str() {
            Some("audio_stop") => assert_ne!(command["stream_id"], "different-stream"),
            Some("capture_stop") => capture_stop = Some(command),
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    }
    assert!(capture_stop.is_some(), "uncertain stream was not cut");

    Ok(())
}

async fn invalid_retired_start_stream(stream: &str, cause: &str) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":stream,"source":"remote_app"}}))
        .await?;
    assert!(
        next_capture_command(&mut receiver)?.is_none(),
        "{cause} sent an order"
    );

    Ok(())
}

#[tokio::test]
async fn invalid_retired_start_streams_leave_the_current_capture_alone() -> Result<()> {
    invalid_retired_start_stream("", "retired_audio_start_empty_stream").await?;
    invalid_retired_start_stream(&"x".repeat(129), "retired_audio_start_stream_too_long").await?;

    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    admit_call_after_empty_capture(&mut runtime, "new-call").await?;
    let call_start = loop {
        let command = receiver.try_recv().context("call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":call_start["stream_id"],"source":"remote_app"}}))
        .await?;
    assert!(next_capture_command(&mut receiver)?.is_none());
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        call_start["stream_id"].as_str()
    );

    Ok(())
}

#[tokio::test]
async fn invalid_retired_stop_receipts_name_each_cause() -> Result<()> {
    for (result, cause) in [
        (json!(null), "retired_audio_stop_result_absent"),
        (
            json!({"stream_id":"live-stream","drain_complete":true}),
            "retired_audio_stop_stream_mismatch",
        ),
        (
            json!({"stream_id":42,"drain_complete":true}),
            "retired_audio_stop_result_malformed",
        ),
        (
            json!({"stream_id":"other","drain_complete":true}),
            "retired_audio_stop_stream_mismatch",
        ),
        (
            json!({"stream_id":"other","drain_complete":false}),
            "retired_audio_stop_drain_incomplete",
        ),
    ] {
        let (runtime, _receiver, _directory, _call, old_stream) =
            retired_stop_with_call(|id, call| {
                let mut result = result.clone();
                if result["stream_id"] == "live-stream" {
                    result["stream_id"] = json!(call);
                }
                json!({"event":"response","id":id,"ok":true,"result":result})
            })
            .await?;

        assert!(said::mentions(&runtime.state, cause), "{cause}");
        assert!(!old_stream.is_empty());
        assert!(runtime.capture.interruption.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn retired_stop_capacity_cuts_instead_of_forgetting_an_order() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    let remote = retired_system_start(&mut runtime, &mut receiver).await?;
    admit_call_after_empty_capture(&mut runtime, "new-call").await?;
    let call_start = loop {
        let command = receiver.try_recv().context("call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected helper command: {command}"),
        }
    };
    for index in 0..64 {
        runtime.capture.retired_stops.insert(
            format!("synthetic-stop-{index}"),
            crate::daemon::audio::RetiredStreamStop {
                stream: format!("synthetic-stream-{index}"),
                sent: true,
                may_finish_call: true,
            },
        );
    }
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":true,
            "result":{"stream_id":remote["stream_id"],"source":"remote_app"}}))
        .await?;
    let stop = loop {
        let command =
            next_capture_command(&mut receiver)?.context("capacity capture_stop absent")?;
        match command["command"].as_str() {
            Some("audio_stop") => {
                assert_eq!(command["stream_id"], call_start["stream_id"]);
            }
            Some("capture_stop") => break command,
            _ => anyhow::bail!("unexpected helper command at capacity: {command}"),
        }
    };
    assert_eq!(stop["command"], "capture_stop");
    assert_eq!(runtime.capture.retired_stops.len(), 64);
    assert!(said::mentions(
        &runtime.state,
        "retired_stop_capacity_exhausted"
    ));

    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    assert!(runtime.capture.retired_stops.is_empty());
    Ok(())
}

async fn unanswered_call_start(
    dir: &std::path::Path,
) -> Result<(Runtime, mpsc::Receiver<Value>, Value)> {
    let (mut runtime, mut receiver) = fixture(dir)?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(dir));
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
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
            id: "unanswered-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    let start = next_capture_command(&mut receiver)?.context("call audio_start absent")?;
    assert_eq!(start["command"], "audio_start");
    Ok((runtime, receiver, start))
}

fn call_start_success(start: &Value) -> Value {
    json!({"event":"response","id":start["id"],"ok":true,
        "result":{"stream_id":start["stream_id"],"processing":"voice_processing",
            "processing_enabled":true,"processing_bypassed":false,
            "agc_enabled":false,"playback_reference":"shared_engine",
            "pipeline_id":"synthetic-pipeline"}})
}

#[tokio::test]
async fn failed_retired_call_start_finishes_the_hangup() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(directory.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("call audio_stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":false,
            "error":{"code":"stream_not_found","message":"synthetic stop overtook start"}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
            "error":{"code":"audio_session_busy","message":"synthetic start failed"}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.capture.mic_stream.is_none(), "mic_stream kept");
    assert!(!runtime.audio_is_draining(), "call still draining");
    assert!(runtime.capture.call_runtime.is_none(), "call still closing");
    assert!(not_current(&runtime, NoticeCode::RetiredStopUnconfirmed));
    assert!(not_current(&runtime, NoticeCode::AudioStopUnconfirmed));
    Ok(())
}

#[tokio::test]
async fn failed_start_then_late_stop_is_read_as_retired() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
        "error":{"code":"audio_session_busy","message":"synthetic"}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.capture.call_runtime.is_none());
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":false,
        "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    let stop_id = stop["id"].as_str().context("stop id")?;
    assert!(!runtime.capture.retired_stops.contains_key(stop_id));
    assert!(!runtime.audio_is_draining());

    assert!(
        not_current(&runtime, NoticeCode::MacCommandDegraded),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(not_current(&runtime, NoticeCode::AudioDrainWaiting));
    assert!(
        said::mentions(&runtime.state, "audio_session_busy"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn late_success_then_first_stop_receipt_is_read_as_retired() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first stop absent")?;
    runtime.native_event(call_start_success(&start)).await?;
    let late_stop = next_capture_command(&mut receiver)?.context("late stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":first_stop["id"],"ok":false,
        "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":late_stop["id"],"ok":true,
        "result":{"stream_id":start["stream_id"],"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let first_stop_id = first_stop["id"].as_str().context("first stop id")?;
    assert!(!runtime.capture.retired_stops.contains_key(first_stop_id));
    assert!(!runtime.audio_is_draining());

    assert!(runtime.capture.call_runtime.is_none());
    assert!(
        not_current(&runtime, NoticeCode::MacCommandDegraded),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(not_current(&runtime, NoticeCode::AudioDrainWaiting));
    Ok(())
}

#[tokio::test]
async fn late_start_closes_only_after_the_late_stop_receipt() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first stop absent")?;
    runtime.native_event(call_start_success(&start)).await?;
    let late_stop = next_capture_command(&mut receiver)?.context("late stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":first_stop["id"],"ok":true,
            "result":{"stream_id":start["stream_id"],"drain_complete":true}}))
        .await?;
    assert!(
        runtime.capture.call_runtime.is_some(),
        "first stop closed call early"
    );
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        start["stream_id"].as_str()
    );
    runtime
        .native_event(json!({"event":"response","id":late_stop["id"],"ok":true,
            "result":{"stream_id":start["stream_id"],"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let id = first_stop["id"].as_str().context("first stop id")?;
    assert!(!runtime.capture.retired_stops.contains_key(id));
    assert!(runtime.capture.call_runtime.is_none());
    assert!(runtime.capture.mic_stream.is_none());

    assert!(!said::mentions(&runtime.state, "playback"));
    assert!(not_current(&runtime, NoticeCode::AudioDrainWaiting));
    Ok(())
}

#[tokio::test]
async fn playback_not_found_remains_a_player_rejection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, _start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":false,
        "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    runtime
        .native_event(
            json!({"event":"response","id":"playback-probe-1","ok":false,
        "_bridge_command":"playback", "error":{"code":"stream_not_found","message":"synthetic"}}),
        )
        .await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::MacCommandDegraded),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(said::mentions(&runtime.state, "playback"));
    assert!(said::mentions(&runtime.state, "stream_not_found"));
    Ok(())
}

#[tokio::test]
async fn answered_call_stop_not_found_is_not_overtaken() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.native_event(call_start_success(&start)).await?;
    while next_capture_command(&mut receiver)?.is_some() {}
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":false,
        "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::MacCommandDegraded)
    );
    assert!(
        said::mentions(&runtime.state, "stream_not_found"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn degraded_reply_names_its_command() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(dir.path())?;
    runtime
        .native_event(json!({"event":"response","id":"probe-audio-stop",
            "ok":false,"_bridge_command":"audio_stop",
            "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::MacCommandDegraded),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(said::mentions(&runtime.state, "audio_stop"));
    assert!(said::mentions(&runtime.state, "stream_not_found"));
    assert!(!said::mentions(&runtime.state, "playback"));
    Ok(())
}

#[tokio::test]
async fn degraded_start_names_its_command() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, _receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],
            "ok":false,"error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::MacCommandDegraded),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(said::mentions(&runtime.state, "audio_start"));
    assert!(said::mentions(&runtime.state, "stream_not_found"));
    assert!(!said::mentions(&runtime.state, "playback"));
    Ok(())
}

#[tokio::test]
async fn confirmed_call_drain_replaces_wait_notice() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.native_event(call_start_success(&start)).await?;
    while next_capture_command(&mut receiver)?.is_some() {}
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("stop absent")?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioDrainWaiting)
    );
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
        "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.capture.call_runtime.is_none());
    assert!(not_current(&runtime, NoticeCode::AudioDrainWaiting));
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioDrainConfirmed),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn answered_stop_then_start_failure_leaves_no_debt() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":first_stop["id"],"ok":false,
        "error":{"code":"stream_not_found","message":"synthetic"}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
        "error":{"code":"audio_session_busy","message":"synthetic"}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.capture.call_runtime.is_none());
    assert!(
        runtime.capture.retired_stops.is_empty(),
        "answered stop left as debt"
    );
    Ok(())
}

#[tokio::test]
async fn seventy_overtaken_hangups_do_not_cut_capture() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, mut start) = unanswered_call_start(dir.path()).await?;
    for round in 0..70 {
        if round > 0 {
            runtime
                .apply(Event::CallStart {
                    id: format!("round-{round}"),
                    duration_ms: 60_000,
                    origin: orbit_core::CallOrigin::App,
                })
                .await?;
            start = loop {
                let command = next_capture_command(&mut receiver)?.context("start absent")?;
                if command["command"] == "audio_start" {
                    break command;
                }
            };
        }
        runtime.apply(Event::CallStop).await?;
        let first_stop = loop {
            let command = next_capture_command(&mut receiver)?.context("stop absent")?;
            if command["command"] == "audio_stop" {
                break command;
            }
        };
        assert_eq!(first_stop["stream_id"], start["stream_id"]);
        runtime
            .native_event(json!({"event":"response","id":first_stop["id"],"ok":false,
            "error":{"code":"stream_not_found","message":"synthetic"}}))
            .await?;
        runtime.native_event(call_start_success(&start)).await?;
        let late_stop = loop {
            let command = next_capture_command(&mut receiver)?.context("late stop absent")?;
            if command["command"] == "audio_stop" {
                break command;
            }
        };
        assert_eq!(late_stop["stream_id"], start["stream_id"]);
        assert!(receiver.try_recv().is_err());
        runtime
            .native_event(json!({"event":"response","id":late_stop["id"],"ok":true,
            "result":{"stream_id":late_stop["stream_id"],"drain_complete":true}}))
            .await?;
        settle_seal_receipts(&mut runtime).await?;
        assert!(receiver.try_recv().is_err());
        assert!(runtime.capture.mic_stream.is_none());
        assert!(!runtime.audio_is_draining());
        assert!(
            runtime.capture.call_runtime.is_none(),
            "call open at round {round}"
        );
        assert!(
            runtime.capture.retired_stops.is_empty(),
            "ledger leaked at round {round}"
        );
        assert!(
            !said::mentions(&runtime.state, "retired_stop_capacity_exhausted"),
            "round {round}: {:?}",
            runtime.state.durable.notice
        );
        runtime.state.durable.voice.call = None;
    }
    Ok(())
}

#[tokio::test]
async fn privacy_notice_survives_retired_start_failure() -> Result<()> {
    for code in [
        "privacy_stopped",
        "operation_cancelled_by_stop",
        "audio_session_busy",
    ] {
        let dir = tempfile::tempdir()?;
        let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
        runtime.apply(Event::CallStop).await?;
        let _stop = next_capture_command(&mut receiver)?.context("stop absent")?;
        runtime
            .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
            .await;
        assert!(
            said::has(&runtime.state, NoticeCode::PrivacyStopped)
                && said::mentions(&runtime.state, "order"),
            "{:?}",
            runtime.state.durable.notice_log
        );
        runtime
            .native_event(json!({"event":"response","id":start["id"],"ok":false,
            "error":{"code":code,"message":"synthetic"}}))
            .await?;
        settle_seal_receipts(&mut runtime).await?;
        assert!(
            said::has(&runtime.state, NoticeCode::PrivacyStopped)
                && said::mentions(&runtime.state, "order"),
            "{code}: {:?}",
            runtime.state.durable.notice_log
        );
        assert!(
            !said::has(&runtime.state, NoticeCode::AudioStartRefused),
            "{code}: {:?}",
            runtime.state.durable.notice_log
        );
    }
    Ok(())
}

#[tokio::test]
async fn cut_cause_survives_retired_start_failure() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let _stop = next_capture_command(&mut receiver)?.context("stop absent")?;
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
            "CAUSA SINTETICA DEL CORTE",
        )
        .await;
    if !runtime.state.privacy_stopped() && runtime.capture.audio.full_stop.is_none() {
        runtime
            .request_full_audio_stop(crate::daemon::audio::FullStop::Cut)
            .await?;
    }
    while next_capture_command(&mut receiver)?.is_some() {}
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
        "error":{"code":"audio_session_busy","message":"synthetic"}}))
        .await?;
    // The cut's cause is said with its code; its sentence is in daemon.log.
    assert!(
        said::mentions(&runtime.state, "helper_integrity_failure"),
        "{:?}",
        runtime.state.durable.notice_log
    );

    assert!(
        said::has(&runtime.state, NoticeCode::OtherTrackStopped)
            && said::mentions(&runtime.state, "audio_session_busy"),
        "{:?}",
        runtime.state.durable.notice_log
    );
    Ok(())
}

#[tokio::test]
async fn an_old_stop_error_does_not_reopen_the_closed_call() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first stop absent")?;
    runtime.native_event(call_start_success(&start)).await?;
    let late_stop = next_capture_command(&mut receiver)?.context("late stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":first_stop["id"],"ok":false,
        "error":{"code":"audio_drain_incomplete","message":"synthetic"}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":late_stop["id"],"ok":true,
        "result":{"stream_id":late_stop["stream_id"],"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let id = first_stop["id"].as_str().context("first stop id")?;
    assert!(!runtime.capture.retired_stops.contains_key(id));

    assert!(runtime.capture.call_runtime.is_none());
    assert!(
        not_current(&runtime, NoticeCode::RetiredStopUnconfirmed),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn retired_pending_stop_obeys_capacity() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(dir.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let first_stop = next_capture_command(&mut receiver)?.context("first stop absent")?;
    for index in 0..64 {
        runtime.capture.retired_stops.insert(
            format!("synthetic-stop-{index}"),
            crate::daemon::audio::RetiredStreamStop {
                stream: format!("synthetic-stream-{index}"),
                sent: true,
                may_finish_call: false,
            },
        );
    }
    runtime.native_event(call_start_success(&start)).await?;
    assert_eq!(runtime.capture.retired_stops.len(), 64);
    assert!(
        said::mentions(&runtime.state, "retired_stop_capacity_exhausted"),
        "{:?}",
        runtime.state.durable.notice
    );
    let mut cut = None;
    while let Some(command) = next_capture_command(&mut receiver)? {
        if command["command"] == "capture_stop" {
            cut = Some(command);
        }
    }
    assert!(cut.is_some(), "capacity must request a full stop");
    assert!(
        !runtime
            .capture
            .retired_stops
            .contains_key(first_stop["id"].as_str().context("first stop id")?)
    );
    Ok(())
}

#[tokio::test]
async fn overtaken_stop_is_not_a_playback_rejection() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver, _start) = unanswered_call_start(directory.path()).await?;
    runtime.apply(Event::CallStop).await?;
    let stop = next_capture_command(&mut receiver)?.context("call audio_stop absent")?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":false,
            "error":{"code":"stream_not_found","message":"synthetic stop overtook start"}}))
        .await?;
    assert!(runtime.audio_is_draining());
    assert!(!said::mentions(&runtime.state, "playback"));

    Ok(())
}

async fn privacy_stop_after_unanswered_call_start(
    receipt_first: bool,
    stop_error: &str,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver, start) = unanswered_call_start(directory.path()).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    let mut privacy = None;
    while let Some(command) = next_capture_command(&mut receiver)? {
        if command["command"] == "privacy_stop" {
            privacy = Some(command);
        }
    }
    let privacy = privacy.context("privacy_stop absent")?;
    runtime.native_event(call_start_success(&start)).await?;
    let mut extra = Vec::new();
    while let Some(command) = next_capture_command(&mut receiver)? {
        extra.push(command);
    }
    let privacy_receipt = json!({"event":"response","id":privacy["id"],"ok":true,
        "result":{"drain_complete":true,"privacy_stopped":true}});
    if receipt_first {
        runtime.native_event(privacy_receipt.clone()).await?;
    }
    for command in &extra {
        if command["command"] == "audio_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":false,
                    "error":{"code":stop_error,"message":"Orbit está detenido en este Mac."}}))
                .await?;
        }
    }
    let before_receipt = runtime.state.notice_code();
    if !receipt_first {
        runtime.native_event(privacy_receipt).await?;
    }
    settle_seal_receipts(&mut runtime).await?;
    assert_ne!(
        before_receipt,
        Some(NoticeCode::RetiredStopUnconfirmed),
        "{extra:?}"
    );
    assert!(not_current(&runtime, NoticeCode::RetiredStopUnconfirmed));
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    for command in &extra {
        if command["command"] == "audio_stop" {
            assert!(log.contains(command["id"].as_str().context("stop id absent")?));
            assert!(log.contains(stop_error), "{log}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn privacy_receipt_after_retired_start_stop_keeps_the_users_cause() -> Result<()> {
    privacy_stop_after_unanswered_call_start(false, "privacy_stopped").await
}

#[tokio::test]
async fn privacy_receipt_before_retired_start_stop_keeps_the_users_cause() -> Result<()> {
    privacy_stop_after_unanswered_call_start(true, "privacy_stopped").await
}

#[tokio::test]
async fn cancelled_selective_stop_after_the_users_brake_stays_in_log() -> Result<()> {
    privacy_stop_after_unanswered_call_start(false, "operation_cancelled_by_stop").await
}

#[tokio::test]
async fn start_then_hangup_receipts_finish_call_in_both_orders() -> Result<()> {
    for retired_answer in ["stream_not_found", "confirmed"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver, start) = unanswered_call_start(directory.path()).await?;
        runtime.apply(Event::CallStop).await?;
        let stop = next_capture_command(&mut receiver)?.context("call audio_stop absent")?;
        runtime.native_event(call_start_success(&start)).await?;
        let mut extra = Vec::new();
        while let Some(command) = next_capture_command(&mut receiver)? {
            extra.push(command);
        }
        runtime
            .native_event(json!({"event":"response","id":stop["id"],"ok":true,
                "result":{"stream_id":start["stream_id"],"drain_complete":true}}))
            .await?;
        for command in &extra {
            if command["command"] == "audio_stop" {
                let receipt = if retired_answer == "confirmed" {
                    json!({"event":"response","id":command["id"],"ok":true,
                        "result":{"stream_id":command["stream_id"],"drain_complete":true}})
                } else {
                    json!({"event":"response","id":command["id"],"ok":false,
                        "error":{"code":"stream_not_found","message":"synthetic stop found no stream"}})
                };
                runtime.native_event(receipt).await?;
            }
        }
        settle_seal_receipts(&mut runtime).await?;
        assert!(runtime.capture.mic_stream.is_none());
        assert!(!runtime.audio_is_draining());
        assert!(runtime.capture.call_runtime.is_none());
        assert!(not_current(&runtime, NoticeCode::RetiredStopUnconfirmed));
    }
    Ok(())
}
