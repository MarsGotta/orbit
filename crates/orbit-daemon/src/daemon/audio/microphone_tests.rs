//! Who holds the microphone, as the helper is told it. The meeting owns the
//! microphone while it records: a voice note dictated over it consumes the
//! same stream and never changes its holder, and a meeting that starts while
//! a note runs takes the microphone over. The helper needs that fact to keep
//! a meeting through a change of audio device (AirPods connecting): with the
//! microphone held by a meeting it moves it to the new device and only the
//! note ends; held by a lone note, it stops. Native traffic is synthetic.

use crate::daemon::Runtime;
use crate::daemon::audio::tests::{assert_no_helper_command, fixture, take_helper_command};
use crate::daemon::turn::said;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::{CapturePhase, Event, protocol::MeetingAudio};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// 100 ms of samples. After a new route the helper's sequence starts over;
/// its capture clock (`monotonic_ns`) never goes back.
fn frame(stream: &str, sequence: u64) -> Value {
    frame_at(stream, sequence, sequence.saturating_mul(100_000_000))
}

fn frame_at(stream: &str, sequence: u64, monotonic_ns: u64) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":800, "sequence":sequence,
        "monotonic_ns":monotonic_ns,
        "pcm_s16le_base64":STANDARD.encode(vec![1; 1_600])})
}

/// What the helper sends when it moved the microphone to the new device:
/// the new stream's first frame comes after this gap.
fn route_gap(stream: &str) -> Value {
    json!({"event":"gap", "source":"mic_local", "stream_id":stream,
        "reason":"audio_device_changed", "frames":0, "duration_unknown":true})
}

fn meeting(id: &str) -> Event {
    Event::StartMeeting {
        id: id.into(),
        title: "Reunión sintética".into(),
        audio: MeetingAudio::MicrophoneOnly,
    }
}

fn phase(runtime: &Runtime) -> Option<CapturePhase> {
    runtime
        .state
        .durable
        .recording
        .as_ref()
        .map(|recording| recording.phase.clone())
}

/// A voice note opens the microphone; the helper confirms it.
async fn voice_note_alone(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime.apply(Event::VoiceStart).await?;
    take_helper_command(helper, "playback_stop")?;
    let start = take_helper_command(helper, "audio_start")?;
    assert_eq!(start["purpose"], "voice_note");
    let stream = start["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    Ok(stream)
}

/// A microphone-only meeting opens the microphone; the helper confirms it.
async fn meeting_alone(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime.apply(meeting("reunion-sola")).await?;
    let start = take_helper_command(helper, "audio_start")?;
    assert_eq!(start["purpose"], "meeting");
    let stream = start["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    Ok(stream)
}

/// The note was begun before the meeting: the meeting that starts while it
/// runs takes the microphone, and the helper is told so on the same stream.
#[tokio::test]
async fn a_meeting_started_over_a_voice_note_takes_the_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = voice_note_alone(&mut runtime, &mut helper).await?;
    runtime.apply(meeting("reunion-tras-la-nota")).await?;
    let told = take_helper_command(&mut helper, "audio_purpose")?;
    assert_eq!(told["stream_id"], stream.as_str());
    assert_eq!(
        told["purpose"], "meeting",
        "the meeting holds the microphone"
    );
    assert_no_helper_command(&mut helper)?;
    Ok(())
}

/// The case the 0.11.0 notes missed: a note begun before the meeting, the
/// meeting started while it runs, and AirPods connecting with both active.
/// Only the note ends; the meeting keeps recording on the new device.
#[tokio::test]
async fn a_device_change_over_a_note_begun_before_the_meeting_only_cancels_the_note() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = voice_note_alone(&mut runtime, &mut helper).await?;
    runtime.native_event(frame(&stream, 0)).await?;
    runtime.apply(meeting("reunion-tras-la-nota")).await?;
    assert_eq!(
        take_helper_command(&mut helper, "audio_purpose")?["purpose"],
        "meeting"
    );
    for sequence in 1..=12 {
        runtime.native_event(frame(&stream, sequence)).await?;
    }
    runtime.native_event(route_gap(&stream)).await?;
    for sequence in 0..=12 {
        let monotonic_ns = 2_000_000_000 + sequence * 100_000_000;
        runtime
            .native_event(frame_at(&stream, sequence, monotonic_ns))
            .await?;
    }
    assert!(!runtime.state.durable.voice_active, "the note is cancelled");
    assert!(said::has(
        &runtime.state,
        NoticeCode::VoiceNoteCancelledByAudioRoute
    ));
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    assert!(!said::has(&runtime.state, NoticeCode::CaptureCut));
    while let Ok(command) = helper.try_recv() {
        assert!(
            !matches!(
                command["command"].as_str(),
                Some("capture_stop" | "privacy_stop" | "audio_stop")
            ),
            "nothing stops the meeting's microphone: {command}"
        );
    }
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("the meeting goes on")?;
    assert!(
        recording.sealed_chunks >= 2,
        "the meeting records before and after the new device: {} chunks",
        recording.sealed_chunks
    );
    Ok(())
}

/// A note dictated over a meeting consumes its microphone: the helper is
/// not told anything new when it starts or when it ends.
#[tokio::test]
async fn a_voice_note_over_a_meeting_never_changes_who_holds_the_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    meeting_alone(&mut runtime, &mut helper).await?;
    assert_no_helper_command(&mut helper)?;
    runtime.apply(Event::VoiceStart).await?;
    take_helper_command(&mut helper, "playback_stop")?;
    assert_no_helper_command(&mut helper)?;
    runtime.apply(Event::VoiceEnd).await?;
    assert_no_helper_command(&mut helper)?;
    assert_eq!(phase(&runtime), Some(CapturePhase::Recording));
    Ok(())
}

/// A meeting paused under a note gives the microphone back to the note, so
/// a change of device then ends only the note, as for a note alone.
#[tokio::test]
async fn a_meeting_paused_under_a_voice_note_gives_it_the_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = meeting_alone(&mut runtime, &mut helper).await?;
    runtime.apply(Event::VoiceStart).await?;
    take_helper_command(&mut helper, "playback_stop")?;
    runtime.apply(Event::PauseMeeting).await?;
    let told = take_helper_command(&mut helper, "audio_purpose")?;
    assert_eq!(told["stream_id"], stream.as_str());
    assert_eq!(told["purpose"], "voice_note");
    assert_no_helper_command(&mut helper)?;
    Ok(())
}
