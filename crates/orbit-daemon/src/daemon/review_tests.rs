//! Inverted regressions from the independent implementation review. All audio
//! is synthetic; no helper, device, provider, or network endpoint is contacted.
use super::*;

async fn privacy_ack(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<()> {
    loop {
        let command = receiver.recv().await.context("privacy command absent")?;
        if command["command"] == "privacy_stop" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":{"privacy_stopped":true,"drain_complete":true}}))
                .await?;
            return Ok(());
        }
    }
}

fn cancelled_source(runtime: &Runtime, frames: u64) -> Result<String> {
    let manifest = runtime
        .fixture_store
        .list_jobs(30)?
        .into_iter()
        .find(|job| {
            job.kind == "recording_manifest" && job.payload["recording_kind"] == "voice_turn"
        })
        .context("cancelled voice manifest absent")?;
    assert_eq!(manifest.payload["upload_allowed"], false);
    assert_eq!(manifest.payload["voice_state"], "cancelled");
    let recording = manifest.payload["recording_id"]
        .as_str()
        .context("recording absent")?;
    let chunks = runtime.fixture_store.list_chunks(recording, None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, frames);
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&format!("upload-{}", chunks[0].metadata.id))?
            .context("upload receipt absent")?
            .state,
        JobState::Cancelled
    );
    assert!(
        runtime.conversation.voice_task.active().is_none()
            && runtime.conversation.voice_operation.is_none()
    );
    assert!(runtime.state.durable.turn.is_none());
    Ok(chunks[0].metadata.id.clone())
}

#[tokio::test]
async fn privacy_keeps_ptt_tail_until_a_confirmed_barrier_and_quarantines_source() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.apply(Event::VoiceStart).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("voice discarded")?
            .pcm
            .len(),
        200
    );
    runtime.native_event(frame(&stream, 1, 50)).await?;
    privacy_ack(&mut runtime, &mut receiver).await?;
    settle_seal_receipts(&mut runtime).await?;
    let chunk = cancelled_source(&runtime, 150)?;
    assert_eq!(runtime.fixture_store.read_chunk(&chunk)?.len(), 344);
    assert_eq!(
        runtime.fixture_store.list_chunks("meeting", None, 10)?[0]
            .metadata
            .frames,
        150
    );
    assert!(runtime.capture.buffer.voice.is_none());
    assert!(!runtime.audio_is_draining());
    Ok(())
}

#[tokio::test]
async fn helper_loss_preserves_the_voice_prefix_without_provider_submission() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.apply(Event::VoiceStart).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    // The loss cuts the live voice note; it no longer latches a stop.
    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    settle_seal_receipts(&mut runtime).await?;
    cancelled_source(&runtime, 100)?;
    assert!(!runtime.state.privacy_stopped());
    assert!(!runtime.state.durable.voice_active);
    assert!(runtime.capture.mic_stream.is_none());
    Ok(())
}

#[tokio::test]
async fn failed_source_write_retains_pcm_and_policy_until_a_receipt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.buffer.ingest(&frame("voice", 0, 100), 42)?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    let chunks = runtime.paths.store.join("chunks");
    let backup = runtime.paths.store.join("fixture-chunks-backup");
    std::fs::rename(&chunks, &backup)?;
    std::fs::write(&chunks, b"synthetic write obstacle")?;
    assert!(runtime.flush_stopped_audio().await.is_err());
    assert_eq!(runtime.capture.pending_seals.len(), 1);
    assert_eq!(runtime.capture.pending_seals[0].pcm, vec![1; 200]);
    let recording = runtime.capture.pending_seals[0]
        .metadata
        .recording_id
        .clone();
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&format!("recording-{recording}"))?
            .context("policy absent")?
            .payload["upload_allowed"],
        false
    );
    assert!(runtime.conversation.voice_task.active().is_none());
    std::fs::remove_file(&chunks)?;
    std::fs::rename(&backup, &chunks)?;
    runtime.retry_seals()?;
    settle_seal_receipts(&mut runtime).await?;
    let receipt = &runtime.fixture_store.list_chunks(&recording, None, 10)?[0];
    assert_eq!(
        &runtime.fixture_store.read_chunk(&receipt.metadata.id)?[44..],
        &vec![1; 200]
    );
    assert!(runtime.capture.pending_seals.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_stop_ack_without_drain_proof_does_not_consume_stream_identity() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop absent")?;
    assert!(
        runtime
            .native_event(json!({"event":"response","id":stop["id"],"ok":true,
        "result":{"stream_id":stream,"drain_complete":false}}))
            .await
            .is_err()
    );
    assert!(runtime.audio_is_draining());
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(stream.as_str()));
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    runtime.native_event(frame(&stream, 1, 100)).await?;
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
        "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(
        runtime.fixture_store.list_chunks("meeting", None, 10)?[0]
            .metadata
            .frames,
        200
    );
    Ok(())
}

