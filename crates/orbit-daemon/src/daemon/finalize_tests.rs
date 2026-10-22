//! These regressions use real encrypted SQLite and synthetic PCM/native receipts.
//! No model, network request or physical capture is executed.
use super::*;
use crate::daemon::audio::tests::{fixture, receive_helper_command, settle_seal_receipts};
use crate::daemon::turn::said;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::{Event, State};
use tokio::sync::mpsc;

const MANIFEST_RECORDING: &str = "22222222-2222-4222-8222-222222222222";

#[path = "capture_protocol_tests.rs"]
mod capture_protocol;

fn titled_manifest(title: Option<&str>) -> Result<Value> {
    manifest_of(title.map(|title| json!({"recording_id":MANIFEST_RECORDING,"title":title})))
}

/// The closure `finalize.rs` writes for a recording whose own job
/// (`recording-{id}`) holds `recording`.
fn manifest_of(recording: Option<Value>) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    for (track, count) in [("mic_local", 3), ("remote_app", 2)] {
        for sequence in 0..count {
            runtime.fixture_store.seal_chunk(
                &orbit_store::ChunkMetadata {
                    capture_epoch_id: None,
                    id: format!("manifest-{track}-{sequence}"),
                    recording_id: MANIFEST_RECORDING.into(),
                    track: track.into(),
                    sequence,
                    sample_rate: 16_000,
                    channels: 1,
                    frames: 16_000,
                    monotonic_ns: 1_000_000_000 + sequence * 1_000_000_000,
                    first_sample: sequence * 16_000,
                    started_at_ms: 1_700_000_001_000 + sequence as i64 * 1_000,
                },
                b"synthetic-manifest-audio",
            )?;
        }
    }
    if let Some(payload) = recording {
        runtime.fixture_store.enqueue_job(&Job {
            id: format!("recording-{MANIFEST_RECORDING}"),
            kind: "recording_manifest".into(),
            state: JobState::Succeeded,
            created_at: 1_700_000_000_000,
            payload,
        })?;
    }
    Ok(runtime
        .capture_finalize_job(MANIFEST_RECORDING, 1_700_000_000_000, 1_700_000_010_000, 2)?
        .payload["manifest"]
        .clone())
}

/// A meeting linked to a calendar event, its invitees and series as the
/// hub's `EventContext` admits them. The user (`is_self`) goes first; the address
/// longer than RFC 5321 allows is left out, never cut.
fn linked_meeting() -> Value {
    json!({"recording_id":MANIFEST_RECORDING,"title":"Comité de dirección","event":{
    "calendar_item_id":"serie-comite","occurrence_start_ms":1_700_000_000_000_i64,
    "title":"Comité de dirección","starts_at_ms":1_700_000_000_000_i64,
    "ends_at_ms":1_700_003_600_000_i64,"recurring":true,
    "attendees":[
        {"name":"Ada","email":format!("{}@example.com", "a".repeat(250)),"is_self":false},
        {"name":"Alina","email":"alina@example.com","is_self":true}
    ]}})
}

fn contract_manifest_variants() -> Result<[(&'static str, Value); 5]> {
    let titled = titled_manifest(Some("Reunión ágil"))?;
    let null_title = titled_manifest(None)?;
    let mut legacy = null_title.clone();
    legacy
        .as_object_mut()
        .context("generated manifest is not an object")?
        .remove("title");
    Ok([
        ("recording-manifest-title-v1.json", titled),
        ("recording-manifest-title-null-v1.json", null_title),
        ("recording-manifest-title-missing-v1.json", legacy),
        (
            "recording-manifest-event-v1.json",
            manifest_of(Some(linked_meeting()))?,
        ),
        (
            "recording-manifest-voice-call-v1.json",
            manifest_of(Some(json!({"recording_id":MANIFEST_RECORDING,
                "recording_kind":"voice_call","started_at_ms":1_700_000_000_000_i64})))?,
        ),
    ])
}

#[test]
fn manifest_fixtures_match_the_daemon_writer() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures");
    for (name, generated) in contract_manifest_variants()? {
        let versioned: Value = serde_json::from_slice(&std::fs::read(root.join(name))?)?;
        assert_eq!(
            versioned, generated,
            "{name} differs from finalize.rs; regenerate with `cargo test -p orbit-daemon write_manifest_fixtures -- --ignored`"
        );
    }
    Ok(())
}

