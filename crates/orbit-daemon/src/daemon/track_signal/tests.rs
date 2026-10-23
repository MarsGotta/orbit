//! Real actor/capture/store, synthetic mono PCM and injected reception time.
//! These tests never start a native capture, a provider or a physical clock.
use super::*;
use crate::daemon::{
    audio::tests::{fixture, settle_seal_receipts},
    topics::{TopicBoard, TopicSource},
};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{CapturePhase, Event, State, audio_signal::SignalCondition};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

async fn start(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    remote: bool,
) -> Result<()> {
    runtime
        .apply(Event::StartMeeting {
            id: "signal-meeting".into(),
            title: "Synthetic signal fixture".into(),
            audio: if remote {
                orbit_core::protocol::MeetingAudio::Application {
                    application_pid: 123,
                }
            } else {
                orbit_core::protocol::MeetingAudio::MicrophoneOnly
            },
        })
        .await?;
    for _ in 0..if remote { 2 } else { 1 } {
        let command = receiver.recv().await.context("missing start")?;
        assert_eq!(command["command"], "audio_start");
        runtime
            .native_event(json!({"event":"response", "id":command["id"],
                "ok":true, "result":{"stream_id":command["stream_id"]}}))
            .await?;
    }
    Ok(())
}

fn frame(source: &str, stream: &str, sequence: u64, sample: i16) -> Value {
    let pcm: Vec<u8> = (0..8_000).flat_map(|_| sample.to_le_bytes()).collect();
    json!({"event":"audio", "source":source, "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":8000, "sequence":sequence,
        "monotonic_ns":sequence * 1_000_000_000, "clock_domain":"mach_absolute_time",
        "pcm_s16le_base64":STANDARD.encode(pcm)})
}

fn signal(runtime: &Runtime) -> Result<RecordingSignalView> {
    runtime
        .recording_signal_view()
        .context("missing live projection")
}

/// The projection as of `delivered`, the instant the frames arrived.
/// `Receiving` is an age on the real clock, and waiting for the store's
/// receipts must not age the samples an assertion is about: under load the
/// wait for twenty receipts alone passed the three-second delivery timeout.
fn signal_as_delivered(runtime: &Runtime, delivered: Instant) -> Result<RecordingSignalView> {
    runtime
        .recording_signal_view_at(delivered)
        .context("missing live projection")
}

#[tokio::test]
async fn projection_requires_each_requested_tracks_current_owner() -> Result<()> {
    for source in ["mic_local", "remote_app"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        start(&mut runtime, &mut receiver, true).await?;
        assert!(runtime.recording_signal_view().is_some());
        // Synthetic invariant probe, not a claimed reachable lifecycle: remove
        // only one capture owner while leaving phase, epoch and streams intact.
        assert!(runtime.capture.buffer.pause_source(source)?.is_empty());
        assert!(runtime.recording_signal_view().is_none(), "{source}");
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
        assert!(receiver.try_recv().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn awaiting_is_not_receiving_and_unrequested_remote_is_not_missing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let observed = signal(&runtime)?;
    assert_eq!(
        observed.mic_local.condition,
        SignalCondition::AwaitingSamples
    );
    assert_eq!(observed.mic_local.received_frames, 0);
    assert_eq!(observed.mic_local.since_last_samples_ms, None);
    assert_eq!(observed.remote_app, None);
    let stream = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    runtime
        .native_event(frame("mic_local", &stream, 0, 500))
        .await?;
    let delivered = Instant::now();
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(
        signal_as_delivered(&runtime, delivered)?
            .mic_local
            .condition,
        SignalCondition::Receiving
    );
    Ok(())
}

#[tokio::test]
async fn low_signal_is_independent_per_track_and_original_pcm_is_unchanged() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, true).await?;
    let mic = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    let remote = runtime
        .capture
        .remote_stream
        .clone()
        .context("remote absent")?;
    for sequence in 0..10 {
        runtime
            .native_event(frame("mic_local", &mic, sequence, 500))
            .await?;
        runtime
            .native_event(frame("remote_app", &remote, sequence, 0))
            .await?;
    }
    let delivered = Instant::now();
    settle_seal_receipts(&mut runtime).await?;
    let observed = signal_as_delivered(&runtime, delivered)?;
    assert_eq!(observed.mic_local.condition, SignalCondition::Receiving);
    let quiet = observed.remote_app.context("remote observation absent")?;
    assert_eq!(quiet.condition, SignalCondition::LowSignal);
    assert_eq!(quiet.low_signal_ms, 10_000);
    assert_eq!(quiet.received_frames, 80_000);
    let mut topics = TopicBoard::new("synthetic-epoch".into());
    let channels = topics.channels();
    let topic_receiver = channels[&orbit_core::topics::TopicId::Recording].subscribe();
    topics.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let topic = serde_json::to_value(topic_receiver.borrow().data.clone())?;
    assert_eq!(topic["knowledge"]["state"], "present");
    let signal = &topic["knowledge"]["value"]["recording_signal"];
    assert_eq!(signal["mic_local"]["received_frames"], 80_000);
    assert_eq!(signal["remote_app"]["received_frames"], 80_000);
    assert!(!runtime.state.privacy_stopped());
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
    assert!(
        receiver.try_recv().is_err(),
        "observation must not issue a native command"
    );
    let chunks = runtime
        .fixture_store
        .list_chunks("signal-meeting", None, 100)?;
    assert_eq!(chunks.len(), 20);
    for chunk in chunks {
        let sample: i16 = if chunk.metadata.track == "mic_local" {
            500
        } else {
            0
        };
        let expected: Vec<u8> = (0..8_000).flat_map(|_| sample.to_le_bytes()).collect();
        assert_eq!(
            runtime.fixture_store.read_chunk(&chunk.metadata.id)?,
            crate::native::wav(&expected, 8_000, 1)?
        );
    }
    runtime
        .native_event(frame("remote_app", &remote, 10, 500))
        .await?;
    let delivered = Instant::now();
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(
        signal_as_delivered(&runtime, delivered)?
            .remote_app
            .context("remote absent")?
            .condition,
        SignalCondition::Receiving
    );
    Ok(())
}

