//! Actor wiring tests, not certification of a microphone or acoustic AEC.
use super::*;
use orbit_core::CallPhase;
use orbit_core::protocol::Command;

#[path = "call_recovery_tests.rs"]
mod recovery;

#[path = "call_health_tests.rs"]
mod health;

#[path = "call_turn_tests.rs"]
mod turns;

fn processing() -> Value {
    json!({"processing":"voice_processing","processing_enabled":true,
        "processing_bypassed":false,"agc_enabled":false,"playback_reference":"shared_engine",
        "pipeline_id":"synthetic-pipeline","playback_active":false,
        "playback_observation":"relay_drain"})
}

#[tokio::test]
async fn new_call_does_not_inherit_an_old_audio_reserve_loss() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.capture.reserve_exhausted = true;
    let _ = request_call(&mut runtime, &mut helper).await?;
    assert!(!runtime.capture.reserve_exhausted);
    Ok(())
}

fn call_frame(stream: &str, sequence: u64, voiced: bool) -> Value {
    let mut value = processing();
    let audio = frame(stream, sequence, 800);
    for key in [
        "event",
        "source",
        "stream_id",
        "sample_rate",
        "channels",
        "frames",
        "sequence",
    ] {
        value[key] = audio[key].clone();
    }
    value["monotonic_ns"] = json!(sequence * 100_000_000);
    let sample = if voiced { 1000_i16 } else { 0_i16 };
    value["pcm_s16le_base64"] = json!(STANDARD.encode(sample.to_le_bytes().repeat(800)));
    value
}

#[tokio::test]
async fn expired_call_tick_does_not_claim_mars_interrupted_its_voice_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-expired-call".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-expired-call".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime.state.durable.call = Some(orbit_core::CallState {
        id: "expired-call".into(),
        phase: CallPhase::Listening,
        started_at_ms: 0,
        expires_at_ms: 1,
        turns: 0,
    });
    runtime.apply(Event::Tick).await?;
    let saved = runtime
        .fixture_store
        .load_snapshot::<orbit_core::State>()?
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
        Some("voice_transcription_call_stopped_automatically")
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    Ok(())
}

#[tokio::test]
async fn helper_cut_does_not_claim_mars_interrupted_its_voice_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-helper-cut".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-helper-cut".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime.state.durable.call = Some(orbit_core::CallState {
        id: "call-helper-cut".into(),
        phase: CallPhase::Listening,
        started_at_ms: 1,
        expires_at_ms: i64::MAX,
        turns: 0,
    });
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::HelperLost,
            "Corte sintético del helper",
        )
        .await;
    let saved = runtime
        .fixture_store
        .load_snapshot::<orbit_core::State>()?
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
        Some("voice_transcription_helper_cut")
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    Ok(())
}

async fn assert_automatic_stop_reason(
    reason: orbit_core::voice::StopReason,
    cause: &str,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-call-stop-reason".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-call-stop-reason".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    runtime.state.durable.call = Some(orbit_core::CallState {
        id: "call-stop-reason".into(),
        phase: CallPhase::Listening,
        started_at_ms: 1,
        expires_at_ms: i64::MAX,
        turns: 0,
    });
    runtime.stop_call_for_reason(reason).await?;
    assert!(
        crate::daemon::turn::said::mentions(&runtime.state, cause),
        "{reason:?}"
    );
    let saved = runtime
        .fixture_store
        .load_snapshot::<orbit_core::State>()?
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
        Some("voice_transcription_call_stopped_automatically"),
        "{reason:?}"
    );
    assert!(
        runtime.conversation.voice_task.active().is_none(),
        "{reason:?}"
    );
    Ok(())
}

#[tokio::test]
async fn automatic_call_stops_keep_their_own_reason() -> Result<()> {
    use orbit_core::voice::StopReason;
    for (reason, cause) in [
        (StopReason::Unauthorized, "unauthorized"),
        (StopReason::GenerationChanged, "generation_changed"),
        (StopReason::ProcessingLost, "processing_lost"),
        (StopReason::Discontinuity, "discontinuity"),
        (StopReason::Expired, "expired"),
        (StopReason::Idle, "idle"),
    ] {
        assert_automatic_stop_reason(reason, cause).await?;
    }
    Ok(())
}

#[tokio::test]
async fn expired_detector_frame_routes_its_reason_into_the_voice_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut helper).await?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-detector-expired".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-detector-expired".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    let expires_at_ms = runtime
        .state
        .durable
        .call
        .as_ref()
        .context("call absent")?
        .expires_at_ms;
    let action = runtime
        .prepare_call_frame(&call_frame(&stream, 0, false), expires_at_ms)
        .await?;
    assert_eq!(
        action,
        orbit_core::voice::TurnAction::Stop(orbit_core::voice::StopReason::Expired)
    );
    let saved = runtime
        .fixture_store
        .load_snapshot::<orbit_core::State>()?
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
        Some("voice_transcription_call_stopped_automatically")
    );
    Ok(())
}