#[tokio::test]
async fn expected_ptt_busy_refusal_is_local_and_does_not_stop_a_meeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    let generation = runtime.state.generation();
    runtime
        .conversation
        .voice_task
        .attach_test(generation, (), task);
    runtime.conversation.voice_operation = Some("pending-stt".into());
    runtime
        .native_event(json!({"event":"hotkey","action":"ptt","phase":"pressed"}))
        .await?;
    assert!(!runtime.state.privacy_stopped());
    assert_eq!(runtime.capture.mic_stream.as_deref(), Some(stream.as_str()));
    assert!(runtime.recording_active());
    assert!(runtime.capture.buffer.voice.is_none());
    assert!(!abort.is_finished());
    assert!(receiver.try_recv().is_err());
    runtime
        .cancel_voice_transcription(super::VoiceCancellationCause::Authority(
            orbit_core::authority::AuthorityChange::Interrupt,
        ))
        .await?;
    Ok(())
}

#[tokio::test]
async fn ptt_refuses_an_active_assistant_turn_before_starting_capture() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime.state.durable.active_request = Some("assistant-turn".into());
    let before = runtime.state.generation();
    runtime
        .native_event(json!({"event":"hotkey","action":"ptt","phase":"pressed"}))
        .await?;
    assert_eq!(
        runtime.state.durable.active_request.as_deref(),
        Some("assistant-turn")
    );
    assert_eq!(runtime.state.generation(), before);
    assert!(!runtime.state.durable.voice_active && !runtime.state.privacy_stopped());
    assert!(runtime.capture.buffer.voice.is_none() && runtime.capture.mic_stream.is_none());
    assert!(receiver.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn replacing_an_interrupted_meeting_persists_its_closure_with_a_gap() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    privacy_ack(&mut runtime, &mut receiver).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.fixture_store.get_job("finalize-meeting")?.is_none());
    runtime.apply(Event::LocalRearm).await?;
    runtime
        .apply(Event::StartMeeting {
            id: "replacement".into(),
            title: "Synthetic next".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-meeting")?
        .context("old identity lost")?;
    assert_eq!(closure.state, JobState::Queued);
    assert_eq!(closure.payload["recording_id"], "meeting");
    assert!(
        closure.payload["manifest"]["gaps"]
            .as_u64()
            .context("gap absent")?
            >= 1
    );
    assert_eq!(
        closure.payload["manifest"]["tracks"][0]["total_frames"],
        100
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("new recording absent")?
            .id,
        "replacement"
    );
    assert!(runtime.work.worker.active().is_none());
    Ok(())
}

#[tokio::test]
async fn pending_old_pcm_blocks_replacement_without_overwriting_its_identity() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    privacy_ack(&mut runtime, &mut receiver).await?;
    runtime.apply(Event::LocalRearm).await?;
    let metadata = runtime.fixture_store.list_chunks("meeting", None, 10)?[0]
        .metadata
        .clone();
    runtime
        .capture
        .pending_seals
        .push_back(crate::capture::Segment {
            metadata,
            pcm: vec![1; 200],
        });
    assert!(
        runtime
            .apply(Event::StartMeeting {
                id: "replacement".into(),
                title: "Synthetic next".into(),
                audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            })
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("old recording absent")?
            .id,
        "meeting"
    );
    assert!(runtime.fixture_store.get_job("finalize-meeting")?.is_none());
    Ok(())
}