/// The closure says what the recording is, so the hub lists a
/// meeting and asks for its note with the event's series template, and
/// leaves a voice call out.
#[test]
fn the_closure_says_its_kind_and_carries_its_event() -> Result<()> {
    let meeting = manifest_of(Some(linked_meeting()))?;
    assert_eq!(meeting["kind"], "meeting");
    assert_eq!(
        meeting["event"],
        json!({"title":"Comité de dirección","starts_at_ms":1_700_000_000_000_i64,
        "ends_at_ms":1_700_003_600_000_i64,"calendar_item_id":"serie-comite",
        "attendees":[
            {"name":"Alina","email":"alina@example.com","is_self":true},
            {"name":"Ada","email":"","is_self":false}
        ]})
    );
    let untitled = titled_manifest(None)?;
    assert_eq!(untitled["kind"], "meeting");
    assert!(untitled.get("event").is_none());
    let call = manifest_of(Some(json!({"recording_id":MANIFEST_RECORDING,
        "recording_kind":"voice_call","started_at_ms":1_700_000_000_000_i64})))?;
    assert_eq!(call["kind"], "voice_call");
    assert!(call.get("event").is_none());
    Ok(())
}

#[test]
#[ignore = "writes the recording manifest contract fixtures; run explicitly"]
fn write_manifest_fixtures() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures");
    for (name, generated) in contract_manifest_variants()? {
        let mut bytes = serde_json::to_vec_pretty(&generated)?;
        bytes.push(b'\n');
        std::fs::write(root.join(name), bytes)?;
    }
    Ok(())
}

#[path = "finalize_lifecycle_tests.rs"]
mod lifecycle;

async fn start(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    id: &str,
) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: id.into(),
            title: "Synthetic recording".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    start_ack(runtime, receiver).await
}

async fn start_ack(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<String> {
    let command = receive_helper_command(receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":true,
        "result":{"stream_id":command["stream_id"]}}))
        .await?;
    Ok(command["stream_id"]
        .as_str()
        .context("stream identity absent")?
        .into())
}

async fn admit_call_after_empty_capture(runtime: &mut Runtime, id: &str) -> Result<()> {
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
            id: id.into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some(id)
    );
    Ok(())
}

fn frame(stream: &str, frames: usize) -> Value {
    json!({"event":"audio","source":"mic_local","stream_id":stream,
        "sample_rate":8000,"channels":1,"frames":frames,"sequence":0,
        "monotonic_ns":1_000_000u64,"pcm_s16le_base64":STANDARD.encode(vec![1; frames*2])})
}

#[tokio::test]
async fn empty_privacy_interruption_closes_before_rearm_and_admits_call() -> Result<()> {
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
    start(&mut runtime, &mut receiver, "empty").await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    loop {
        let command = receiver.recv().await.context("privacy stop absent")?;
        if command["command"] == "privacy_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                    "result":{"privacy_stopped":true,"drain_complete":true}}))
                .await?;
            break;
        }
    }
    settle_seal_receipts(&mut runtime).await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.sealed_chunks, 0);
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(
        recording.ended_at_ms.is_some(),
        "empty interruption stayed open"
    );
    // The empty close is its own notice.
    assert!(
        said::has(&runtime.state, NoticeCode::RecordingEmpty),
        "current notice: {:?}; history: {:?}",
        runtime.state.durable.notice,
        runtime.state.durable.notice_log.records()
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-empty")?
        .context("empty closure absent")?;
    assert!(closure.payload["manifest"].get("interruption").is_none());
    assert_eq!(closure.payload["interruption"]["audio_available"], false);
    assert_eq!(closure.payload["interruption"]["cause"], "privacy_stop");
    runtime.apply(Event::LocalRearm).await?;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime
        .apply(Event::CallStart {
            id: "next-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("next-call")
    );
    Ok(())
}

#[tokio::test]
async fn helper_loss_closes_empty_recording_and_allows_another() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, "lost-helper").await?;
    runtime
        .native_event(json!({"event":"helper_disconnected"}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(
        recording.ended_at_ms.is_some(),
        "helper loss left an empty recording open"
    );
    // The helper's loss cut the capture, said with its cause's code, then
    // the empty close.
    assert!(said::mentions(&runtime.state, "helper_lost"));
    assert!(said::has(&runtime.state, NoticeCode::CaptureCut));
    assert!(said::has(&runtime.state, NoticeCode::RecordingEmpty));
    assert!(!runtime.state.privacy_stopped());
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
            "capabilities.microphone_state",
            fixture_value,
        );
    }
    runtime
        .apply(Event::StartMeeting {
            id: "after-loss".into(),
            title: "After loss".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|r| r.id.as_str()),
        Some("after-loss")
    );
    Ok(())
}

