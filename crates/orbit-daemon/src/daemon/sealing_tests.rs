use super::*;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
// `retry_seals` hands the segments to the store actor's own thread
// and does not promise their receipts have landed when it returns, so the
// queue draining is an event, not the instant after asking for it. This
// wait lives next to the fixture, so every sealing test waits on the same
// event with the same measured guard.
use crate::daemon::audio::tests::{await_seal_receipts, fixture, receive_helper_command};
use orbit_store::ChunkMetadata;

fn segment(track: &str, sequence: u64) -> crate::capture::Segment {
    crate::capture::Segment {
        metadata: ChunkMetadata {
            capture_epoch_id: None,
            id: format!("recording-{track}-{sequence}"),
            recording_id: "recording".into(),
            track: track.into(),
            sequence,
            sample_rate: 8000,
            channels: 1,
            frames: 8000,
            monotonic_ns: sequence * 1_000_000_000,
            first_sample: sequence * 8000,
            started_at_ms: 1000,
        },
        pcm: vec![7; 16000],
    }
}

#[tokio::test]
async fn a_failed_write_retains_both_tracks_until_a_durable_receipt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let blocked = runtime
        .paths
        .store
        .join("chunks/recording-mic_local-0.orbit");
    std::fs::create_dir(&blocked)?;
    assert!(
        runtime
            .seal(vec![segment("mic_local", 0), segment("remote_app", 0)])
            .is_err()
    );
    assert_eq!(runtime.capture.pending_seals.len(), 2);
    assert_eq!(
        runtime
            .capture
            .pending_seals
            .iter()
            .map(|part| part.pcm.len())
            .sum::<usize>(),
        32000
    );
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert!(runtime.flush_stopped_audio().await.is_err());
    assert_eq!(runtime.capture.pending_seals.len(), 2);
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    std::fs::remove_dir(blocked)?;
    runtime.retry_seals()?;
    await_seal_receipts(&mut runtime)?;
    assert!(runtime.capture.pending_seals.is_empty());
    assert!(runtime.state.privacy_stopped());
    assert!(runtime.transfers.worker.active().is_none());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 2);
    for track in ["mic_local", "remote_app"] {
        let bytes = runtime
            .fixture_store
            .read_chunk(&format!("recording-{track}-0"))?;
        assert_eq!(&bytes[44..], &vec![7; 16000]);
    }
    Ok(())
}

#[tokio::test]
async fn a_repeated_attempt_never_advances_or_replaces_pending_identity() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let blocked = runtime
        .paths
        .store
        .join("chunks/recording-mic_local-0.orbit");
    std::fs::create_dir(&blocked)?;
    assert!(runtime.seal(vec![segment("mic_local", 0)]).is_err());
    for _ in 0..3 {
        assert!(runtime.retry_seals().is_err());
    }
    assert_eq!(
        runtime
            .capture
            .pending_seals
            .get(0)
            .context("no seal owed")?
            .metadata
            .sequence,
        0
    );
    assert_eq!(
        runtime
            .capture
            .pending_seals
            .get(0)
            .context("no seal owed")?
            .metadata
            .first_sample,
        0
    );
    assert_eq!(
        runtime
            .capture
            .pending_seals
            .get(0)
            .context("no seal owed")?
            .pcm,
        vec![7; 16000]
    );
    assert!(
        runtime
            .command(orbit_core::protocol::Command::Shutdown)
            .await
            .is_err()
    );
    assert_eq!(runtime.capture.pending_seals.len(), 1);
    std::fs::remove_dir(blocked)?;
    runtime.retry_seals()?;
    // The same disease, one test further down the same file: the
    // chunk count only moves once the store actor's receipt has come back.
    await_seal_receipts(&mut runtime)?;
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    Ok(())
}

