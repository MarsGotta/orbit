//! One track of a meeting that fails, or changes device under a voice note,
//! never cuts the other: the helper declares it with a `gap` and the daemon
//! keeps the recording. Native traffic is synthetic; no helper or device.

use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::turn::said;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::{CapturePhase, Event};
use tokio::sync::mpsc;

fn frame(source: &str, stream: &str, sequence: u64, monotonic_ns: u64) -> Value {
    json!({"event":"audio", "source":source, "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":800, "sequence":sequence,
        "monotonic_ns":monotonic_ns,
        "pcm_s16le_base64":STANDARD.encode(vec![1; 1_600])})
}

fn gap(source: &str, stream: &str, reason: &str) -> Value {
    json!({"event":"gap", "source":source, "stream_id":stream, "reason":reason,
        "frames":0, "duration_unknown":true})
}

/// A meeting with the microphone and the system's audio, both acknowledged.
/// Returns (microphone stream, remote stream).
async fn two_track_meeting(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<(String, String)> {
    runtime
        .apply(Event::StartMeeting {
            id: "two-tracks".into(),
            title: "Reunión de prueba".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let mut microphone = None;
    let mut remote = None;
    while microphone.is_none() || remote.is_none() {
        let command = receiver.recv().await.context("audio_start absent")?;
        if command["command"] != "audio_start" {
            continue;
        }
        runtime
            .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":{"stream_id":command["stream_id"]}}))
            .await?;
        let stream = command["stream_id"]
            .as_str()
            .context("stream absent")?
            .to_owned();
        if command["source"] == "remote_app" {
            remote = Some(stream);
        } else {
            microphone = Some(stream);
        }
    }
    Ok((
        microphone.context("microphone absent")?,
        remote.context("remote absent")?,
    ))
}

fn phase(runtime: &Runtime) -> Option<CapturePhase> {
    runtime
        .state
        .durable
        .recording
        .as_ref()
        .map(|recording| recording.phase.clone())
}

/// The system's audio stream stopped and the helper started it again under
/// the same stream: its sequence starts over after the declared restart,
/// and the meeting goes on with both tracks.
#[tokio::test]
async fn a_restarted_system_audio_stream_continues_the_meeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let (microphone, remote) = two_track_meeting(&mut runtime, &mut receiver).await?;
    for sequence in 0..3 {
        runtime
            .native_event(frame("remote_app", &remote, sequence, 1_000 + sequence))
            .await?;
        runtime
            .native_event(frame("mic_local", &microphone, sequence, 1_000 + sequence))
            .await?;
    }
    runtime
        .native_event(gap("remote_app", &remote, "system_audio_restarted"))
        .await?;
    runtime
        .native_event(frame("remote_app", &remote, 0, 5_000))
        .await?;
    runtime
        .native_event(frame("mic_local", &microphone, 3, 5_000))
        .await?;
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    assert!(!said::has(&runtime.state, NoticeCode::CaptureCut));
    Ok(())
}

/// The system's audio stopped for good: the helper declares that track
/// stopped, the meeting goes on with the microphone, and the person hears
/// which track stopped.
#[tokio::test]
async fn a_stopped_system_audio_track_leaves_the_microphone_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let (microphone, remote) = two_track_meeting(&mut runtime, &mut receiver).await?;
    runtime
        .native_event(frame("remote_app", &remote, 0, 1_000))
        .await?;
    runtime
        .native_event(frame("mic_local", &microphone, 0, 1_000))
        .await?;
    runtime
        .native_event(gap("remote_app", &remote, "track_stopped"))
        .await?;
    for sequence in 1..4 {
        runtime
            .native_event(frame("mic_local", &microphone, sequence, 1_000 + sequence))
            .await?;
    }
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    assert!(!said::has(&runtime.state, NoticeCode::CaptureCut));
    assert!(said::has(&runtime.state, NoticeCode::TrackStopped));
    assert!(said::mentions(&runtime.state, "remote_app"));
    Ok(())
}

/// The microphone stopped for good (its conversion failed): the system's
/// audio keeps recording and the person hears it was the microphone.
#[tokio::test]
async fn a_stopped_microphone_track_leaves_the_system_audio_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let (microphone, remote) = two_track_meeting(&mut runtime, &mut receiver).await?;
    runtime
        .native_event(gap("mic_local", &microphone, "track_stopped"))
        .await?;
    for sequence in 0..3 {
        runtime
            .native_event(frame("remote_app", &remote, sequence, 1_000 + sequence))
            .await?;
    }
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    assert!(said::has(&runtime.state, NoticeCode::TrackStopped));
    assert!(said::mentions(&runtime.state, "mic_local"));
    Ok(())
}

/// AirPods connect while the person dictates a voice note over a meeting:
/// the helper moves the microphone to the new device, the note is cancelled
/// and said so, and the meeting goes on.
#[tokio::test]
async fn a_device_change_under_a_voice_note_cancels_the_note_and_keeps_the_meeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let (microphone, _remote) = two_track_meeting(&mut runtime, &mut receiver).await?;
    runtime
        .native_event(frame("mic_local", &microphone, 0, 1_000))
        .await?;
    runtime.apply(Event::VoiceStart).await?;
    assert!(runtime.state.durable.voice_active, "the voice note started");
    runtime
        .native_event(frame("mic_local", &microphone, 1, 2_000))
        .await?;
    runtime
        .native_event(gap("mic_local", &microphone, "audio_device_changed"))
        .await?;
    runtime
        .native_event(frame("mic_local", &microphone, 0, 9_000))
        .await?;
    assert!(
        !runtime.state.durable.voice_active,
        "the voice note under the device change is cancelled"
    );
    assert!(said::has(
        &runtime.state,
        NoticeCode::VoiceNoteCancelledByAudioRoute
    ));
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    assert!(!said::has(&runtime.state, NoticeCode::CaptureCut));
    Ok(())
}