#[tokio::test]
async fn manifest_write_failure_keeps_voice_owned_and_shutdown_cannot_discard_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(false);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.buffer.ingest(&frame("voice", 0, 100), 42)?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fault.execute_batch("CREATE TRIGGER synthetic_manifest_fault BEFORE INSERT ON jobs WHEN NEW.kind = 'recording_manifest' BEGIN SELECT RAISE(FAIL, 'synthetic manifest write failure'); END;")?;
    // The loop contains the failure; with nothing live it is said.
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::CaptureMaintenanceFailed,
            "Synthetic disk failure",
        )
        .await;
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("voice lost on manifest error")?
            .pcm,
        vec![1; 200]
    );
    assert!(runtime.capture.pending_seals.is_empty());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    assert!(
        runtime
            .command(crate::daemon::Command::Shutdown)
            .await
            .is_err()
    );
    assert!(runtime.capture.buffer.voice.is_some());
    assert!(runtime.conversation.voice_task.active().is_none());
    fault.execute_batch("DROP TRIGGER synthetic_manifest_fault;")?;
    runtime.capture.seal_retry_at = None;
    runtime.maintain_capture().await?;
    settle_seal_receipts(&mut runtime).await?;
    cancelled_source(&runtime, 100)?;
    assert!(runtime.capture.buffer.voice.is_none());
    // Closing cut the capture without latching; the next start comes back working.
    assert!(!runtime.state.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn a_gap_during_privacy_drain_remains_attached_to_the_interrupted_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
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
    // Recorded, no longer escalated. Privacy is already stopped here, so the
    // error this used to return bought no safety: it only ran fail_closed again,
    // which requests another native stop, whose teardown produces the next gap.
    // Five of those in eight seconds is what the developer's Mac was doing. The guarantee
    // this test is named for — the gap stays attached to the interrupted
    // recording — is asserted below and unchanged.
    runtime
        .native_event(
            json!({"event":"gap","source":"mic_local","stream_id":stream,
        "reason":"pre_stop_possible"}),
        )
        .await?;
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
    privacy_ack(&mut runtime, &mut receiver).await?;
    settle_seal_receipts(&mut runtime).await?;
    runtime.apply(Event::LocalRearm).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-meeting")?
        .context("closure absent")?;
    assert_eq!(closure.payload["manifest"]["gaps"], 1);
    Ok(())
}

#[tokio::test]
async fn ptt_only_mic_gap_does_not_corrupt_a_paused_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.apply(Event::VoiceStart).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime.apply(Event::PauseMeeting).await?;
    assert_eq!(runtime.capture.buffer.recording_owner("mic_local"), None);
    assert!(
        runtime
            .native_event(json!({"event":"gap","source":"mic_local","stream_id":stream}))
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .gaps,
        0
    );
    Ok(())
}

#[tokio::test]
async fn abandoning_an_unconfirmed_stop_declares_missing_tail_before_finalization() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.native_event(frame(&stream, 0, 100)).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    runtime.abandon_audio_drain().await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(!runtime.audio_is_draining());
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .gaps
            >= 1
    );
    runtime.apply(Event::LocalRearm).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-meeting")?
        .context("closure absent")?;
    assert!(
        closure.payload["manifest"]["gaps"]
            .as_u64()
            .context("gap absent")?
            >= 1
    );
    assert_eq!(
        runtime.fixture_store.list_chunks("meeting", None, 10)?[0]
            .metadata
            .frames,
        100
    );
    Ok(())
}
