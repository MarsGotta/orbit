use super::*;
use crate::daemon::turn::said;
use crate::daemon::{
    CaptureRuntime, ConversationRuntime, DurableSnapshot, FeatureRuntime, WorkRuntime,
};
use crate::{
    capture::CaptureBuffer,
    config::{Paths, Settings},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::{Connection, State};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[path = "actor_tests.rs"]
pub(in crate::daemon) mod actor;

#[path = "review_tests.rs"]
mod review;

#[path = "call_tests.rs"]
pub(in crate::daemon) mod call;

/// Occupy one ticketed slot with a pending worker and return its ticket, for
/// tests that deliver the matching work event by hand.
pub(in crate::daemon) fn pending_work<T>(
    work: &mut crate::daemon::ticketed::Ticketed<T>,
    generation: u64,
    value: T,
) -> Result<crate::daemon::ticketed::WorkTicket> {
    work.start_logical(generation, orbit_ipc::new_op(), value, |_| {
        tokio::spawn(std::future::pending())
    })
    .context("the test worker slot was already occupied")
}

pub(in crate::daemon) fn fixture(
    directory: &std::path::Path,
) -> Result<(Runtime, mpsc::Receiver<Value>)> {
    let paths = Paths::resolve(Some(directory.to_owned()))?;
    let store = crate::daemon::test_store::open(&paths.store, [0x42; 32])?;
    let (native, receiver) = mpsc::channel(64);
    let (hub_events, _) = mpsc::channel(16);
    // The production capacity: tests that reason about wake-up coalescing
    // drain against the real bound, not a coincidental twin.
    let (work_events, _) = mpsc::channel(crate::daemon::WORK_EVENTS_CAPACITY);
    let (notes_events, _) = mpsc::channel(crate::daemon::recording_notes::NOTE_EVENTS);
    let store_actor = crate::daemon::store_actor::StoreActor::spawn(store, work_events.clone())?;
    let fixture_store = crate::daemon::test_store::open_existing(&paths.store, [0x42; 32])?;
    let reader = crate::daemon::store_actor::open_reader(&paths.store, [0x42; 32])?;
    let voice_audio_reader = std::sync::Arc::new(std::sync::Mutex::new(
        crate::daemon::store_actor::open_reader(&paths.store, [0x42; 32])?,
    ));
    let mut state = State::default();
    crate::daemon::authority_fixture::set(&mut state, "connection", json!(Connection::Online));
    crate::daemon::authority_fixture::set(&mut state, "capabilities.native_helper", json!(true));
    crate::daemon::authority_fixture::set(
        &mut state,
        "capabilities.microphone_state",
        json!(orbit_core::PermissionState::Granted),
    );
    crate::daemon::authority_fixture::set(
        &mut state,
        "capabilities.screen_state",
        json!(orbit_core::PermissionState::Granted),
    );
    let durable_snapshot = DurableSnapshot::from(&state);
    Ok((
        Runtime {
            closing: false,
            privacy_retirement_owed: false,
            paths,
            settings: Settings::default(),
            development: true,
            // None on purpose: a test run must never write into the real
            // profile's log directory.
            diagnostics: None,
            report_log: None,
            // No client: a test that wants one points it at a fake Sentry.
            error_reports: Default::default(),
            turn: crate::daemon::turn::Turn::fresh(orbit_core::notice::ReportSource::Daemon),
            state,
            reader,
            voice_audio_reader,
            fixture_store,
            store_actor,
            native,
            hub: super::super::hub_link::HubLink::default(),
            hub_protocol_version: None,
            unrecognized_hub_events: Default::default(),
            hub_task: None,
            snapshot_revision: None,
            hub_events,
            pending_native: BTreeMap::new(),
            token: None,
            work: WorkRuntime {
                worker: crate::daemon::ticketed::Ticketed::logical(),
                retry_gate: crate::retry::RetryGate::default(),
                upload_cursor: None,
                finalize_cursor: None,
                prefer_finalize: true,
                work_events,
                durable_snapshot,
                force_snapshot_once: false,
                finalize_ready: BTreeMap::new(),
            },
            conversation: ConversationRuntime {
                bridge_request: None,
                interaction_response: None,
                interrupted_turns: Default::default(),
                voice_task: crate::daemon::ticketed::Ticketed::logical(),
                voice_operation: None,
                speech_task: crate::daemon::ticketed::Ticketed::logical(),
                speech_available: None,
                speak_next: false,
                speech_notice_call_id: None,
            },
            capture: CaptureRuntime {
                buffer: CaptureBuffer::default(),
                pending_seals: Default::default(),
                reserve_exhausted: false,
                seal_retry_at: None,
                audio: AudioLifecycle::default(),
                call_runtime: None,
                mic_stream: None,
                remote_stream: None,
                pending_starts: Vec::new(),
                retired_call_starts: BTreeMap::new(),
                retired_stops: BTreeMap::new(),
                interruption: None,
            },
            features: FeatureRuntime {
                sources: Default::default(),
                calendar: Default::default(),
                computer: Default::default(),
                meeting: Default::default(),
                call: Default::default(),
                question: Default::default(),
                native_host: Default::default(),
                notes: crate::daemon::recording_notes::NotesRuntime::for_tests(notes_events),
            },
            reads: Default::default(),
        },
        receiver,
    ))
}

/// Maintenance polls the helper's `status`. It may precede a capture order,
/// but no other unexpected command may be silently discarded by a test.
pub(in crate::daemon) fn take_helper_command(
    receiver: &mut mpsc::Receiver<Value>,
    expected: &str,
) -> Result<Value> {
    loop {
        let command = receiver
            .try_recv()
            .context("expected helper command absent")?;
        match command["command"].as_str() {
            Some("status") => continue,
            Some(name) if name == expected => return Ok(command),
            _ => bail!("expected {expected}, got {command}"),
        }
    }
}

pub(in crate::daemon) async fn receive_helper_command(
    receiver: &mut mpsc::Receiver<Value>,
    expected: &str,
) -> Result<Value> {
    loop {
        let command = receiver
            .recv()
            .await
            .context("expected helper command absent")?;
        match command["command"].as_str() {
            Some("status") => continue,
            Some(name) if name == expected => return Ok(command),
            _ => bail!("expected {expected}, got {command}"),
        }
    }
}

pub(in crate::daemon) fn assert_no_helper_command(
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<()> {
    while let Ok(command) = receiver.try_recv() {
        if command["command"] != "status" {
            bail!("unexpected helper command: {command}");
        }
    }
    Ok(())
}

/// A stall guard on a durable seal receipt, not a budget.
/// [`await_seal_receipts`] returns the instant the last receipt is applied, so
/// this number never decides a passing run; it only turns a receipt that
/// never comes into a named failure instead of a hung suite.
///
/// Measured, not chosen round. A first measurement, over 160 drains under 48
/// busy loops and 8 fsync writers on this 32-core box: p50 0 ms, p95 322 ms,
/// p99 742 ms, worst 1 306.7 ms. A second, over 2 700 of these waits with the courtesy wait at
/// zero, under 8 fsync writers and a load average of 27 to 59: p50 11.6 ms,
/// p95 499.8 ms, p99 744.5 ms, worst 11 624.9 ms; and with the store actor
/// ordered, over 2 900 waits under the same writers at a load average of 23
/// to 101: p50 76.5 ms, p99 1 943.3 ms, worst 10 006.3 ms. The guard is 40×
/// the worst of all three, the factor the first measurement used, far outside what it must
/// never arbitrate.
pub(in crate::daemon) const SEAL_RECEIPT_GUARD: Duration = Duration::from_millis(464_996);

/// Waits for the fact a sealing test asserts on: every seal already
/// handed to the store actor has been written and its receipt applied.
///
/// `seal` hands the PCM to the store actor's own thread and returns without
/// waiting for the receipt, in tests exactly as in production. This waits on
/// the receipts through `collect_ready_seals`, the collector production itself
/// runs, which returns as soon as `pending_seals` is empty. A receipt that
/// never comes is reported with the chunks still missing one and the time
/// waited.
pub(in crate::daemon) fn await_seal_receipts(runtime: &mut Runtime) -> Result<()> {
    let started = std::time::Instant::now();
    runtime.collect_ready_seals(SEAL_RECEIPT_GUARD, true)?;
    let waited = started.elapsed();
    if !runtime.capture.pending_seals.is_empty() {
        let missing: Vec<&str> = runtime
            .capture
            .pending_seals
            .iter()
            .map(|segment| segment.metadata.id.as_str())
            .collect();
        bail!(
            "expected the durable receipt of {} pending seal(s) [{}]; the queue did not empty in {waited:?}",
            missing.len(),
            missing.join(", ")
        );
    }
    Ok(())
}

/// What production does once those receipts are in. Whichever of its
/// two observers applies a receipt, the collector or `StoreWork::Sealed`, it
/// ends in `resume_deferred_drain`, which closes a recording whose
/// finish the in-flight seal had deferred.
pub(in crate::daemon) async fn settle_seal_receipts(runtime: &mut Runtime) -> Result<()> {
    await_seal_receipts(runtime)?;
    runtime.resume_deferred_drain().await
}

async fn start(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: "meeting".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let command = receiver.recv().await.context("start command absent")?;
    assert_eq!(command["command"], "audio_start");
    runtime
        .native_event(json!({"event":"response", "id":command["id"], "ok":true,
        "result":{"stream_id":command["stream_id"]}}))
        .await?;
    Ok(command["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned())
}

/// 200 ms at the 8 kHz of `frame`: a voice note long enough to send, above
/// `orbit_core::voice::MINIMUM_SPEECH_MS`.
const VOICE_NOTE_FRAMES: usize = 1_600;

fn frame(stream: &str, sequence: u64, frames: usize) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":frames, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(10_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
}

#[tokio::test]
async fn failed_seal_preserves_close_and_seals_once_after_destination_recovers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    // Model the helper's confirmed drain at the boundary immediately before
    // complete_audio_drain: this isolates the drain's own seal from the
    // earlier per-stream seal performed by the normal response handler.
    runtime.capture.audio.stops.clear();
    runtime.capture.mic_stream = None;
    let chunks_path = runtime.paths.store.join("chunks");
    std::fs::remove_dir(&chunks_path)?;
    std::fs::write(&chunks_path, b"fixture: not a directory")?;
    let Err(failure) = runtime.complete_audio_drain().await else {
        bail!("the invalid destination should have prevented the seal")
    };
    assert!(failure.to_string().contains("directorio"), "{failure}");
    assert_eq!(runtime.capture.audio.recording_end, Some(true));
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::RecordingSealFailed),
        "{:?}",
        runtime.state.durable.notice
    );
    std::fs::remove_file(&chunks_path)?;
    std::fs::create_dir(&chunks_path)?;
    runtime.retry_seals()?;
    settle_seal_receipts(&mut runtime).await?;
    let chunks = runtime.fixture_store.list_chunks("meeting", None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, 100);
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Idle
    );
    Ok(())
}