#[tokio::test]
async fn new_call_turn_names_the_turn_instead_of_a_mars_interrupt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut helper).await?;
    runtime
        .apply(Event::VoiceStored {
            id: "voice-call-barge-in".into(),
            duration_ms: 1_000,
        })
        .await?;
    runtime.conversation.voice_operation = Some("voice-call-barge-in".into());
    pending_work(
        &mut runtime.conversation.voice_task,
        runtime.state.generation(),
        (),
    )?;
    let at = runtime
        .state
        .durable
        .call
        .as_ref()
        .context("call absent")?
        .started_at_ms;
    let first = runtime
        .prepare_call_frame(&call_frame(&stream, 0, true), at)
        .await?;
    assert_eq!(first, orbit_core::voice::TurnAction::None);
    let second = runtime
        .prepare_call_frame(&call_frame(&stream, 1, true), at + 100)
        .await?;
    assert_eq!(
        second,
        orbit_core::voice::TurnAction::Begin { interrupt: true }
    );
    let saved = runtime
        .fixture_store
        .load_snapshot::<orbit_core::State>()?
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
        Some("voice_transcription_call_turn_replaced")
    );
    Ok(())
}

async fn request_call(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<Value> {
    request_call_for(runtime, receiver, 60_000).await
}

async fn request_call_for(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    duration_ms: i64,
) -> Result<Value> {
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
            duration_ms,
        })
        .await?;
    assert_eq!(
        receiver.recv().await.context("playback stop absent")?["command"],
        "playback_stop"
    );
    let command = receiver.recv().await.context("audio start absent")?;
    assert_eq!(command["processing"], "voice_processing");
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .context("call absent")?
            .phase,
        CallPhase::Starting
    );
    Ok(command)
}

pub(in crate::daemon) async fn start_call(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    start_call_for(runtime, receiver, 60_000).await
}

/// A call authorized for `duration_ms`, opened and listening.
async fn start_call_for(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    duration_ms: i64,
) -> Result<String> {
    let command = request_call_for(runtime, receiver, duration_ms).await?;
    let mut result = processing();
    result["stream_id"] = command["stream_id"].clone();
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":true,"result":result}))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .context("call absent")?
            .phase,
        CallPhase::Listening
    );
    Ok(command["stream_id"]
        .as_str()
        .context("stream absent")?
        .into())
}

#[tokio::test]
async fn interrupt_snapshot_failure_keeps_the_call_listening() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let _stream = start_call(&mut runtime, &mut helper).await?;
    runtime
        .state
        .durable
        .call
        .as_mut()
        .context("call missing before interrupt")?
        .phase = CallPhase::UserSpeaking;
    runtime.state.durable.active_request = Some("call-turn".into());
    runtime.conversation.voice_operation = Some("call-turn".into());
    runtime.conversation.voice_task.attach_test(
        runtime.state.generation(),
        (),
        tokio::spawn(std::future::pending()),
    );
    runtime.store_actor.snapshot(&runtime.state).await?;
    runtime.work.durable_snapshot = crate::daemon::kernel::DurableSnapshot::from(&runtime.state);
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER review_fail_call_interrupt_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           SELECT RAISE(ABORT, 'synthetic call interrupt snapshot failure');
         END;",
    )?;

    let error = runtime
        .command(Command::Interrupt)
        .await
        .err()
        .context("interrupt snapshot unexpectedly succeeded")?;
    assert!(
        error
            .to_string()
            .contains("synthetic call interrupt snapshot failure"),
        "fixture failed before the decision snapshot: {error}"
    );
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some("call-turn")
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    assert!(runtime.conversation.voice_operation.is_none());
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .context("call lost")?
            .phase,
        CallPhase::Listening,
        "the call turn was cut before the failed decision snapshot"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_native_processing_ack_never_authorizes_derived_voice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let command = request_call(&mut runtime, &mut receiver).await?;
    let mut result = processing();
    result["stream_id"] = command["stream_id"].clone();
    result["processing_bypassed"] = json!(true);
    assert!(
        runtime
            .native_event(json!({"event":"response","id":command["id"],"ok":true,"result":result}))
            .await
            .is_err()
    );
    assert!(!runtime.state.capabilities().voice_processing_available);
    assert!(runtime.capture.buffer.voice.is_none());
    // REF-12: the loop contains the failure; the live call is cut with its
    // cause and nothing latches.
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::CaptureEventFailed,
            "Synthetic invalid processing readback",
        )
        .await;
    assert!(runtime.state.durable.call.is_none() && !runtime.state.privacy_stopped());
    assert!(
        runtime
            .fixture_store
            .get_job("call-processing-synthetic-call")?
            .is_none()
    );
    loop {
        let stop = receiver.recv().await.context("capture stop absent")?;
        assert_ne!(
            stop["command"], "privacy_stop",
            "a cut never latches the helper"
        );
        if stop["command"] == "capture_stop" {
            runtime
                .native_event(json!({"event":"response","id":stop["id"],"ok":true,
                "result":{"drain_complete":true,"privacy_stopped":false}}))
                .await?;
            break;
        }
    }
    assert!(!runtime.accepts_audio(&call_frame(
        command["stream_id"].as_str().context("stream absent")?,
        0,
        true
    )));
    assert!(runtime.conversation.voice_task.active().is_none());
    Ok(())
}

