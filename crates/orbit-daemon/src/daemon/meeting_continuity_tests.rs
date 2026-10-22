//! A meeting is not lost to a cut: a declared gap keeps the recording, a cut
//! the person did not ask for resumes (or closes) on its own, and a late drain
//! receipt still closes a finished meeting. Synthetic audio and helper only.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::daemon::audio::tests::fixture;
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn frame(stream: &str, sequence: u64, frames: usize) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":frames, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(100_000_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
}

pub(super) async fn start(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: "reunion-sintetica".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60 * 60_000,
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

/// A helper's answer to `status` with the microphone authorized.
pub(super) fn granted_status(id: &Value) -> Value {
    json!({"event":"response","id":id,"ok":true,
        "result":{"permissions":{"microphone":"authorized","screen":true,
            "accessibility":false,"calendars":"not_determined"}}})
}

pub(super) fn phase(runtime: &Runtime) -> Option<orbit_core::CapturePhase> {
    runtime
        .state
        .durable
        .recording
        .as_ref()
        .map(|recording| recording.phase.clone())
}

/// The helper and the daemon's bridge drop audio on purpose under
/// backpressure and declare a `gap` for it (`daemon_backpressure`,
/// `audio_ring_overflow`). The frame after it carries the producer's next
/// sequence: it is accepted, the meeting goes on, and the hole is recorded
/// with its duration. It once cut the whole recording (`capture_event_failed`).
#[tokio::test]
async fn a_declared_gap_does_not_cut_the_recording_at_the_next_frame() -> Result<()> {
    for reason in ["daemon_backpressure", "audio_ring_overflow"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let stream = start(&mut runtime, &mut receiver).await?;
        runtime.handle_native_event(frame(&stream, 0, 800)).await;
        runtime.handle_native_event(frame(&stream, 1, 800)).await;
        assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Recording));
        // Frames 2..=4 were dropped by whoever was behind and declared here.
        runtime
            .handle_native_event(json!({"v":1,"event":"gap","source":"mic_local",
                "stream_id":stream,"reason":reason,"frames":2_400}))
            .await;
        assert_eq!(
            phase(&runtime),
            Some(orbit_core::CapturePhase::Recording),
            "{reason}: the gap alone keeps the recording"
        );
        // The hole is recorded with its duration: 2 400 samples at 8 kHz.
        let notice = runtime.state.durable.notice.clone().context("gap unsaid")?;
        assert_eq!(notice.code, orbit_core::notice::NoticeCode::AudioGap);
        assert_eq!(
            notice.data.get(&orbit_core::notice::NoticeKey::DurationMs),
            Some(&orbit_core::notice::DataValue::Int(300)),
            "{reason}: the gap's duration is recorded"
        );
        runtime.handle_native_event(frame(&stream, 5, 800)).await;
        assert_eq!(
            phase(&runtime),
            Some(orbit_core::CapturePhase::Recording),
            "{reason}: the first frame after a declared gap cut the recording; notice: {:?}",
            runtime.state.durable.notice
        );
    }
    Ok(())
}

/// A cut the person did not ask for never leaves the meeting open waiting
/// for «Terminar»: here the helper dies mid-meeting and comes back, and the
/// same recording goes on (or, failing that, would be closed).
#[tokio::test]
async fn a_cut_meeting_is_resumed_or_closed_without_anyone_acting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":[]}))
        .await;
    let mut reopened = false;
    for _ in 0..5 {
        runtime.run_maintenance().await;
        while let Ok(command) = receiver.try_recv() {
            reopened |= command["command"] == "audio_start";
            // The relaunched helper answers its status with its permissions.
            if command["command"] == "status" {
                runtime
                    .handle_native_event(granted_status(&command["id"]))
                    .await;
            }
        }
    }
    let closed = runtime
        .reader
        .get_job("finalize-reunion-sintetica")?
        .is_some();
    assert!(
        reopened || closed,
        "the meeting stays {:?} with {} chunks: not resumed, not closed; notice {:?}",
        phase(&runtime),
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map_or(0, |recording| recording.sealed_chunks),
        runtime.state.durable.notice
    );
    Ok(())
}