#[tokio::test]
async fn finish_waits_for_ack_and_preserves_the_final_native_frames() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Finalizing
    );
    assert!(runtime.apply(Event::VoiceStart).await.is_err());
    let stop = receiver.recv().await.context("stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    runtime.native_event(frame(&stream, 1, 100)).await?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
        "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Idle
    );
    let chunks = runtime.fixture_store.list_chunks("meeting", None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, 200);
    assert_eq!(
        runtime
            .fixture_store
            .read_chunk(&chunks[0].metadata.id)?
            .len(),
        44 + 400
    );
    assert!(!runtime.audio_is_draining());
    Ok(())
}

#[tokio::test]
async fn pause_does_not_record_microphone_frames_owned_by_voice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.apply(Event::VoiceStart).await?;
    let playback_stop = receiver.recv().await.context("playback command absent")?;
    assert_eq!(playback_stop["command"], "playback_stop");
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime.apply(Event::PauseMeeting).await?;
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(stream.as_str()));
    assert!(!runtime.audio_is_draining());
    runtime.native_event(frame(&stream, 1, 8000)).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("voice absent")?
            .pcm
            .len(),
        16_200
    );
    Ok(())
}

#[tokio::test]
async fn privacy_stop_cancels_tasks_and_drains_only_the_bound_old_stream() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    let generation = runtime.state.generation();
    pending_work(&mut runtime.conversation.voice_task, generation, ())?;
    pending_work(&mut runtime.conversation.speech_task, generation, ())?;
    let voice_abort = runtime
        .conversation
        .voice_task
        .task_handle()
        .context("voice worker absent")?
        .abort_handle();
    let speech_abort = runtime
        .conversation
        .speech_task
        .task_handle()
        .context("speech worker absent")?
        .abort_handle();
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    tokio::task::yield_now().await;
    assert!(voice_abort.is_finished() && speech_abort.is_finished());
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(runtime.conversation.speech_task.active().is_none());
    assert!(runtime.state.privacy_stopped());
    let stop = receiver.recv().await.context("privacy stop absent")?;
    assert_eq!(stop["command"], "privacy_stop");
    runtime
        .native_event(frame("another-stream", 7, 8000))
        .await?;
    runtime.native_event(frame(&stream, 1, 100)).await?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
        "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await?;
    let chunks = runtime.fixture_store.list_chunks("meeting", None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, 200);
    runtime.native_event(frame(&stream, 2, 8000)).await?;
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    assert!(runtime.capture.mic_stream.is_none());
    Ok(())
}