#[tokio::test]
async fn empty_capture_cut_preserves_its_cause_without_claiming_saved_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start(&mut runtime, &mut receiver, "cut-cause").await?;
    runtime
        .native_event(json!({"event":"helper_disconnected",
            "cause":"native_audio_invalid event=audio field=sample_rate type=string"}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;

    // The cut is said with its code and the empty close after it;
    // the helper's detailed cause is in daemon.log, not in the notice.
    let notice = &runtime.state.durable.notice;
    assert!(said::mentions(&runtime.state, "helper_lost"), "{notice:?}");
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::RecordingEmpty)
    );
    assert!(
        !said::has(&runtime.state, NoticeCode::CaptureCutConfirmed),
        "an empty cut claims no saved audio: {:?}",
        runtime.state.durable.notice_log
    );
    assert_eq!(runtime.state.durable.notice_log.latest(), notice.as_ref());
    let closure = runtime
        .fixture_store
        .get_job("finalize-cut-cause")?
        .context("empty closure absent")?;
    assert_eq!(closure.payload["interruption"]["cause"], "helper_lost");
    assert!(
        closure.payload["interruption"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("sample_rate")),
        "{}",
        closure.payload
    );
    Ok(())
}

#[tokio::test]
async fn permission_refusal_closes_empty_capture_and_admits_call_immediately() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "permission-refused".into(),
            title: "Permission refused".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let command = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":false,
            "error":{"code":"microphone_permission_required",
                "message":"Falta el permiso de micrófono de macOS"}}))
        .await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(
        recording.ended_at_ms.is_some(),
        "permission refusal stayed open"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-permission-refused")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "microphone_permission_required"
    );
    assert!(said::has(
        &runtime.state,
        NoticeCode::MicrophonePermissionMissing
    ));
    assert!(said::has(&runtime.state, NoticeCode::RecordingEmpty));
    admit_call_after_empty_capture(&mut runtime, "after-permission-refusal").await?;
    Ok(())
}

#[tokio::test]
async fn remote_start_refusal_stops_the_started_microphone_before_closing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "partial-permission-refusal".into(),
            title: "System audio refused".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_ms: 60_000,
        })
        .await?;
    let microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    assert_eq!(microphone["source"], "mic_local");
    assert_eq!(remote["source"], "remote_app");
    runtime
        .native_event(json!({"event":"response","id":microphone["id"],"ok":true,
            "result":{"stream_id":microphone["stream_id"]}}))
        .await?;
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"screen_permission_required",
                "message":"Falta el permiso de captura de pantalla"}}))
        .await?;

    assert!(said::has(
        &runtime.state,
        NoticeCode::ScreenPermissionMissing
    ));
    let stop = receiver
        .try_recv()
        .context("capture_stop absent after partial refusal")?;
    assert_eq!(stop["command"], "capture_stop");
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .and_then(|r| r.ended_at_ms),
        None
    );
    assert!(
        runtime.accepts_audio(&frame(
            microphone["stream_id"]
                .as_str()
                .context("microphone stream absent")?,
            100
        ))
    );
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert!(recording.ended_at_ms.is_some());
    let closure = runtime
        .fixture_store
        .get_job("finalize-partial-permission-refusal")?
        .context("closure absent after drain")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "screen_permission_required"
    );
    admit_call_after_empty_capture(&mut runtime, "after-partial-refusal").await?;
    Ok(())
}

#[tokio::test]
async fn retired_start_refusal_does_not_touch_the_next_call() -> Result<()> {
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
    runtime
        .apply(Event::StartMeeting {
            id: "old-system".into(),
            title: "Synthetic system recording".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_ms: 60_000,
        })
        .await?;
    let microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    assert_eq!(microphone["source"], "mic_local");
    assert_eq!(remote["source"], "remote_app");
    runtime
        .cut_capture(
            orbit_core::privacy_stop::StopCause::HelperCaptureStop,
            "helper_capture_stop",
            "Corte sintético anterior",
        )
        .await;
    let stop = receive_helper_command(&mut receiver, "capture_stop").await?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    admit_call_after_empty_capture(&mut runtime, "next-call").await?;
    let call_start = loop {
        let command = receiver.recv().await.context("call audio_start absent")?;
        match command["command"].as_str() {
            Some("playback_stop" | "status") => continue,
            Some("audio_start") => break command,
            _ => anyhow::bail!("unexpected command before call start: {command}"),
        }
    };
    let call_stream = call_start["stream_id"]
        .as_str()
        .context("call stream absent")?;
    let interruption_before = runtime.capture.interruption.clone();
    let notice_before = runtime.state.durable.notice.clone();

    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"screen_permission_required",
                "message":"Falta el permiso de captura de pantalla"}}))
        .await?;
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(call_stream));
    assert_eq!(runtime.capture.interruption, interruption_before);
    assert_eq!(runtime.state.durable.notice, notice_before);
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .map(|call| call.id.as_str()),
        Some("next-call")
    );
    assert!(
        runtime
            .capture
            .pending_starts
            .iter()
            .any(|id| id == call_start["id"].as_str().unwrap_or_default())
    );
    assert!(
        receiver.try_recv().is_err(),
        "late refusal requested another helper action"
    );
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(!log.contains("Synthetic system recording"));
    Ok(())
}