#[tokio::test]
async fn explicit_call_stop_preserves_silence_and_cancelled_utterance_in_original() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut receiver).await?;
    runtime.native_event(call_frame(&stream, 0, true)).await?;
    runtime.native_event(call_frame(&stream, 1, true)).await?;
    assert!(runtime.capture.buffer.voice.is_some());
    runtime.command(crate::daemon::Command::Interrupt).await?;
    assert!(runtime.capture.buffer.voice.is_none());
    for sequence in 2..12 {
        runtime
            .native_event(call_frame(&stream, sequence, false))
            .await?;
    }
    assert!(runtime.state.durable.call.is_some() && runtime.capture.mic_stream.is_some());
    assert!(runtime.conversation.voice_task.active().is_none());
    runtime.apply(Event::CallStop).await?;
    let stop = loop {
        let command = receiver.recv().await.context("audio stop absent")?;
        if command["command"] == "audio_stop" {
            break command;
        }
    };
    runtime.native_event(call_frame(&stream, 12, false)).await?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
        "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let chunks = runtime
        .fixture_store
        .list_chunks("synthetic-call", None, 10)?;
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.metadata.frames)
            .sum::<u64>(),
        13 * 800
    );
    let mut pcm = Vec::new();
    for chunk in chunks {
        pcm.extend_from_slice(&runtime.fixture_store.read_chunk(&chunk.metadata.id)?[44..]);
    }
    assert_eq!(&pcm[..3200], &1000_i16.to_le_bytes().repeat(1600));
    assert!(pcm[3200..].iter().all(|byte| *byte == 0));
    let manifest = runtime
        .fixture_store
        .get_job("recording-synthetic-call")?
        .context("manifest absent")?;
    assert_eq!(manifest.payload["original_includes_silence"], true);
    assert_eq!(manifest.payload["aec_hardware_validated"], false);
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .is_some()
    );
    assert!(runtime.capture.call_runtime.is_none() && runtime.capture.mic_stream.is_none());
    Ok(())
}

#[tokio::test]
async fn barge_in_invalidates_stt_operation_without_stopping_original_capture() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut receiver).await?;
    runtime.conversation.voice_operation = Some("stale-call-turn".into());
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    let generation = runtime.state.generation();
    runtime
        .conversation
        .voice_task
        .attach_test(generation, (), task);
    for sequence in 0..2 {
        runtime
            .native_event(call_frame(&stream, sequence, true))
            .await?;
    }
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    assert!(runtime.conversation.voice_operation.is_none());
    assert!(runtime.capture.buffer.voice.is_some());
    assert_eq!(runtime.state.generation(), generation);
    assert_eq!(
        runtime
            .state
            .durable
            .call
            .as_ref()
            .context("call stopped")?
            .phase,
        CallPhase::UserSpeaking
    );
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(stream.as_str()));
    assert_eq!(
        receiver
            .recv()
            .await
            .context("barge playback stop absent")?["command"],
        "playback_stop"
    );
    assert!(receiver.try_recv().is_err());
    runtime
        .work_event(WorkResult::Capture(
            crate::daemon::CaptureWork::Transcribed {
                ticket: crate::daemon::ticketed::WorkTicket::test(generation),
                source: VoiceSource {
                    recording_id: "stale-source".into(),
                    chunk_id: "stale-chunk".into(),
                    session_id: None,
                    message_id: "stale-call-turn".into(),
                    duration_ms: 0,
                },
                text: "SYNTHETIC-LATE-TURN".into(),
            },
        ))
        .await?;
    assert!(runtime.state.durable.turn.is_none());
    assert_eq!(
        runtime.capture.buffer.recording_owner("mic_local"),
        Some("synthetic-call")
    );
    Ok(())
}