#[tokio::test]
async fn stale_or_missing_stream_identity_never_reaches_capture() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    let mut wrong_source = frame(&stream, 0, 8000);
    wrong_source["source"] = json!("remote_app");
    runtime.native_event(wrong_source).await?;
    let mut missing = frame(&stream, 0, 8000);
    missing
        .as_object_mut()
        .context("frame malformed")?
        .remove("stream_id");
    runtime.native_event(missing).await?;
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    Ok(())
}

#[tokio::test]
async fn voice_source_is_durable_before_credentials_or_provider_are_needed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_source: Option<String> = Some("synthetic-session".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.audio.submit_voice = true;
    runtime
        .capture
        .buffer
        .ingest(&frame("voice", 5, VOICE_NOTE_FRAMES), 1234)?;
    let Err(refusal) = runtime.dispatch_voice_transcription().await else {
        anyhow::bail!("voice dispatch unexpectedly succeeded without pairing")
    };
    assert!(matches!(
        refusal.downcast_ref::<orbit_core::refusal::Refusal>(),
        Some(orbit_core::refusal::Refusal::NotPaired)
    ));
    assert!(runtime.conversation.voice_task.active().is_none());
    let note = &runtime
        .state
        .durable
        .turn
        .as_ref()
        .context("voice note absent")?
        .user;
    assert_eq!(
        note.voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some("voice_hub_pairing_required")
    );
    let jobs = runtime.fixture_store.list_jobs(10)?;
    let manifest = jobs
        .iter()
        .find(|job| job.kind == "recording_manifest")
        .context("voice source manifest absent")?;
    assert_eq!(manifest.payload["recording_kind"], "voice_turn");
    assert_eq!(manifest.payload["session_id"], "synthetic-session");
    let recording = manifest.payload["recording_id"]
        .as_str()
        .context("recording absent")?;
    let chunks = runtime.fixture_store.list_chunks(recording, None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, VOICE_NOTE_FRAMES as u64);
    assert_eq!(chunks[0].metadata.started_at_ms, 1234);
    assert_eq!(chunks[0].metadata.monotonic_ns, 50_000);
    let chunk = chunks[0].metadata.id.clone();
    let store_path = runtime.paths.store.clone();
    drop(runtime);
    let reopened = crate::daemon::test_store::open(store_path, [0x42; 32])?;
    assert_eq!(
        reopened.read_chunk(&chunk)?.len(),
        44 + 2 * VOICE_NOTE_FRAMES
    );
    Ok(())
}

/// A voice note is sent to `/v1/audio/transcribe` and kept on the
/// Mac for playback; nothing on the hub reads its audio, which it listed as
/// a meeting that never closed. Its chunk is never uploaded.
#[tokio::test]
async fn a_voice_note_is_not_uploaded_as_a_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.audio.submit_voice = true;
    runtime
        .capture
        .buffer
        .ingest(&frame("voice", 5, VOICE_NOTE_FRAMES), 1234)?;
    let _unpaired = runtime.dispatch_voice_transcription().await;
    let jobs = runtime.fixture_store.list_jobs(10)?;
    let manifest = jobs
        .iter()
        .find(|job| job.kind == "recording_manifest")
        .context("voice source manifest absent")?;
    assert_eq!(manifest.payload["recording_kind"], "voice_turn");
    assert_eq!(manifest.payload["upload_allowed"], false);
    let chunk = manifest.payload["source_chunk_id"]
        .as_str()
        .context("chunk absent")?;
    let upload = runtime
        .fixture_store
        .get_job(&format!("upload-{chunk}"))?
        .context("upload job absent")?;
    assert_eq!(upload.state, JobState::Cancelled);
    // Kept on the Mac for playback.
    assert_eq!(
        runtime.fixture_store.read_chunk(chunk)?.len(),
        44 + 2 * VOICE_NOTE_FRAMES
    );
    Ok(())
}

#[tokio::test]
async fn occupied_voice_worker_returns_typed_capacity_with_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.token = Some("synthetic".into());
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.audio.submit_voice = true;
    runtime
        .capture
        .buffer
        .ingest(&frame("voice", 5, VOICE_NOTE_FRAMES), 1234)?;
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    let Err(refusal) = runtime.dispatch_voice_transcription().await else {
        anyhow::bail!("voice dispatch unexpectedly reserved an occupied slot")
    };
    assert!(matches!(
        refusal.downcast_ref::<orbit_core::refusal::Refusal>(),
        Some(orbit_core::refusal::Refusal::WorkInFlight {
            in_flight: 1,
            max: 1
        })
    ));
    Ok(())
}