#[tokio::test]
async fn voice_refusal_during_starting_recording_keeps_permission_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "starting-with-voice".into(),
            title: "Synthetic starting recording".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let start = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime.apply(Event::VoiceStart).await?;
    let playback_stop = receiver.try_recv().context("playback_stop absent")?;
    assert_eq!(playback_stop["command"], "playback_stop");
    let purpose = receive_helper_command(&mut receiver, "audio_purpose").await?;
    assert_eq!(purpose["stream_id"], start["stream_id"]);
    assert_eq!(purpose["purpose"], "voice_note");
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
            "error":{"code":"microphone_permission_required",
                "message":"Falta el permiso de micrófono de macOS"}}))
        .await?;

    // A refused voice note says the missing permission, never «no audio»;
    // the recording's empty close that follows is only recorded (silent),
    // so the fixable cause stays the visible one for this op.
    assert!(
        said::has(&runtime.state, NoticeCode::MicrophonePermissionMissing),
        "{:?}",
        runtime.state.durable.notice_log
    );
    assert!(!said::has(&runtime.state, NoticeCode::VoiceNoteEmpty));
    assert!(!runtime.state.durable.voice_active);
    assert!(runtime.capture.buffer.voice.is_none());
    assert!(runtime.capture.audio.full_stop.is_none());
    assert!(
        receiver.try_recv().is_err(),
        "an empty voice note requested another helper action"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-starting-with-voice")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "microphone_permission_required"
    );
    Ok(())
}

#[tokio::test]
async fn second_refusal_keeps_first_cause_and_wait_notice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "two-refusals".into(),
            title: "Synthetic two-track recording".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_ms: 60_000,
        })
        .await?;
    let microphone = receive_helper_command(&mut receiver, "audio_start").await?;
    let remote = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":remote["id"],"ok":false,
            "error":{"code":"screen_permission_required", "message":"Pantalla denegada"}}))
        .await?;
    let stop = receiver
        .try_recv()
        .context("capture_stop absent after first refusal")?;
    assert_eq!(stop["command"], "capture_stop");
    runtime
        .native_event(json!({"event":"response","id":microphone["id"],"ok":false,
            "error":{"code":"microphone_permission_required", "message":"Micrófono denegado"}}))
        .await?;
    // Both refusals are said, the second track's code included;
    // the wait for the Mac's confirmation is no longer a sentence.
    assert!(
        said::has(&runtime.state, NoticeCode::ScreenPermissionMissing),
        "{:?}",
        runtime.state.durable.notice_log
    );
    assert!(
        said::mentions(&runtime.state, "microphone_permission_required"),
        "{:?}",
        runtime.state.durable.notice_log
    );
    assert_eq!(
        runtime
            .capture
            .audio
            .full_stop
            .as_ref()
            .map(|(id, _)| id.as_str()),
        stop["id"].as_str()
    );
    assert!(
        receiver.try_recv().is_err(),
        "second refusal requested a second stop"
    );
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-two-refusals")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "screen_permission_required"
    );
    Ok(())
}

#[tokio::test]
async fn expiry_closes_empty_capture_and_admits_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    // This test has no hub token: keep the closure's unrelated upload worker
    // offline so its credential refusal cannot replace the capture notice.
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "expired".into(),
        title: "Expired".into(),
        started_at_ms: 0,
        ended_at_ms: None,
        expires_at_ms: 0,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Recording,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    runtime.run_maintenance().await;

    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(
        recording.ended_at_ms.is_some(),
        "expired empty capture stayed open"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-expired")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "recording_expired"
    );
    assert!(
        said::has(&runtime.state, NoticeCode::RecordingCeilingReached),
        "{:?}",
        runtime.state.durable.notice_log
    );
    let ceiling = runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .find(|notice| notice.code == NoticeCode::RecordingCeilingReached)
        .context("ceiling notice")?;
    assert_eq!(
        ceiling.data.get(&orbit_core::notice::NoticeKey::Minutes),
        Some(&orbit_core::notice::DataValue::Int(0))
    );
    admit_call_after_empty_capture(&mut runtime, "after-expiry").await?;
    Ok(())
}

#[tokio::test]
async fn live_expiry_closes_after_the_empty_audio_drain() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, "live-expired").await?;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime
        .state
        .durable
        .recording
        .as_mut()
        .context("recording absent")?
        .expires_at_ms = 0;

    runtime.run_maintenance().await;
    let stop = receive_helper_command(&mut receiver, "audio_stop").await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .and_then(|r| r.ended_at_ms),
        None
    );
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stop["stream_id"],"drain_complete":true}}))
        .await?;

    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .and_then(|r| r.ended_at_ms)
            .is_some(),
        "the drained empty capture stayed open"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-live-expired")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "recording_expired"
    );
    admit_call_after_empty_capture(&mut runtime, "after-live-expiry").await?;
    Ok(())
}