/// A failed sealing retry must not skip the rest of the capture
/// maintenance turn. Before, the `?` on the retry returned before the core
/// `Tick`, so a disk that kept refusing seals also froze the finalize
/// deadline: a recording that should have been interrupted on that very turn
/// stayed `Finalizing`, and the call checks and the queued finalize never ran
/// either.
///
/// The failure is produced at the destination check — a directory where the
/// chunk file must go — which fails deterministically without racing the
/// store actor's thread, and the deadline is aged past
/// `FINALIZING_TIMEOUT_MS` so the Tick must judge the recording this turn.
#[tokio::test]
async fn a_failed_seal_retry_does_not_skip_the_rest_of_the_turn() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(orbit_core::Event::StartMeeting {
            id: "seal-retry".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let start = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":start["stream_id"]}}))
        .await?;
    runtime.apply(orbit_core::Event::FinishMeeting).await?;
    // A pending seal whose destination is not a regular file: the retry
    // fails at the destination check, on the daemon's own thread.
    runtime
        .capture
        .pending_seals
        .push_back(segment("mic_local", 0));
    let blocked = runtime
        .paths
        .store
        .join("chunks/recording-mic_local-0.orbit");
    std::fs::create_dir(&blocked)?;
    runtime.capture.seal_retry_at = None;
    // Old enough that the finalize deadline fires on this very turn.
    if let Some(recording) = runtime.state.durable.capture.recording.as_mut() {
        recording.finalizing_since_ms = recording
            .finalizing_since_ms
            .map(|since| since.saturating_sub(orbit_core::FINALIZING_TIMEOUT_MS + 1_000));
    }

    runtime.maintain_capture().await?;
    // A failed seal is counted without cutting; the finalize tick still ran.
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording")?
            .phase,
        orbit_core::CapturePhase::Interrupted,
        "the rest of the turn (the finalize-deadline Tick) did not run"
    );
    Ok(())
}

/// A failed stopped-audio flush is the same disease one branch
/// earlier: the `return Err` skipped the rest of the capture maintenance
/// turn, including the core `Tick`, so a disk that kept refusing the local
/// flush also froze the finalize deadline. The microphone is not
/// touched by this test at all — the flush fails at the destination check,
/// on the daemon's own thread.
///
/// The Finalizing phase and its aged deadline are set by hand: the natural
/// path to a stopped-audio flush owed (the user's stop) closes the recording
/// locally instead of parking it in `Finalizing`, so the deadline the Tick
/// must judge cannot be produced by the event itself.
#[tokio::test]
async fn a_failed_stopped_audio_flush_does_not_skip_the_rest_of_the_turn() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .apply(orbit_core::Event::StartMeeting {
            id: "flush-retry".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let start = receive_helper_command(&mut receiver, "audio_start").await?;
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":start["stream_id"]}}))
        .await?;
    // The user's stop is latched directly: the flush it owes runs from maintenance
    // once the drain is no longer pending, which is the branch under test.
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    // Buffered voice the flush must try to seal; its chunk id is generated,
    // so the destination failure is produced at the directory check.
    runtime.capture.buffer.begin_voice()?;
    runtime.capture.buffer.ingest_voice_frame(
        &json!({"event":"audio","source":"mic_local","stream_id":"flush-voice",
            "sample_rate":8000,"channels":1,"frames":80,"sequence":0,
            "monotonic_ns":0,"pcm_s16le_base64":STANDARD.encode(vec![1u8; 160])}),
        0,
    )?;
    let chunks = runtime.paths.store.join("chunks");
    std::fs::remove_dir_all(&chunks)?;
    std::fs::write(&chunks, b"not a directory")?;
    runtime.capture.seal_retry_at = None;
    // Old enough that the finalize deadline fires on this very turn. Set by
    // hand with the phase: the natural FinishMeeting path is unavailable
    // here (the user's latched stop closes the recording locally instead of
    // parking it in `Finalizing`).
    if let Some(recording) = runtime.state.durable.capture.recording.as_mut() {
        recording.phase = orbit_core::CapturePhase::Finalizing;
        recording.finalizing_since_ms = Some(0);
    }

    let maintenance = runtime.maintain_capture().await;
    let error = maintenance
        .err()
        .context("the failed flush must still reach the containment table")?;
    assert!(
        format!("{error:#}").contains("La ruta de fragmentos no es un directorio privado"),
        "the refusal must keep its cause: {error:#}"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording")?
            .phase,
        orbit_core::CapturePhase::Interrupted,
        "the rest of the turn (the finalize-deadline Tick) did not run"
    );
    Ok(())
}