#[tokio::test]
async fn an_empty_transcription_fails_the_voice_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-message".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-message".into());
    let ticket = pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime
        .work_event(WorkResult::Capture(
            crate::daemon::CaptureWork::Transcribed {
                ticket,
                source: VoiceSource {
                    recording_id: "voice-recording".into(),
                    chunk_id: "voice-source".into(),
                    session_id: Some("session".into()),
                    message_id: "voice-message".into(),
                    duration_ms: 0,
                },
                text: String::new(),
            },
        ))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .turn
            .as_ref()
            .context("turn absent")?
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some("voice_transcription_empty")
    );
    Ok(())
}

#[tokio::test]
async fn late_transcript_after_privacy_stop_is_not_submitted() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    let original_turn = runtime.state.durable.turn.clone();
    let ticket = pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime
        .work_event(WorkResult::Capture(
            crate::daemon::CaptureWork::Transcribed {
                ticket,
                source: VoiceSource {
                    recording_id: "late-voice".into(),
                    chunk_id: "late-source".into(),
                    session_id: None,
                    message_id: "late-message".into(),
                    duration_ms: 0,
                },
                text: "SYNTHETIC-LATE-RESULT-DO-NOT-SUBMIT".into(),
            },
        ))
        .await?;
    assert_eq!(runtime.state.durable.turn, original_turn);
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(!runtime.conversation.speak_next);
    Ok(())
}

#[tokio::test]
async fn generation_change_marks_the_pending_note_instead_of_leaving_it_in_progress() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-generation".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-generation".into());
    let started_generation = runtime.state.generation();
    let ticket = pending_work(&mut runtime.conversation.voice_task, started_generation, ())?;
    {
        let fixture_value = serde_json::json!(runtime.state.generation().saturating_add(1));
        crate::daemon::authority_fixture::set(&mut runtime.state, "generation", fixture_value);
    }
    runtime
        .work_event(WorkResult::Capture(
            crate::daemon::CaptureWork::VoiceProblem {
                ticket,
                operation_id: "voice-generation".into(),
                failure: orbit_core::VoiceTranscriptionFailure::local("synthetic_late_failure"),
            },
        ))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .turn
            .as_ref()
            .context("turn absent")?
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some("voice_transcription_generation_changed")
    );
    Ok(())
}

#[tokio::test]
async fn interrupt_invalidates_an_already_queued_voice_result_without_stopping_meeting()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    let generation = runtime.state.generation();
    runtime.conversation.voice_operation = Some("voice-message".into());
    pending_work(&mut runtime.conversation.voice_task, generation, ())?;
    let abort = runtime
        .conversation
        .voice_task
        .task_handle()
        .context("voice worker absent")?
        .abort_handle();
    runtime
        .work_event(WorkResult::Problem {
            ticket: crate::daemon::ticketed::WorkTicket::test(generation),
            message: "Synthetic TTS error".into(),
        })
        .await?;
    assert!(runtime.conversation.voice_task.active().is_some());
    runtime.command(crate::daemon::Command::Interrupt).await?;
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    assert!(runtime.conversation.voice_task.active().is_none());
    assert_eq!(runtime.state.generation(), generation);
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(stream.as_str()));
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Recording
    );
    runtime
        .work_event(WorkResult::Capture(
            crate::daemon::CaptureWork::Transcribed {
                ticket: crate::daemon::ticketed::WorkTicket::test(generation),
                source: VoiceSource {
                    recording_id: "cancelled-voice".into(),
                    chunk_id: "source".into(),
                    session_id: None,
                    message_id: "voice-message".into(),
                    duration_ms: 0,
                },
                text: "SYNTHETIC-NOT-SUBMITTED".into(),
            },
        ))
        .await?;
    assert!(runtime.state.durable.turn.is_none());
    Ok(())
}

#[tokio::test]
async fn interrupt_before_voice_finish_preserves_source_without_submitting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.audio.submit_voice = true;
    runtime.capture.buffer.ingest(&frame("voice", 0, 100), 5)?;
    runtime
        .cancel_voice_transcription(VoiceCancellationCause::Authority(
            orbit_core::authority::AuthorityChange::Interrupt,
        ))
        .await?;
    runtime.dispatch_voice_transcription().await?;
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(runtime.conversation.voice_operation.is_none());
    Ok(())
}

#[tokio::test]
async fn system_scope_starts_remote_app_without_pid_and_marks_the_epoch() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "meeting".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_ms: 60_000,
        })
        .await?;
    let mut remote_start = None;
    for _ in 0..2 {
        let command = receiver.recv().await.context("start command absent")?;
        assert_eq!(command["command"], "audio_start");
        if command["source"] == "remote_app" {
            remote_start = Some(command.clone());
        }
        runtime
            .native_event(json!({"event":"response", "id":command["id"], "ok":true,
            "result":{"stream_id":command["stream_id"]}}))
            .await?;
    }
    let remote_start = remote_start.context("remote_app start absent")?;
    assert_eq!(remote_start["source"], "remote_app");
    assert_eq!(remote_start["capture_scope"], "system");
    assert!(
        remote_start.get("application_pid").is_none(),
        "the system scope does not travel with a PID: {remote_start}"
    );
    let epoch = runtime
        .fixture_store
        .list_jobs(10)?
        .into_iter()
        .find(|job| job.kind == "capture_epoch")
        .context("capture epoch job absent")?;
    assert_eq!(
        epoch.payload["expected_tracks"],
        json!(["mic_local", "remote_app"])
    );
    assert_eq!(epoch.payload["capture_scope"], "system");
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Recording
    );
    Ok(())
}

#[test]
fn meeting_start_json_without_audio_defaults_to_system_and_a_stray_pid_is_rejected() {
    let plain = serde_json::from_value::<orbit_core::protocol::Command>(json!({
        "command": "meeting_start", "title": "Reunión", "duration_minutes": 60
    }));
    assert!(
        matches!(
            plain,
            Ok(orbit_core::protocol::Command::MeetingStart {
                audio: orbit_core::protocol::MeetingAudio::System,
                ..
            })
        ),
        "without an audio field the order must read as system scope"
    );
    let stray_pid = serde_json::from_value::<orbit_core::protocol::Command>(json!({
        "command": "meeting_start", "title": "Reunión", "duration_minutes": 60,
        "application_pid": 42
    }));
    assert!(
        stray_pid.is_err(),
        "application_pid is no longer a field of meeting_start"
    );
}