#[tokio::test]
async fn finalize_deadline_closes_empty_capture_and_admits_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "finalize-deadline".into(),
        title: "Finalize deadline".into(),
        started_at_ms: 0,
        ended_at_ms: None,
        expires_at_ms: i64::MAX,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Finalizing,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: Some(0),
    });
    runtime.run_maintenance().await;

    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(
        recording.ended_at_ms.is_some(),
        "finalize deadline stayed open"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-finalize-deadline")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "capture_finalize_timed_out"
    );
    assert!(
        said::has(&runtime.state, NoticeCode::RecordingCloseUnconfirmed),
        "{:?}",
        runtime.state.durable.notice_log
    );
    admit_call_after_empty_capture(&mut runtime, "after-finalize-deadline").await?;
    Ok(())
}

#[tokio::test]
async fn legacy_empty_interruption_is_closed_during_recovery() -> Result<()> {
    {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        start(&mut runtime, &mut receiver, "legacy").await?;
        runtime
            .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
            .await;
        // A snapshot after stop but before its helper has answered.
        let saved = serde_json::to_value(&runtime.state)?;
        runtime
            .store_actor
            .run(move |store| Ok(store.save_snapshot(&saved)?))
            .await?;
        drop(runtime);
        let (mut runtime, _receiver) = fixture(directory.path())?;
        runtime.state = runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?
            .recovered();
        runtime
            .close_empty_interrupted(orbit_core::RecordingInterruption {
                cause: orbit_core::RecordingInterruptionCause::CauseNotSaved,
                detail: None,
                audio_available: false,
            })
            .await?;
        let recording = runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?;
        assert_eq!(
            recording.phase,
            CapturePhase::Interrupted,
            "interrupted recording must retain its phase"
        );
        assert!(
            recording.ended_at_ms.is_some(),
            "legacy empty interruption stayed open"
        );
        assert!(runtime.fixture_store.get_job("finalize-legacy")?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn interrupted_recording_with_a_sealed_chunk_keeps_its_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver, "with-audio").await?;
    runtime.native_event(frame(&stream, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    loop {
        let command = receiver.recv().await.context("privacy stop absent")?;
        if command["command"] == "privacy_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                    "result":{"privacy_stopped":true,"drain_complete":true}}))
                .await?;
            break;
        }
    }
    settle_seal_receipts(&mut runtime).await?;
    runtime.run_maintenance().await;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.sealed_chunks, 1);
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(recording.ended_at_ms.is_none());
    assert_eq!(
        runtime.fixture_store.list_chunks("with-audio", None, 10)?[0]
            .metadata
            .frames,
        100
    );
    Ok(())
}

#[tokio::test]
async fn recovery_keeps_an_existing_closure_and_continues() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "already-closed".into(),
        title: "Already closed".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 61_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Interrupted,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    let existing = Job {
        id: "finalize-already-closed".into(),
        kind: "recording_finalize".into(),
        state: JobState::Succeeded,
        created_at: 2_000,
        payload: json!({"recording_id":"already-closed","manifest":{
            "tracks":[],"gaps":0,"started_at_ms":1_000,"finalized_at_ms":2_000,
            "title":"Already closed"},"interruption":{
                "cause":"privacy_stop","audio_available":false}}),
    };
    runtime.fixture_store.enqueue_job(&existing)?;

    runtime
        .close_empty_interrupted(orbit_core::RecordingInterruption {
            cause: orbit_core::RecordingInterruptionCause::Restart,
            detail: None,
            audio_available: false,
        })
        .await?;
    runtime.recover_call_closures(3_000).await?;

    assert_eq!(
        runtime.fixture_store.get_job("finalize-already-closed")?,
        Some(existing)
    );
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .ended_at_ms
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn later_stop_keeps_the_earlier_empty_interruption_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "earlier-cut".into(),
        title: "Earlier cut".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 61_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Interrupted,
        sealed_chunks: 0,
        gaps: 1,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    runtime.capture.interruption = Some(orbit_core::RecordingInterruption {
        cause: orbit_core::RecordingInterruptionCause::HelperLost,
        detail: None,
        audio_available: false,
    });
    runtime
        .close_empty_interrupted(orbit_core::RecordingInterruption {
            cause: orbit_core::RecordingInterruptionCause::PrivacyStop,
            detail: None,
            audio_available: false,
        })
        .await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-earlier-cut")?
        .context("closure absent")?;
    assert_eq!(closure.payload["interruption"]["cause"], "helper_lost");
    Ok(())
}