/// «Terminar» whose drain receipt arrives after `FINALIZING_TIMEOUT_MS`
/// (15 s; a stalled daemon or helper): the core Tick leaves the recording
/// interrupted, and once its late tail is sealed the daemon closes it, so the
/// hub gets its closure and writes its note.
#[tokio::test]
async fn a_finished_meeting_whose_drain_is_late_is_still_closed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    runtime.command(Command::MeetingFinish).await?;
    let stop = loop {
        let command = receiver.recv().await.context("audio_stop absent")?;
        if command["command"] == "audio_stop" {
            break command;
        }
    };
    // The drain receipt is late: move the finish 16 s into the past.
    let mut recording = serde_json::to_value(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?,
    )?;
    let since = recording["finalizing_since_ms"]
        .as_i64()
        .context("not finalizing")?;
    recording["finalizing_since_ms"] = json!(since - 16_000);
    crate::daemon::authority_fixture::set(&mut runtime.state, "recording", recording);
    runtime.run_maintenance().await;
    // The helper's receipt arrives now, with the tail of the meeting.
    runtime.handle_native_event(frame(&stream, 2, 4_000)).await;
    runtime
        .handle_native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    for _ in 0..3 {
        runtime.run_maintenance().await;
    }
    let closed = runtime
        .reader
        .get_job("finalize-reunion-sintetica")?
        .is_some();
    assert!(
        closed,
        "«Terminar» was pressed and the meeting is left {:?}, never closed; notice {:?}",
        phase(&runtime),
        runtime.state.durable.notice
    );
    Ok(())
}

/// The Mac going to sleep is one of the four things that end a capture: the
/// meeting is not resumed but closed, at the end of what was recorded, so the
/// hub writes its note, and the closure says its cause.
#[tokio::test]
async fn a_meeting_cut_by_the_mac_going_to_sleep_is_closed_and_said() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    runtime
        .handle_native_event(
            json!({"event":"capture_stopped","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":true}),
        )
        .await;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "capture_stop" {
            runtime
                .handle_native_event(json!({"event":"response","id":command["id"],"ok":true,
                    "result":{"drain_complete":true,"privacy_stopped":false}}))
                .await;
        }
    }
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    for _ in 0..3 {
        runtime.run_maintenance().await;
    }
    while let Ok(command) = receiver.try_recv() {
        assert_ne!(
            command["command"], "audio_start",
            "a sleeping Mac is not reopened"
        );
    }
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    let ended_at_ms = recording
        .ended_at_ms
        .context("the cut meeting stayed open")?;
    assert!(ended_at_ms >= recording.started_at_ms && ended_at_ms <= now());
    let closure = runtime
        .reader
        .get_job("finalize-reunion-sintetica")?
        .context("the closure is not on the Mac")?;
    assert_eq!(closure.payload["manifest"]["finalized_at_ms"], ended_at_ms);
    let closed = runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .find(|notice| notice.code == orbit_core::notice::NoticeCode::RecordingClosedAfterCut)
        .cloned()
        .context("the closure is not said")?;
    assert_eq!(
        closed.data.get(&orbit_core::notice::NoticeKey::Cause),
        Some(&orbit_core::notice::DataValue::Token(
            orbit_core::notice::Token::new("session_inactive").context("token")?
        ))
    );
    Ok(())
}

/// The daemon died with the meeting live (a panic, or the process ended):
/// the next daemon finds it in its snapshot, and once its helper is ready
/// the same recording goes on, after the chunks already on the Mac.
#[tokio::test]
async fn a_meeting_live_when_the_daemon_died_goes_on_after_the_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    // What a new daemon starts from: the saved state, recovered, and
    // nothing of the old process's capture.
    let saved = runtime.state.durable.clone();
    runtime.capture.recovery =
        crate::daemon::cut_recovery::recovery_after_restart(saved.recording.as_ref());
    runtime.state.durable = saved.recovered();
    runtime.capture.buffer = crate::capture::CaptureBuffer::default();
    runtime.capture.mic_stream = None;
    runtime.capture.pending_starts.clear();
    while receiver.try_recv().is_ok() {}
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Interrupted));
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":[]}))
        .await;
    let mut reopened = None;
    for _ in 0..3 {
        runtime.run_maintenance().await;
        while let Ok(command) = receiver.try_recv() {
            if command["command"] == "status" {
                runtime
                    .handle_native_event(granted_status(&command["id"]))
                    .await;
            }
            if command["command"] == "audio_start" {
                reopened = Some(command);
            }
        }
    }
    let reopened = reopened.context("the meeting did not go on after the restart")?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|recording| recording.id.as_str()),
        Some("reunion-sintetica"),
        "the same recording goes on"
    );
    runtime
        .native_event(json!({"event":"response","id":reopened["id"],"ok":true,
            "result":{"stream_id":reopened["stream_id"]}}))
        .await?;
    let resumed = reopened["stream_id"].as_str().context("stream")?.to_owned();
    runtime.handle_native_event(frame(&resumed, 0, 8_000)).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    assert_eq!(phase(&runtime), Some(orbit_core::CapturePhase::Recording));
    assert!(
        runtime
            .reader
            .chunk_receipt("reunion-sintetica-mic_local-2")?
            .is_some(),
        "the resumed audio continues the recording's chunks"
    );
    Ok(())
}