fn track_progress<'a>(runtime: &'a Runtime, name: &str) -> Result<&'a orbit_core::TrackProgress> {
    runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?
        .tracks
        .iter()
        .find(|track| track.name == name)
        .context("track absent")
}

/// A two-track recording publishes its progress per track. Sealed
/// chunks and the wall clock of the last one belong to each track, and a gap
/// the helper counts lands on the track that reported it, never on the other.
#[tokio::test]
async fn tracks_report_sealed_chunks_and_gaps_per_track() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "two-tracks".into(),
            title: "Reunión con dos pistas".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_ms: 60_000,
        })
        .await?;
    let mut microphone = None;
    let mut remote = None;
    for _ in 0..2 {
        let command = receiver.recv().await.context("start command absent")?;
        assert_eq!(command["command"], "audio_start");
        runtime
            .native_event(json!({"event":"response","id":command["id"],"ok":true,
            "result":{"stream_id":command["stream_id"]}}))
            .await?;
        if command["source"] == "remote_app" {
            remote = Some(command);
        } else {
            microphone = Some(command);
        }
    }
    let microphone_stream = microphone.context("microphone start absent")?["stream_id"]
        .as_str()
        .context("microphone stream absent")?
        .to_owned();
    let remote_stream = remote.context("remote start absent")?["stream_id"]
        .as_str()
        .context("remote stream absent")?
        .to_owned();
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?
        .id
        .clone();
    assert_eq!(
        track_progress(&runtime, "mic_local")?.sealed_chunks,
        0,
        "nothing was captured yet"
    );

    // One second of microphone audio seals exactly one microphone chunk.
    runtime
        .native_event(frame(&microphone_stream, 0, 8000))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let microphone_first = track_progress(&runtime, "mic_local")?.last_chunk_at_ms;
    assert_eq!(track_progress(&runtime, "mic_local")?.sealed_chunks, 1);
    assert_eq!(track_progress(&runtime, "remote_app")?.sealed_chunks, 0);
    assert!(
        microphone_first > 0,
        "the last sealed chunk carries its wall clock"
    );

    // A later seal of the same track advances that track's clock alone.
    tokio::time::sleep(Duration::from_millis(25)).await;
    runtime
        .native_event(frame(&microphone_stream, 1, 8000))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let microphone_second = track_progress(&runtime, "mic_local")?.last_chunk_at_ms;
    assert_eq!(track_progress(&runtime, "mic_local")?.sealed_chunks, 2);
    assert_eq!(track_progress(&runtime, "remote_app")?.sealed_chunks, 0);
    assert!(
        microphone_second > microphone_first,
        "the last sealed chunk did not advance: {microphone_first} → {microphone_second}"
    );
    let sealed = runtime.fixture_store.list_chunks(&recording, None, 10)?;
    let mut microphone_started = sealed
        .iter()
        .filter(|chunk| chunk.metadata.track == "mic_local")
        .map(|chunk| chunk.metadata.started_at_ms)
        .collect::<Vec<_>>();
    microphone_started.sort_unstable();
    assert_eq!(microphone_started.len(), 2);
    assert_eq!(microphone_started.last(), Some(&microphone_second));

    // The remote track counts its own chunks without touching the microphone.
    let mut remote_frame = frame(&remote_stream, 0, 8000);
    remote_frame["source"] = json!("remote_app");
    runtime.native_event(remote_frame).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(track_progress(&runtime, "remote_app")?.sealed_chunks, 1);
    assert_eq!(track_progress(&runtime, "mic_local")?.sealed_chunks, 2);

    // A helper-counted gap belongs to the track that reported it.
    runtime
        .native_event(json!({"event":"gap","source":"mic_local","stream_id":microphone_stream}))
        .await?;
    assert_eq!(track_progress(&runtime, "mic_local")?.gaps, 1);
    assert_eq!(track_progress(&runtime, "remote_app")?.gaps, 0);
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .gaps,
        1
    );
    Ok(())
}