#[tokio::test]
async fn later_cut_closes_an_earlier_empty_interruption_without_relabeling() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "earlier-refusal".into(),
        title: "Earlier refusal".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 61_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Interrupted,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    runtime.capture.interruption = Some(orbit_core::RecordingInterruption {
        cause: orbit_core::RecordingInterruptionCause::HelperCaptureStop,
        detail: None,
        audio_available: false,
    });
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::HelperLost,
            "El helper se desconectó después del rechazo.",
        )
        .await;
    let closure = runtime
        .fixture_store
        .get_job("finalize-earlier-refusal")?
        .context("closure absent after later cut")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "helper_capture_stop"
    );
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .ended_at_ms
            .is_some()
    );
    assert!(said::has(&runtime.state, NoticeCode::RecordingEmpty));
    admit_call_after_empty_capture(&mut runtime, "after-later-cut").await?;
    Ok(())
}

#[tokio::test]
async fn live_cut_keeps_the_first_typed_interruption() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, "typed-before-live-cut").await?;
    runtime.capture.interruption = Some(orbit_core::RecordingInterruption {
        cause: orbit_core::RecordingInterruptionCause::HelperCaptureStop,
        detail: None,
        audio_available: false,
    });
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::HelperLost,
            "Se perdió el helper después del corte anterior.",
        )
        .await;
    let stop = receive_helper_command(&mut receiver, "capture_stop").await?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-typed-before-live-cut")?
        .context("closure absent after live cut")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "helper_capture_stop"
    );
    Ok(())
}

#[tokio::test]
async fn recovery_does_not_call_a_stale_zero_count_no_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "stale-count".into(),
        title: "Stale count".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 61_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Interrupted,
        sealed_chunks: 0,
        gaps: 1,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    runtime.fixture_store.seal_chunk(
        &orbit_store::ChunkMetadata {
            capture_epoch_id: None,
            id: "stale-count-chunk".into(),
            recording_id: "stale-count".into(),
            track: "mic_local".into(),
            sequence: 0,
            sample_rate: 8_000,
            channels: 1,
            frames: 100,
            monotonic_ns: 1_000_000,
            first_sample: 0,
            started_at_ms: 1_000,
        },
        &[1; 200],
    )?;
    runtime
        .close_empty_interrupted(orbit_core::RecordingInterruption {
            cause: orbit_core::RecordingInterruptionCause::Restart,
            detail: None,
            audio_available: false,
        })
        .await?;
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .ended_at_ms
            .is_none()
    );
    assert_eq!(
        runtime
            .fixture_store
            .list_chunks("stale-count", None, 1)?
            .len(),
        1
    );
    Ok(())
}

async fn interrupted(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<()> {
    let stream = start(runtime, receiver, "old").await?;
    runtime.native_event(frame(&stream, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    loop {
        let command = receiver.recv().await.context("privacy stop absent")?;
        if command["command"] == "privacy_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":{"privacy_stopped":true,"drain_complete":true}}))
                .await?;
            break;
        }
    }
    settle_seal_receipts(runtime).await?;
    // `fail_closed` goes through the core decision, so the
    // `StopPlayback` effect arrives at the helper right after the privacy
    // stop. Drain the trailing commands before the next start so the
    // helper pipe stays quiet.
    while receiver.try_recv().is_ok() {}
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    Ok(())
}

fn fail_second_write(runtime: &Runtime) -> Result<rusqlite::Connection> {
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    // Only the second write fails, after the same connection sees the closure
    // INSERT. No job INSERT or audio receipt write is blocked by this fixture.
    database.execute_batch(
        "CREATE TRIGGER synthetic_snapshot_failure BEFORE UPDATE ON settings
        WHEN OLD.key = 'snapshot' AND EXISTS (SELECT 1 FROM jobs WHERE id = 'finalize-old')
        BEGIN SELECT RAISE(ABORT, 'synthetic second write failure'); END;",
    )?;
    Ok(database)
}

async fn finish(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    stream: &str,
) -> Result<()> {
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receive_helper_command(receiver, "audio_stop").await?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
        "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    // The confirmation seals the tail, and the recording closes when
    // that seal's receipt lands, which is where a closure failure surfaces.
    settle_seal_receipts(runtime).await
}

fn acknowledge_uploads(runtime: &mut Runtime, recording: &str) -> Result<u64> {
    let chunks = runtime.fixture_store.list_chunks(recording, None, 10)?;
    for chunk in &chunks {
        runtime.fixture_store.update_job(
            &format!("upload-{}", chunk.metadata.id),
            JobState::Succeeded,
        )?;
        assert_eq!(
            &runtime.fixture_store.read_chunk(&chunk.metadata.id)?[44..],
            vec![1; chunk.metadata.frames as usize * 2]
        );
    }
    Ok(chunks.iter().map(|chunk| chunk.metadata.frames).sum())
}

