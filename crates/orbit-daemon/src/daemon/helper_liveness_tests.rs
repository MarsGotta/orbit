//! Helper liveness at the daemon boundary: a silent Mac cuts what was live
//! with its numbers and never stops Orbit. Time and native traffic are
//! synthetic; no helper, provider, device or real profile is contacted.

use super::*;
use crate::daemon::audio::tests::{await_seal_receipts, fixture};
use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{CapturePhase, Event, protocol::MeetingAudio};
use tokio::sync::mpsc;

const TIMEOUT_CAUSE: &str = "El Mac no confirmó la parada de audio en 3000 ms (plazo: 3000 ms sin respuesta del Mac, 15000 ms en total).";

async fn next_command(receiver: &mut mpsc::Receiver<Value>, name: &str) -> Result<Value> {
    loop {
        let command = receiver.recv().await.context("orden nativa ausente")?;
        if command["command"] == name {
            return Ok(command);
        }
    }
}

async fn begin_call_drain(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<()> {
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
            id: "call-liveness".into(),
            duration_ms: 60_000,
        })
        .await?;
    let _ = next_command(receiver, "audio_start").await?;
    runtime.apply(Event::CallStop).await?;
    let _ = next_command(receiver, "audio_stop").await?;
    Ok(())
}

fn audio_frame(stream: &str, sequence: u64) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":100, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(12_500_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; 200])})
}

#[tokio::test(start_paused = true)]
async fn live_helper_gets_fifteen_seconds_and_call_timeout_keeps_privacy_armed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    begin_call_drain(&mut runtime, &mut helper).await?;

    assert!(runtime.state.views.draining_since_ms.is_some());
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::AudioDrainWaiting)
    );
    for elapsed in 1..=14 {
        tokio::time::advance(Duration::from_secs(1)).await;
        runtime
            .native_event(json!({"event":"state", "privacy_stopped":false}))
            .await?;
        runtime.handle_audio_timeout().await;
        assert!(!runtime.state.privacy_stopped(), "failed at {elapsed} s");
        assert!(runtime.audio_is_draining());
    }

    tokio::time::advance(Duration::from_secs(1)).await;
    runtime
        .native_event(json!({"event":"state", "privacy_stopped":false}))
        .await?;
    runtime.handle_audio_timeout().await;

    assert!(!runtime.state.privacy_stopped());
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::CallDrainUnconfirmed)
    );
    assert!(!runtime.audio_is_draining());
    assert_eq!(runtime.state.views.draining_since_ms, None);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn three_silent_seconds_cut_what_was_live_and_say_why_with_numbers() -> Result<()> {
    // The silent Mac used to stop Orbit. Now what was live is cut,
    // the cause carries the wait and its windows, and nothing latches.
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    begin_call_drain(&mut runtime, &mut helper).await?;

    tokio::time::advance(Duration::from_secs(3)).await;
    runtime.handle_audio_timeout().await;

    assert!(!runtime.state.privacy_stopped());
    // The cut names its cause as a code; the sentence with the wait
    // and its windows is in daemon.log under the same op.
    assert!(
        crate::daemon::turn::said::mentions(&runtime.state, "audio_stop_unconfirmed"),
        "{:?}",
        runtime.state.durable.notice_log
    );
    let logged = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(logged.contains(TIMEOUT_CAUSE), "{logged}");
    assert_eq!(runtime.state.views.draining_since_ms, None);
    while let Ok(command) = helper.try_recv() {
        assert_ne!(
            command["command"], "privacy_stop",
            "a cut never latches the helper"
        );
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn silent_meeting_drain_cuts_the_recording_and_preserves_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "meeting-liveness".into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let start = next_command(&mut helper, "audio_start").await?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream ausente")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response", "id":start["id"], "ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(audio_frame(&stream, 0)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let _ = next_command(&mut helper, "audio_stop").await?;

    tokio::time::advance(Duration::from_secs(3)).await;
    runtime.handle_audio_timeout().await;

    // The recording is cut with what was kept; Orbit is not stopped.
    assert!(!runtime.state.privacy_stopped());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording missing")?
            .phase,
        CapturePhase::Interrupted
    );
    assert_eq!(runtime.state.views.draining_since_ms, None);
    Ok(())
}