#[tokio::test]
async fn silent_call_stop_replaces_the_wait_notice() -> Result<()> {
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
            id: "synthetic-timeout-call".into(),
            duration_ms: 60_000,
        })
        .await?;
    let start = loop {
        let command = receiver.try_recv().context("audio_start absent")?;
        if command["command"] == "audio_start" {
            break command;
        }
    };
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
        "result":{"stream_id":start["stream_id"],"processing":"voice_processing",
            "processing_enabled":true,"processing_bypassed":false,
            "agc_enabled":false,"playback_reference":"shared_engine",
            "pipeline_id":"synthetic-pipeline"}}))
        .await?;
    runtime.apply(Event::CallStop).await?;
    let _stop = loop {
        let command = receiver.try_recv().context("audio_stop absent")?;
        if command["command"] == "audio_stop" {
            break command;
        }
    };
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioDrainWaiting)
    );
    runtime.capture.audio.deadline = Some(Instant::now());
    runtime.handle_audio_timeout().await;
    assert!(!runtime.audio_is_draining());
    assert_ne!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioDrainWaiting),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(
        said::mentions(&runtime.state, "audio_stop_unconfirmed"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn drain_deadline_keeps_an_unsent_capture_stop_owed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.capture.mic_stream = Some("synthetic-open-mic".into());
    let (blocked, _occupied) = mpsc::channel(1);
    blocked.try_send(json!({"fixture":"occupied"}))?;
    runtime.native = blocked;
    assert!(
        runtime
            .request_full_audio_stop(FullStop::Cut)
            .await
            .is_err()
    );
    assert!(runtime.full_stop_send_owed());
    runtime.capture.audio.deadline = Some(Instant::now());

    runtime.handle_audio_timeout().await;
    assert!(
        runtime.full_stop_send_owed(),
        "the helper never accepted capture_stop"
    );
    assert_eq!(
        runtime.capture.mic_stream.as_deref(),
        Some("synthetic-open-mic")
    );
    let (native, mut receiver) = mpsc::channel(8);
    runtime.native = native;
    runtime.run_maintenance().await;
    let retry = take_helper_command(&mut receiver, "capture_stop")?;
    runtime
        .handle_native_event(json!({"event":"response","id":retry["id"],"ok":true,
        "result":{"drain_complete":true}}))
        .await;
    assert!(runtime.capture.mic_stream.is_none());
    Ok(())
}

#[tokio::test]
async fn lost_helper_ends_an_owed_stop_and_seals_received_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    let (blocked, _occupied) = mpsc::channel(1);
    blocked.try_send(json!({"fixture":"occupied"}))?;
    runtime.native = blocked;
    assert!(
        runtime
            .request_full_audio_stop(FullStop::Cut)
            .await
            .is_err()
    );
    assert!(runtime.full_stop_send_owed());

    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    assert!(!runtime.state.capabilities().native_helper());
    runtime.capture.audio.deadline = Some(Instant::now());
    runtime.handle_audio_timeout().await;
    settle_seal_receipts(&mut runtime).await?;

    assert!(!runtime.audio_is_draining());
    assert!(!runtime.full_stop_send_owed());
    assert!(runtime.capture.mic_stream.is_none());
    let sealed = runtime.fixture_store.list_chunks("meeting", None, 10)?;
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].metadata.frames, 100);
    assert_ne!(runtime.state.notice_code(), Some(NoticeCode::StopRetrying));
    let refusal = runtime
        .apply(Event::StartMeeting {
            id: "after-helper-loss".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await
        .err()
        .context("the missing helper must refuse the next meeting")?;
    assert_eq!(
        crate::daemon::refusal_from_error(&refusal),
        orbit_core::refusal::Refusal::HelperNotRunning
    );
    Ok(())
}

#[tokio::test]
async fn helper_lost_after_retry_notice_seals_audio_on_next_tick() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    let (blocked, _occupied) = mpsc::channel(1);
    blocked.try_send(json!({"fixture":"occupied"}))?;
    runtime.native = blocked;
    assert!(
        runtime
            .request_full_audio_stop(FullStop::Cut)
            .await
            .is_err()
    );
    runtime.capture.audio.deadline = Some(Instant::now());
    runtime.handle_audio_timeout().await;
    assert_eq!(runtime.state.notice_code(), Some(NoticeCode::StopRetrying));
    assert!(runtime.capture.audio.deadline.is_none());

    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    assert!(!runtime.state.capabilities().native_helper());
    runtime.handle_audio_timeout().await;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioStopUnconfirmed),
        "{:?}",
        runtime.state.durable.notice
    );
    // The cause, with its 500 ms, is in daemon.log under the same op.
    let told = said::told(&runtime);
    assert!(told.contains("se desconectó"), "{told}");
    assert!(told.contains("500 ms"), "{told}");
    runtime.run_maintenance().await;
    settle_seal_receipts(&mut runtime).await?;

    assert!(!runtime.audio_is_draining());
    assert!(!runtime.full_stop_send_owed());
    assert!(runtime.capture.mic_stream.is_none());
    let sealed = runtime.fixture_store.list_chunks("meeting", None, 10)?;
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].metadata.frames, 100);
    assert_ne!(runtime.state.notice_code(), Some(NoticeCode::StopRetrying));
    assert_ne!(
        runtime.state.notice_code(),
        Some(NoticeCode::PendingWorkKeepOpen)
    );
    let refusal = runtime
        .apply(Event::StartMeeting {
            id: "after-helper-loss".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await
        .err()
        .context("the missing helper must refuse the next meeting")?;
    assert_eq!(
        crate::daemon::refusal_from_error(&refusal),
        orbit_core::refusal::Refusal::HelperNotRunning
    );
    Ok(())
}

#[tokio::test]
async fn delivered_retry_rearms_drain_deadline_until_unconfirmed_cut() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    let (blocked, _occupied) = mpsc::channel(1);
    blocked.try_send(json!({"fixture":"occupied"}))?;
    runtime.native = blocked;
    assert!(
        runtime
            .request_full_audio_stop(FullStop::Cut)
            .await
            .is_err()
    );
    runtime.capture.audio.deadline = Some(Instant::now());
    runtime.handle_audio_timeout().await;
    assert!(runtime.full_stop_send_owed());
    assert!(runtime.capture.audio.deadline.is_none());
    runtime.capture.audio.started_at = Some(
        Instant::now()
            .checked_sub(Duration::from_secs(20))
            .context("a twenty-second-old drain start")?,
    );

    let (native, mut recovered) = mpsc::channel(8);
    runtime.native = native;
    runtime.retry_full_stop_send().await?;
    let delivered = recovered.try_recv().context("capture_stop retry absent")?;
    assert_eq!(delivered["command"], "capture_stop");
    assert!(!runtime.full_stop_send_owed());
    assert!(
        runtime.capture.audio.deadline.is_some(),
        "delivery must restart the drain deadline even without an ACK"
    );
    assert!(
        runtime
            .capture
            .audio
            .deadline
            .is_some_and(|deadline| deadline > Instant::now()),
        "the new drain window must not inherit a hard limit already 20 seconds old"
    );
    runtime.capture.audio.deadline = Some(Instant::now());
    runtime.handle_audio_timeout().await;
    settle_seal_receipts(&mut runtime).await?;
    assert!(!runtime.audio_is_draining());
    assert!(runtime.capture.mic_stream.is_none());
    assert_eq!(
        runtime
            .fixture_store
            .list_chunks("meeting", None, 10)?
            .len(),
        1
    );
    assert!(said::mentions(&runtime.state, "audio_stop_unconfirmed"));
    Ok(())
}

#[tokio::test]
async fn a_new_recording_clears_the_previous_reserve_loss_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.capture.reserve_exhausted = true;
    runtime
        .start_recording_audio(
            "fresh-recording".into(),
            orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        )
        .await?;
    assert_eq!(
        helper
            .recv()
            .await
            .context("helper did not receive audio_start")?["command"],
        "audio_start"
    );
    assert!(
        !runtime.capture.reserve_exhausted,
        "old recording's reserve loss survived a new capture"
    );
    Ok(())
}