#[tokio::test]
async fn replacement_snapshot_failure_rolls_back_closure_and_resume_closes_full_audio() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    interrupted(&mut runtime, &mut receiver).await?;
    let previous = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("snapshot absent")?;
    let database = fail_second_write(&runtime)?;
    assert!(
        start(&mut runtime, &mut receiver, "failed-next")
            .await
            .is_err()
    );
    assert_eq!(runtime.state, previous);
    assert_eq!(
        runtime.fixture_store.load_snapshot::<State>()?,
        Some(previous)
    );
    assert!(
        receiver.try_recv().is_err(),
        "no capture before the snapshot commits"
    );
    assert!(
        runtime.fixture_store.get_job("finalize-old")?.is_none(),
        "a rolled-back replacement must not leave a premature immutable closure"
    );
    database.execute_batch("DROP TRIGGER synthetic_snapshot_failure")?;

    runtime.apply(Event::ResumeMeeting).await?;
    let resumed = start_ack(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&resumed, 100)).await?;
    finish(&mut runtime, &mut receiver, &resumed).await?;
    assert_eq!(acknowledge_uploads(&mut runtime, "old")?, 200);
    let closure = runtime
        .fixture_store
        .get_job("finalize-old")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["manifest"]["tracks"][0]["total_frames"],
        200
    );
    assert!(runtime.finalize_is_ready(&closure)?);

    let fresh = start(&mut runtime, &mut receiver, "valid-next").await?;
    runtime.native_event(frame(&fresh, 50)).await?;
    finish(&mut runtime, &mut receiver, &fresh).await?;
    assert_eq!(acknowledge_uploads(&mut runtime, "valid-next")?, 50);
    let recovered = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("snapshot absent")?
        .recovered();
    drop(runtime);
    let (mut runtime, receiver) = fixture(directory.path())?;
    runtime.state = recovered;
    // The restart invents no stop; nothing restarts capture either.
    assert!(!runtime.state.privacy_stopped());
    assert!(
        receiver.is_empty(),
        "reopening must not restart native capture"
    );
    assert_eq!(
        runtime.fixture_store.get_job("finalize-old")?,
        Some(closure.clone())
    );
    assert!(runtime.finalize_is_ready(&closure)?);
    assert_eq!(acknowledge_uploads(&mut runtime, "old")?, 200);
    let fresh_closure = runtime
        .fixture_store
        .get_job("finalize-valid-next")?
        .context("new closure absent")?;
    assert!(runtime.finalize_is_ready(&fresh_closure)?);
    Ok(())
}

#[tokio::test]
async fn existing_closure_fences_resume_after_snapshot_failure_and_restart() -> Result<()> {
    for state in [JobState::Running, JobState::Succeeded] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        interrupted(&mut runtime, &mut receiver).await?;
        let old = runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("old identity absent")?
            .clone();
        runtime
            .queue_capture_finalize("old", old.started_at_ms, super::super::now(), old.gaps + 1)
            .await?;
        runtime.fixture_store.update_job("finalize-old", state)?;
        let closure = runtime
            .fixture_store
            .get_job("finalize-old")?
            .context("closure absent")?;
        let database = fail_second_write(&runtime)?;
        assert!(
            start(&mut runtime, &mut receiver, "failed-next")
                .await
                .is_err()
        );
        assert_eq!(
            runtime.fixture_store.get_job("finalize-old")?,
            Some(closure.clone())
        );
        database.execute_batch("DROP TRIGGER synthetic_snapshot_failure")?;
        assert_eq!(
            runtime
                .apply(Event::ResumeMeeting)
                .await
                .err()
                .context("closed recording unexpectedly resumed")?
                .downcast_ref::<orbit_core::refusal::Refusal>(),
            Some(&orbit_core::refusal::Refusal::InvalidParameter {
                parameter: orbit_core::refusal::Parameter::Request,
                expected: orbit_core::refusal::Expected::OpenRecording
            })
        );
        assert!(
            receiver.try_recv().is_err(),
            "an existing closure must prevent audio_start"
        );

        let recovered = runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?
            .recovered();
        drop(runtime);
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        // Native permissions are freshly observed by the fixture, never inferred
        // from the snapshot. Only an explicit local rearm grants capture again.
        let observed_capabilities = runtime.state.capabilities().clone();
        runtime.state = recovered;
        {
            let fixture_value = serde_json::json!(observed_capabilities);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities",
                fixture_value,
            );
        }
        runtime.native_event(json!({"event":"local_rearm"})).await?;
        assert_eq!(
            runtime
                .apply(Event::ResumeMeeting)
                .await
                .err()
                .context("closed recording unexpectedly resumed after recovery")?
                .downcast_ref::<orbit_core::refusal::Refusal>(),
            Some(&orbit_core::refusal::Refusal::InvalidParameter {
                parameter: orbit_core::refusal::Parameter::Request,
                expected: orbit_core::refusal::Expected::OpenRecording
            })
        );
        assert!(receiver.try_recv().is_err());
        let fresh = start(&mut runtime, &mut receiver, "valid-next").await?;
        runtime.native_event(frame(&fresh, 50)).await?;
        finish(&mut runtime, &mut receiver, &fresh).await?;
        assert_eq!(
            runtime.fixture_store.get_job("finalize-old")?,
            Some(closure)
        );
        assert_eq!(acknowledge_uploads(&mut runtime, "old")?, 100);
        assert_eq!(acknowledge_uploads(&mut runtime, "valid-next")?, 50);
    }
    Ok(())
}

