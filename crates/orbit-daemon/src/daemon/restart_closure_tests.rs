//! A recording an older Orbit left cut (`Interrupted`) or still closing is
//! closed when this one starts, at the end of the audio it kept, so the hub
//! writes its note. A capture the restart itself cut goes on instead.
//! Synthetic audio and helper only.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::meeting_continuity_tests::{frame, granted_status, phase, start};

/// Records two one-second chunks, lets `edit` change the saved recording as
/// an older run would have left it, and returns the runtime as a new daemon
/// finds it after its start, with the recording's last audio.
async fn restarted_with(
    directory: &std::path::Path,
    edit: impl FnOnce(&mut orbit_core::Recording),
) -> Result<(Runtime, mpsc::Receiver<Value>, i64)> {
    let (mut runtime, mut receiver) = fixture(directory)?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    let mut saved = runtime.state.durable.clone();
    let recording = saved.capture.recording.as_mut().context("recording")?;
    let last_audio = recording
        .tracks
        .iter()
        .map(|track| track.last_chunk_at_ms)
        .max()
        .context("tracks")?;
    assert!(last_audio > recording.started_at_ms || recording.sealed_chunks == 2);
    edit(recording);
    runtime.capture.recovery =
        crate::daemon::cut_recovery::recovery_after_restart(saved.capture.recording.as_ref(), true);
    runtime.state.durable = saved.recovered();
    runtime.capture.buffer.reset();
    runtime.audio.mic_stream = None;
    runtime.audio.pending_starts.clear();
    while receiver.try_recv().is_ok() {}
    runtime
        .recover_recording_at_start(orbit_core::RecordingInterruption {
            cause: orbit_core::RecordingInterruptionCause::CauseNotSaved,
            detail: None,
            audio_available: false,
        })
        .await?;
    Ok((runtime, receiver, last_audio))
}

fn closed_at(runtime: &Runtime) -> Option<i64> {
    runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .and_then(|recording| recording.ended_at_ms)
}

#[tokio::test]
async fn a_recording_an_older_orbit_left_interrupted_is_closed_at_start_at_its_last_audio()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _receiver, last_audio) = restarted_with(directory.path(), |recording| {
        recording.phase = orbit_core::CapturePhase::Interrupted;
    })
    .await?;
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Idle));
    assert_eq!(closed_at(&runtime), Some(last_audio));
    Ok(())
}

/// The counts in the snapshot are only a projection of the sealed chunks:
/// one saved before its receipts arrived (or by a version that lost them)
/// used to leave the recording interrupted for ever, since an «empty»
/// recording with chunks is closed by nobody.
#[tokio::test]
async fn an_interrupted_recording_whose_snapshot_lost_its_counts_is_closed_at_its_last_audio()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _receiver, last_audio) = restarted_with(directory.path(), |recording| {
        recording.phase = orbit_core::CapturePhase::Interrupted;
        recording.sealed_chunks = 0;
        recording.tracks.clear();
    })
    .await?;
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Idle));
    assert_eq!(closed_at(&runtime), Some(last_audio));
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording")?;
    assert_eq!(recording.sealed_chunks, 2, "rebuilt from the sealed chunks");
    Ok(())
}

#[tokio::test]
async fn a_recording_an_older_orbit_left_closing_is_closed_at_start_at_its_last_audio() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (runtime, _receiver, last_audio) = restarted_with(directory.path(), |recording| {
        recording.phase = orbit_core::CapturePhase::Finalizing;
        recording.finalizing_since_ms = Some(recording.started_at_ms + 1);
    })
    .await?;
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Idle));
    assert_eq!(closed_at(&runtime), Some(last_audio));
    Ok(())
}

/// The closure is durable work for the hub: its finalize job is queued.
#[tokio::test]
async fn the_closure_at_start_queues_the_recording_for_its_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver, _) = restarted_with(directory.path(), |recording| {
        recording.phase = orbit_core::CapturePhase::Interrupted;
    })
    .await?;
    runtime.queue_finalize().await?;
    let job = runtime
        .reader
        .get_job("finalize-reunion-sintetica")?
        .context("the closure was not queued for the hub")?;
    assert_eq!(job.kind, "recording_finalize");
    Ok(())
}

/// A meeting that was live when the daemon stopped is not closed at start:
/// it goes on once the helper is back.
#[tokio::test]
async fn a_meeting_the_restart_cut_is_not_closed_at_start_and_goes_on() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver, _) = restarted_with(directory.path(), |_| {}).await?;
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Interrupted));
    assert_eq!(closed_at(&runtime), None);
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":[]}))
        .await;
    runtime
        .capture
        .advance_recovery_clock(std::time::Duration::from_secs(1));
    let mut reopened = false;
    for _ in 0..3 {
        runtime.run_maintenance().await;
        while let Ok(command) = receiver.try_recv() {
            if command["command"] == "status" {
                runtime
                    .handle_native_event(granted_status(&command["id"]))
                    .await;
                runtime
                    .capture
                    .advance_recovery_clock(std::time::Duration::from_secs(30));
            }
            reopened |= command["command"] == "audio_start";
        }
    }
    assert!(reopened, "the meeting did not go on after the restart");
    Ok(())
}