#[tokio::test]
async fn privacy_stop_marks_pending_voice_transcription_as_failed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-privacy".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-privacy".into());
    let _ticket = pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;

    // Trigger privacy stop
    runtime.apply(Event::PrivacyStop).await?;

    assert_eq!(
        runtime
            .state
            .durable
            .turn
            .as_ref()
            .context("turn absent")?
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some("voice_transcription_privacy_stopped")
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    Ok(())
}

async fn assert_voice_retirement_cause(
    change: orbit_core::authority::AuthorityChange,
    expected: &str,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-retired".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-retired".into());
    runtime.retire_voice_transcription(change);
    assert_eq!(
        runtime
            .state
            .durable
            .turn
            .as_ref()
            .context("turn absent")?
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some(expected)
    );
    Ok(())
}

#[tokio::test]
async fn user_stop_names_privacy_as_the_voice_retirement_cause() -> Result<()> {
    assert_voice_retirement_cause(
        orbit_core::authority::AuthorityChange::UserStop,
        "voice_transcription_privacy_stopped",
    )
    .await
}

#[tokio::test]
async fn voice_cancellation_cause_is_persisted_by_each_entry_point() -> Result<()> {
    for entry in ["interrupt", "voice_interrupt", "close"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        runtime
            .apply(Event::VoiceStored {
                id: "voice-cancelled".into(),
                duration_ms: 1_000,
            })
            .await?;
        runtime.conversation.voice_operation = Some("voice-cancelled".into());
        // `..._by_mars`: wire/stored name kept for compatibility with 0.9.0
        let expected = match entry {
            "interrupt" => {
                runtime.apply(Event::Interrupt).await?;
                "voice_transcription_interrupted_by_mars"
            }
            "voice_interrupt" => {
                runtime
                    .cancel_voice_transcription(VoiceCancellationCause::Authority(
                        orbit_core::authority::AuthorityChange::Interrupt,
                    ))
                    .await?;
                "voice_transcription_interrupted_by_mars"
            }
            "close" => {
                runtime
                    .apply(Event::Close {
                        cause: "Cierre sintético".into(),
                    })
                    .await?;
                "voice_transcription_orbit_closing"
            }
            _ => unreachable!("fixture entry"),
        };
        let saved = runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?;
        assert_eq!(
            saved
                .durable
                .turn
                .as_ref()
                .context("turn absent")?
                .user
                .voice
                .as_ref()
                .and_then(|voice| voice.failure.as_ref())
                .map(|failure| failure.code.as_str()),
            Some(expected),
            "{entry}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn session_device_and_revocation_retire_the_pending_voice_note() -> Result<()> {
    for event in [
        Event::Session("new-session".into()),
        Event::DeviceChanged,
        Event::Connect(Connection::Revoked),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        {
            let fixture_source: Option<String> = Some("old-session".into());
            let fixture_value = serde_json::json!(fixture_source);
            crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
        }
        runtime
            .apply(Event::VoiceStored {
                id: "voice-change".into(),
                duration_ms: 1_000,
            })
            .await?;
        runtime.conversation.voice_operation = Some("voice-change".into());
        pending_work(
            &mut runtime.conversation.voice_task,
            runtime.state.generation(),
            (),
        )?;
        runtime.apply(event.clone()).await?;
        assert!(
            runtime.conversation.voice_task.active().is_none(),
            "{event:?}"
        );
        let saved = runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?;
        assert_eq!(
            saved
                .durable
                .turn
                .as_ref()
                .context("turn absent")?
                .user
                .voice
                .as_ref()
                .and_then(|voice| voice.failure.as_ref())
                .map(|failure| failure.code.as_str()),
            Some("voice_transcription_generation_changed"),
            "{event:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn hub_link_flicker_keeps_the_pending_voice_worker_and_note() -> Result<()> {
    for events in [
        vec![
            Event::Connect(Connection::Connecting),
            Event::Connect(Connection::Online),
        ],
        vec![Event::Connect(Connection::Offline)],
        vec![Event::HubDisconnected],
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        runtime
            .apply(Event::VoiceStored {
                id: "voice-link-flicker".into(),
                duration_ms: 1_000,
            })
            .await?;
        runtime.conversation.voice_operation = Some("voice-link-flicker".into());
        pending_work(
            &mut runtime.conversation.voice_task,
            runtime.state.generation(),
            (),
        )?;
        let generation = runtime.state.generation();
        for event in events {
            runtime.apply(event.clone()).await?;
            assert_eq!(runtime.state.generation(), generation, "{event:?}");
            assert!(
                runtime.conversation.voice_task.active().is_some(),
                "{event:?}"
            );
            assert_eq!(
                runtime.conversation.voice_operation.as_deref(),
                Some("voice-link-flicker"),
                "{event:?}"
            );
            assert!(
                runtime
                    .state
                    .durable
                    .turn
                    .as_ref()
                    .context("turn absent")?
                    .user
                    .voice
                    .as_ref()
                    .and_then(|voice| voice.failure.as_ref())
                    .is_none(),
                "{event:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn hub_link_change_cancels_a_call_turn_but_keeps_a_saved_voice_note() -> Result<()> {
    for event in [
        Event::Connect(Connection::Connecting),
        Event::Connect(Connection::Offline),
        Event::HubDisconnected,
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        runtime
            .apply(Event::VoiceStored {
                id: "voice-call-link".into(),
                duration_ms: 1_000,
            })
            .await?;
        runtime.conversation.voice_operation = Some("voice-call-link".into());
        let ticket = pending_work(
            &mut runtime.conversation.voice_task,
            runtime.state.generation(),
            (),
        )?;
        runtime.state.durable.call = Some(orbit_core::CallState {
            id: "call-link".into(),
            phase: orbit_core::CallPhase::Listening,
            started_at_ms: 1,
            expires_at_ms: i64::MAX,
            turns: 0,
        });
        runtime.apply(event.clone()).await?;
        assert!(runtime.state.durable.call.is_none(), "{event:?}");
        assert!(
            runtime.conversation.voice_task.active().is_none(),
            "{event:?}"
        );
        assert_eq!(
            runtime.conversation.voice_operation.as_deref(),
            None,
            "{event:?}"
        );
        assert_eq!(
            runtime
                .state
                .durable
                .turn
                .as_ref()
                .context("turn absent")?
                .user
                .voice
                .as_ref()
                .and_then(|voice| voice.failure.as_ref())
                .map(|failure| failure.code.as_str()),
            Some("voice_transcription_generation_changed"),
            "{event:?}"
        );
        runtime
            .work_event(WorkResult::Capture(
                crate::daemon::CaptureWork::Transcribed {
                    ticket,
                    source: VoiceSource {
                        recording_id: "late-call-link".into(),
                        chunk_id: "late-call-source".into(),
                        session_id: None,
                        message_id: "voice-call-link".into(),
                        duration_ms: 1_000,
                    },
                    text: "late call turn".into(),
                },
            ))
            .await?;
        assert!(!runtime.conversation.speak_next);

        let directory = tempfile::tempdir()?;
        let (mut note_runtime, _receiver) = fixture(directory.path())?;
        note_runtime
            .apply(Event::VoiceStored {
                id: "saved-note-link".into(),
                duration_ms: 1_000,
            })
            .await?;
        note_runtime.conversation.voice_operation = Some("saved-note-link".into());
        pending_work(
            &mut note_runtime.conversation.voice_task,
            note_runtime.state.generation(),
            (),
        )?;
        note_runtime.apply(event.clone()).await?;
        assert!(
            note_runtime.conversation.voice_task.active().is_some(),
            "{event:?}"
        );
        assert!(
            note_runtime
                .state
                .durable
                .turn
                .as_ref()
                .context("turn absent")?
                .user
                .voice
                .as_ref()
                .and_then(|voice| voice.failure.as_ref())
                .is_none(),
            "{event:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn meeting_capture_cut_keeps_a_saved_voice_note_transcription() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "saved-note-during-meeting".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("saved-note-during-meeting".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: "meeting-capture".into(),
        title: "Reunión sintética".into(),
        started_at_ms: now(),
        ended_at_ms: None,
        expires_at_ms: i64::MAX,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        phase: orbit_core::CapturePhase::Recording,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    assert!(runtime.state.durable.call.is_none());
    runtime
        .cut_capture(
            orbit_core::privacy_stop::StopCause::HelperLost,
            "helper_lost",
            "Corte sintético de la reunión",
        )
        .await;
    assert!(runtime.conversation.voice_task.active().is_some());
    assert_eq!(
        runtime.conversation.voice_operation.as_deref(),
        Some("saved-note-during-meeting")
    );
    assert!(
        runtime
            .state
            .durable
            .turn
            .as_ref()
            .context("turn absent")?
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn refused_voice_start_keeps_the_permission_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.apply(Event::VoiceStart).await?;
    let playback_stop = receiver.try_recv().context("playback_stop absent")?;
    assert_eq!(playback_stop["command"], "playback_stop");
    let start = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":false,
            "error":{"code":"microphone_permission_required",
                "message":"Falta el permiso de micrófono de macOS"}}))
        .await?;

    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::MicrophonePermissionMissing),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(!said::has(&runtime.state, NoticeCode::VoiceNoteEmpty));
    assert!(!runtime.state.durable.voice_active);
    assert!(runtime.capture.buffer.voice.is_none());
    Ok(())
}

#[tokio::test]
async fn pending_audio_warns_only_during_exit() -> Result<()> {
    for closing in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        said::trace(&mut runtime, directory.path());
        let stream = start(&mut runtime, &mut receiver).await?;
        runtime.native_event(frame(&stream, 0, 100)).await?;
        if closing {
            runtime
                .close_for_exit(
                    orbit_core::privacy_stop::StopCause::Shutdown,
                    "Cierre sintético",
                )
                .await;
        } else {
            runtime
                .cut_capture(
                    orbit_core::privacy_stop::StopCause::CaptureEventFailed,
                    "audio_device_changed_voice_note",
                    "Cambio sintético de entrada",
                )
                .await;
        }
        assert!(
            !runtime.has_any_pending().is_empty(),
            "fixture must retain unconfirmed audio"
        );
        assert_eq!(
            said::has(&runtime.state, NoticeCode::PendingWorkKeepOpen),
            closing,
            "closing={closing}: {:?}",
            runtime.state.durable.notice_log.records()
        );
        let logged = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
        assert!(logged.contains("pending_in_memory"), "{logged}");
    }
    Ok(())
}

#[tokio::test]
async fn helper_knows_who_uses_a_shared_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "purpose-meeting".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let meeting = take_helper_command(&mut helper, "audio_start")?;
    assert_eq!(meeting["purpose"], "meeting");
    let stream = meeting["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":meeting["id"],"ok":true,
        "result":{"stream_id":stream}}))
        .await?;
    // Drain status polls; every subsequent purpose update must name this stream.
    while helper.try_recv().is_ok() {}
    runtime.apply(Event::VoiceStart).await?;
    let playback = helper.try_recv().context("playback stop absent")?;
    assert_eq!(playback["command"], "playback_stop");
    let shared = take_helper_command(&mut helper, "audio_purpose")?;
    assert_eq!(shared["stream_id"], stream);
    assert_eq!(shared["purpose"], "voice_note");
    runtime.apply(Event::VoiceEnd).await?;
    let resumed = take_helper_command(&mut helper, "audio_purpose")?;
    assert_eq!(resumed["stream_id"], stream);
    assert_eq!(resumed["purpose"], "meeting");

    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.apply(Event::VoiceStart).await?;
    let playback = helper.try_recv().context("playback stop absent")?;
    assert_eq!(playback["command"], "playback_stop");
    let note = take_helper_command(&mut helper, "audio_start")?;
    assert_eq!(note["purpose"], "voice_note");
    runtime
        .native_event(json!({"event":"response","id":note["id"],"ok":true,
        "result":{"stream_id":note["stream_id"]}}))
        .await?;
    runtime
        .apply(Event::StartMeeting {
            id: "shared-meeting".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    runtime.apply(Event::PauseMeeting).await?;
    while let Ok(command) = helper.try_recv() {
        assert_eq!(
            command["command"], "status",
            "the note keeps its purpose and microphone"
        );
    }
    Ok(())
}