#[tokio::test]
async fn unauthenticated_or_misbound_closure_cannot_grant_a_capture_restart() -> Result<()> {
    for corrupt in ["ciphertext", "state", "identity"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        interrupted(&mut runtime, &mut receiver).await?;
        if corrupt == "identity" {
            runtime.fixture_store.enqueue_job(&Job {
                id: "finalize-old".into(),
                kind: "recording_finalize".into(),
                state: JobState::Succeeded,
                created_at: 1,
                payload: json!({"recording_id":"foreign"}),
            })?;
        } else {
            runtime.queue_capture_finalize("old", 1, 2, 1).await?;
            let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
            database.execute_batch(if corrupt == "ciphertext" {
                "UPDATE jobs SET payload = zeroblob(length(payload)) WHERE id = 'finalize-old'"
            } else {
                "UPDATE jobs SET state = 'succeeded' WHERE id = 'finalize-old'"
            })?;
        }
        let previous = runtime.state.clone();
        let error = runtime
            .apply(Event::ResumeMeeting)
            .await
            .err()
            .context("corrupt closure permitted resume")?;
        assert!(
            error
                .downcast_ref::<orbit_core::refusal::Refusal>()
                .is_none(),
            "corruption is not an authenticated ordinary CaptureClosed refusal"
        );
        assert_eq!(runtime.state, previous);
        assert!(receiver.try_recv().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn finish_under_privacy_stop_rearm_and_new_meeting_leaves_a_durable_closure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver, "first").await?;
    runtime.native_event(frame(&stream, 100)).await?;
    // Privacy brake, confirmed by the Mac: audio drains and the recording
    // stays Interrupted with privacy still stopped.
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    loop {
        let command = receiver.recv().await.context("privacy stop absent")?;
        if command["command"] == "privacy_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":{"privacy_stopped":true,"drain_complete":true}}))
                .await?;
            break;
        }
    }
    settle_seal_receipts(&mut runtime).await?;
    // The core's `StopPlayback` effect follows the privacy stop;
    // drain the trailing commands before the next capture so the helper
    // pipe stays quiet.
    while receiver.try_recv().is_ok() {}
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    // The person closes the interrupted meeting while privacy stays stopped.
    runtime.apply(Event::FinishMeeting).await?;
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    // A fresh meeting must be accepted, not refused as an active recording.
    start(&mut runtime, &mut receiver, "second").await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-first")?
        .context("durable closure for the first recording absent")?;
    assert_eq!(closure.kind, "recording_finalize");
    assert_eq!(closure.payload["recording_id"], "first");
    Ok(())
}

/// Deciding whether a closure may run is two counts, not a walk
/// that decrypts every chunk manifest and every upload receipt. Two thousand
/// chunks make any per-chunk read impossible to miss in the decrypt budget.
#[tokio::test]
async fn finalize_readiness_does_not_decrypt_chunks_or_upload_receipts() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    const CHUNKS: u64 = 2000;
    for sequence in 0..CHUNKS {
        runtime.fixture_store.seal_chunk(
            &orbit_store::ChunkMetadata {
                capture_epoch_id: None,
                id: format!("readiness-budget-mic_local-{sequence}"),
                recording_id: "readiness-budget".into(),
                track: "mic_local".into(),
                sequence,
                sample_rate: 8000,
                channels: 1,
                frames: 1,
                monotonic_ns: sequence,
                first_sample: sequence,
                started_at_ms: 1,
            },
            b"synthetic-audio",
        )?;
        runtime.fixture_store.update_job(
            &format!("upload-readiness-budget-mic_local-{sequence}"),
            JobState::Succeeded,
        )?;
    }
    runtime
        .queue_capture_finalize("readiness-budget", 1, 2, 0)
        .await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-readiness-budget")?
        .context("closure absent")?;
    let before = runtime.reader.decryption_count();
    assert!(
        runtime.finalize_is_ready(&closure)?,
        "every fixture upload succeeded, so the closure must be ready"
    );
    let cost = runtime.reader.decryption_count() - before;
    assert!(
        cost <= 16,
        "readiness for {CHUNKS} chunks decrypted {cost} records; the decision must be counts, not chunk reads"
    );
    Ok(())
}