#[tokio::test]
async fn stale_delivery_is_a_projection_not_a_gap_or_automatic_stop() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, true).await?;
    let mic = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    runtime.native_event(frame("mic_local", &mic, 0, 0)).await?;
    settle_seal_receipts(&mut runtime).await?;
    let original = runtime.state.clone();
    let observed = runtime
        .recording_signal_view_at(Instant::now() + Duration::from_secs(3))
        .context("projection absent")?;
    assert_eq!(
        observed.mic_local.condition,
        SignalCondition::DeliveryDelayed
    );
    assert!(
        observed
            .mic_local
            .since_last_samples_ms
            .is_some_and(|age| age >= 3_000)
    );
    let remote = observed.remote_app.context("remote absent")?;
    assert_eq!(remote.condition, SignalCondition::DeliveryDelayed);
    assert_eq!(remote.since_last_samples_ms, None);
    assert_eq!(runtime.state, original);
    assert!(receiver.try_recv().is_err());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    Ok(())
}

#[tokio::test]
async fn rejected_or_old_stream_pcm_cannot_refresh_an_observation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    runtime
        .native_event(frame("mic_local", "stale-stream", 0, 500))
        .await?;
    assert_eq!(signal(&runtime)?.mic_local.received_frames, 0);
    let stream = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    let mut invalid = frame("mic_local", &stream, 0, 500);
    invalid["pcm_s16le_base64"] = "invalid".into();
    assert!(runtime.native_event(invalid).await.is_err());
    assert_eq!(signal(&runtime)?.mic_local.received_frames, 0);
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
    Ok(())
}

#[tokio::test]
async fn paused_meeting_is_not_kept_live_by_shared_ptt_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let mic = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    runtime
        .native_event(frame("mic_local", &mic, 0, 500))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    runtime.apply(Event::VoiceStart).await?;
    runtime.apply(Event::PauseMeeting).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Paused
    );
    let before = runtime
        .capture
        .audio
        .meeting
        .signal
        .as_ref()
        .context("monitor absent")?
        .microphone
        .clone();
    runtime.native_event(frame("mic_local", &mic, 1, 0)).await?;
    assert!(runtime.recording_signal_view().is_none());
    assert_eq!(
        runtime
            .capture
            .audio
            .meeting
            .signal
            .as_ref()
            .context("monitor absent")?
            .microphone,
        before
    );
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("PTT absent")?
            .pcm
            .len(),
        16_000
    );
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    Ok(())
}