/// The recording that was interrupted after the Mac had confirmed.
///
/// `complete_audio_drain` will not declare a recording finalized while a chunk
/// of it is still being written, which is right. It stores the intent and comes
/// back — and coming back is what went missing. Two paths observe a seal
/// landing: `collect_ready_seals`, draining the store actor's completions from
/// `seal` and from the tick, and the work event `StoreWork::Sealed`. Only the
/// second resumed the finish, and only when the segment was still pending by
/// the time it ran. Under load the collector gets there first, the work event
/// finds nothing to apply, and the deferred finish is dropped.
///
/// The recording then sits in `Finalizing` until `FINALIZING_TIMEOUT_MS` turns
/// it into `Interrupted` and tells the person the Mac never confirmed the
/// close. The Mac had confirmed it. The daemon had recorded that it did.
///
/// Measured before this test existed: four failures in twenty runs of
/// `real_daemon_observes_synthetic_tracks_without_discarding_originals` under
/// load, every one of them this.
async fn meeting_awaiting_only_its_finish(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<()> {
    runtime
        .apply(Event::StartMeeting {
            id: "meeting-awaiting-finish".into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let start = next_command(helper, "audio_start").await?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream ausente")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response", "id":start["id"], "ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(audio_frame(&stream, 0)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = next_command(helper, "audio_stop").await?;
    // The Mac answers and the daemon applies the answer through the
    // production path. That answer seals the tail, and the finish is deferred
    // behind the seal. Holding the store's writer keeps the seal in flight, so
    // the deferral is certain rather than the likely outcome of a race.
    let writer = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    writer.execute_batch("BEGIN IMMEDIATE")?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    assert!(
        !runtime.capture.pending_seals.is_empty(),
        "the confirmation had to leave the seal of the audio tail in flight"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording")?
            .phase,
        CapturePhase::Finalizing,
        "with the seal in flight the finish had to be deferred"
    );
    writer.execute_batch("ROLLBACK")?;
    // The collector applies the receipt, and the collector does not resume the
    // finish. What is left owed is the finish itself.
    await_seal_receipts(runtime)?;
    Ok(())
}

fn finalizing_for(runtime: &mut Runtime, millis: i64) {
    if let Some(recording) = runtime.state.durable.recording.as_mut() {
        recording.finalizing_since_ms = recording
            .finalizing_since_ms
            .map(|since| since.saturating_sub(millis));
    }
}

/// What already arrived is acted on before any deadline judges it.
///
/// Nothing is outstanding: the helper has confirmed and no seal is in flight.
/// A maintenance turn has to finish the recording, and finish it before
/// `Event::Tick` in the same turn can interrupt it, however long it has been
/// finalizing.
#[tokio::test]
async fn a_finish_owed_only_to_the_daemon_is_paid_before_the_deadline_judges_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    meeting_awaiting_only_its_finish(&mut runtime, &mut helper).await?;
    assert!(runtime.capture.pending_seals.is_empty());
    // Old enough that the deadline would fire on this very turn.
    finalizing_for(&mut runtime, orbit_core::FINALIZING_TIMEOUT_MS + 1_000);

    // A real maintenance turn, because the wiring is the whole subject: it is
    // not enough that a method exists which would pay the finish, something has
    // to call it, and call it before the tick that judges the recording. The
    // turn ends by scheduling uploads, which this fixture has no credential
    // for and which happens after everything under test, so the outcome of the
    // turn is not what this asserts on — the state is.
    runtime.run_maintenance().await;

    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording")?;
    assert_ne!(
        recording.phase,
        CapturePhase::Interrupted,
        "the Mac confirmed the finish and the daemon had it recorded: interrupting it would be a lie"
    );
    assert_ne!(
        recording.phase,
        CapturePhase::Finalizing,
        "nothing was pending, so the recording had to be finished"
    );
    assert_ne!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::RecordingCloseUnconfirmed),
        "that notice says something that did not happen"
    );
    Ok(())
}

/// A finish that is genuinely not ready is not forced through.
///
/// The other half, and the one it would be easy to lose: a recording is not
/// finalized while a chunk of it is unwritten. With a seal still in flight the
/// resume must decline, the recording must stay in `Finalizing`, and the
/// existing deadline must still interrupt it exactly as it does today.
#[tokio::test]
async fn a_finish_still_waiting_on_a_seal_is_not_forced_and_still_times_out() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    meeting_awaiting_only_its_finish(&mut runtime, &mut helper).await?;
    // A chunk the store has not acknowledged: the audio is not all written, so
    // the recording is not finished, whatever the helper said.
    runtime
        .capture
        .pending_seals
        .push_back(crate::capture::Segment {
            metadata: orbit_store::ChunkMetadata {
                capture_epoch_id: None,
                id: "pending-seal".into(),
                recording_id: "pending-seal".into(),
                track: "mic_local".into(),
                sequence: 0,
                sample_rate: 8_000,
                channels: 1,
                frames: 8_000,
                monotonic_ns: 0,
                first_sample: 0,
                started_at_ms: 1_000,
            },
            pcm: vec![7; 16_000],
        });

    runtime.resume_deferred_drain().await?;

    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording")?
            .phase,
        CapturePhase::Finalizing,
        "with a chunk unwritten the recording is not finished"
    );

    // And the deadline still does what it does today.
    finalizing_for(&mut runtime, orbit_core::FINALIZING_TIMEOUT_MS + 1_000);
    runtime.apply(Event::Tick).await?;

    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording")?
            .phase,
        CapturePhase::Interrupted,
        "a finish that really does not arrive still interrupts the recording"
    );
    assert_eq!(
        runtime.state.durable.notice,
        Some(
            orbit_core::notice::Notice {
                sequence: runtime
                    .state
                    .durable
                    .notice
                    .as_ref()
                    .and_then(|notice| notice.sequence),
                ..orbit_core::notice::Notice::new(
                    orbit_core::notice::NoticeCode::RecordingCloseUnconfirmed
                )
            }
            .with(orbit_core::notice::NoticeKey::Fragments, true)
        )
    );
    Ok(())
}