#[tokio::test]
async fn stopping_preserves_late_tail_without_presenting_it_as_live() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let mic = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    runtime
        .native_event(frame("mic_local", &mic, 0, 500))
        .await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    runtime.native_event(frame("mic_local", &mic, 1, 0)).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.recording_signal_view().is_none());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 2);
    runtime
        .native_event(json!({"event":"response", "id":stop["id"],
        "ok":true, "result":{"drain_complete":true}}))
        .await?;
    assert!(runtime.recording_signal_view().is_none());
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .sealed_chunks,
        2
    );
    Ok(())
}

#[tokio::test]
async fn projection_never_persists_and_recovery_clears_a_supplied_old_view() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let mut projected = runtime.state.clone();
    projected.views.recording_signal = runtime.recording_signal_view();
    assert!(projected.views.recording_signal.is_some());
    assert!(runtime.state.views.recording_signal.is_none());
    let stored: State = runtime
        .fixture_store
        .load_snapshot()?
        .context("snapshot absent")?;
    assert!(stored.views.recording_signal.is_none());
    let round_trip: State = serde_json::from_slice(&serde_json::to_vec(&projected)?)?;
    assert!(round_trip.views.recording_signal.is_some());
    assert!(round_trip.recovered().views.recording_signal.is_none());
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    assert!(runtime.recording_signal_view().is_none());
    Ok(())
}

#[tokio::test]
async fn replaced_epoch_or_recording_identity_cannot_borrow_old_measurements() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let mic = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    let epoch = runtime
        .capture
        .audio
        .meeting
        .signal
        .as_ref()
        .context("monitor absent")?
        .capture_epoch_id
        .clone();
    runtime
        .capture
        .audio
        .meeting
        .signal
        .as_mut()
        .context("monitor absent")?
        .capture_epoch_id = "old-epoch".into();
    runtime.native_event(frame("mic_local", &mic, 0, 0)).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.recording_signal_view().is_none());
    assert_eq!(
        runtime
            .capture
            .audio
            .meeting
            .signal
            .as_ref()
            .context("monitor absent")?
            .microphone
            .view(0)
            .received_frames,
        0
    );
    runtime
        .capture
        .audio
        .meeting
        .signal
        .as_mut()
        .context("monitor absent")?
        .capture_epoch_id = epoch;
    runtime
        .capture
        .audio
        .meeting
        .signal
        .as_mut()
        .context("monitor absent")?
        .recording_id = "old-meeting".into();
    runtime.native_event(frame("mic_local", &mic, 1, 0)).await?;
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.recording_signal_view().is_none());
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 2);
    Ok(())
}

#[tokio::test]
async fn resume_starts_a_new_observation_and_old_stream_cannot_update_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let old_stream = runtime
        .capture
        .mic_stream
        .clone()
        .context("microphone absent")?;
    runtime
        .native_event(frame("mic_local", &old_stream, 0, 0))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let old_epoch = signal(&runtime)?.capture_epoch_id;
    runtime.apply(Event::PauseMeeting).await?;
    let stop = receiver.recv().await.context("stop absent")?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"],
        "ok":true, "result":{"drain_complete":true}}))
        .await?;
    runtime.apply(Event::ResumeMeeting).await?;
    let new_epoch = signal(&runtime)?.capture_epoch_id;
    assert_ne!(old_epoch, new_epoch);
    runtime
        .native_event(frame("mic_local", &old_stream, 1, 0))
        .await?;
    assert_eq!(signal(&runtime)?.mic_local.received_frames, 0);
    assert_eq!(
        signal(&runtime)?.mic_local.condition,
        SignalCondition::AwaitingSamples
    );
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    Ok(())
}

#[tokio::test]
async fn impossible_receive_time_is_not_presented_as_a_current_observation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start(&mut runtime, &mut receiver, false).await?;
    let origin = runtime
        .capture
        .audio
        .meeting
        .signal
        .as_ref()
        .context("monitor absent")?
        .started_at;
    let before = origin
        .checked_sub(Duration::from_millis(1))
        .context("clock underflow")?;
    assert!(runtime.recording_signal_view_at(before).is_none());
    Ok(())
}
