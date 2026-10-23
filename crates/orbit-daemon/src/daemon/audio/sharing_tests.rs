//! The meeting owns the microphone: what else consumes it over a recording
//! (a voice note, a question to the assistant by voice, a call that asks to
//! open) leaves the recording's chunks and their timings exactly as a
//! recording without it, whether that finishes, fails or is cancelled.
//! Each case records the same synthetic frames as a meeting alone and the
//! two chunk lists are compared. Native traffic and the hub are synthetic.

use crate::daemon::Runtime;
use crate::daemon::audio::tests::{fixture, settle_seal_receipts, take_helper_command};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::protocol::Command;
use orbit_core::{CapturePhase, Event, protocol::MeetingAudio};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const RECORDING: &str = "reunion-compartida";
/// Ten frames of 100 ms: one chunk of a second per stretch.
const STRETCH: u64 = 10;

/// What consumes the microphone between the meeting's first and last
/// stretch of frames.
#[derive(Clone, Copy, Debug)]
enum Interlude {
    Nothing,
    /// A note sent for transcription while paired.
    VoiceNoteSent,
    /// A note whose transcription fails: this Mac is not paired.
    VoiceNoteUnpaired,
    /// Esc: the note is kept and never sent.
    VoiceNoteCancelled,
    /// «Oye Deimos, …» heard and sent as a turn about the meeting.
    QuestionSent,
    /// «Oye Deimos, …» heard and the turn cannot leave (no hub link).
    QuestionNotSent,
    /// «Oye Deimos» heard and the question abandoned by the helper.
    QuestionAbandoned,
    /// A call with the assistant asked for over the recording.
    CallRefused,
}

/// One chunk as the recording keeps it, without its random ids; its start
/// time is taken from the first chunk's, since the wall clock moves on.
#[derive(Debug, PartialEq, Eq)]
struct Chunk {
    track: String,
    sequence: u64,
    frames: u64,
    first_sample: u64,
    monotonic_ns: u64,
    started_after_first_ms: i64,
    sha256: String,
}

/// A frame of 100 ms of samples. Each carries its own sample value, so a
/// frame that lands in another chunk changes that chunk's digest. Its capture
/// clock moves a microsecond a frame: a chunk's start is the clock's anchor
/// plus the capture delta, never later than its receipt, and synthetic frames
/// arrive far faster than 100 ms apart. With a delta under a millisecond,
/// every chunk of a recording whose anchor holds starts at that anchor.
fn frame(stream: &str, sequence: u64) -> Value {
    let sample = u8::try_from(sequence % 200).unwrap_or(0).saturating_add(1);
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":800, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(1_000),
        "pcm_s16le_base64":STANDARD.encode(vec![sample; 1_600])})
}

async fn frames(runtime: &mut Runtime, stream: &str, from: u64) -> Result<()> {
    for sequence in from..from + STRETCH {
        runtime.native_event(frame(stream, sequence)).await?;
    }
    Ok(())
}

async fn interlude(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    stream: &str,
    what: Interlude,
) -> Result<()> {
    match what {
        Interlude::Nothing | Interlude::CallRefused => {
            if matches!(what, Interlude::CallRefused) {
                let refused = runtime
                    .apply(Event::CallStart {
                        id: "llamada-sintetica".into(),
                        duration_ms: 60_000,
                        origin: orbit_core::voice::CallOrigin::App,
                    })
                    .await;
                ensure!(refused.is_err(), "a call never opens over a recording");
            }
            frames(runtime, stream, STRETCH).await
        }
        Interlude::VoiceNoteSent | Interlude::VoiceNoteUnpaired | Interlude::VoiceNoteCancelled => {
            if matches!(what, Interlude::VoiceNoteSent) {
                runtime.token = Some("token-sintetico".into());
            }
            runtime.apply(Event::VoiceStart).await?;
            take_helper_command(helper, "playback_stop")?;
            frames(runtime, stream, STRETCH).await?;
            let end = if matches!(what, Interlude::VoiceNoteCancelled) {
                Command::VoiceCancel
            } else {
                Command::VoiceEnd
            };
            let ended = runtime.command(end).await;
            // Unpaired, the transcription fails and says so; the note stays.
            ensure!(
                ended.is_err() == matches!(what, Interlude::VoiceNoteUnpaired),
                "{what:?}: {ended:?}"
            );
            ensure!(!runtime.state.durable.voice.voice_active);
            Ok(())
        }
        Interlude::QuestionSent | Interlude::QuestionNotSent | Interlude::QuestionAbandoned => {
            let (hub, _frames) = mpsc::channel(16);
            if matches!(what, Interlude::QuestionSent) {
                runtime.hub.install(hub);
            }
            frames(runtime, stream, STRETCH).await?;
            let heard = if matches!(what, Interlude::QuestionAbandoned) {
                json!({"v":1,"event":"notice","code":"wake_phrase_abandoned",
                    "abandonment":{"cause":"question_wait_expired","elapsed_ms":7_000,"silent_ms":7_000}})
            } else {
                json!({"v":1,"event":"assistant_question","stream_id":stream,
                    "text":"¿Qué se decidió sobre el presupuesto?",
                    "started_ms":1_000,"ended_ms":2_000})
            };
            let _ = runtime.native_event(heard).await;
            Ok(())
        }
    }
}

/// A microphone-only meeting of three stretches with `what` in the middle,
/// finished and drained; its chunks as the store keeps them.
async fn record(what: Interlude) -> Result<Vec<Chunk>> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: RECORDING.into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let start = take_helper_command(&mut helper, "audio_start")?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    frames(&mut runtime, &stream, 0).await?;
    interlude(&mut runtime, &mut helper, &stream, what).await?;
    // A clock anchored again after the interlude would start the last chunk
    // at least this much later than the anchor.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    frames(&mut runtime, &stream, 2 * STRETCH).await?;
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("the meeting goes on")?;
    ensure!(
        recording.phase == CapturePhase::Recording && recording.gaps == 0,
        "{what:?}: the meeting kept recording without a gap: {recording:?}"
    );
    // Nothing but the meeting's own finish may stop its microphone.
    while let Ok(command) = helper.try_recv() {
        ensure!(
            !matches!(
                command["command"].as_str(),
                Some("audio_stop" | "capture_stop" | "privacy_stop")
            ),
            "{what:?} stopped the meeting's capture: {command}"
        );
    }
    runtime.apply(Event::FinishMeeting).await?;
    let stop = loop {
        let command = helper.try_recv().context("audio_stop absent")?;
        if command["command"] == "audio_stop" {
            break command;
        }
    };
    ensure!(stop["stream_id"] == stream.as_str(), "{what:?}: {stop}");
    runtime
        .native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    chunks(&runtime)
}

fn chunks(runtime: &Runtime) -> Result<Vec<Chunk>> {
    let mut sealed = runtime.fixture_store.list_chunks(RECORDING, None, 64)?;
    sealed.sort_by_key(|chunk| (chunk.metadata.track.clone(), chunk.metadata.sequence));
    let first = sealed
        .first()
        .map(|chunk| chunk.metadata.started_at_ms)
        .context("no chunk was sealed")?;
    Ok(sealed
        .into_iter()
        .map(|chunk| Chunk {
            track: chunk.metadata.track,
            sequence: chunk.metadata.sequence,
            frames: chunk.metadata.frames,
            first_sample: chunk.metadata.first_sample,
            monotonic_ns: chunk.metadata.monotonic_ns,
            started_after_first_ms: chunk.metadata.started_at_ms - first,
            sha256: chunk.sha256,
        })
        .collect())
}

#[tokio::test]
async fn what_consumes_the_meetings_microphone_leaves_its_chunks_and_timings_untouched()
-> Result<()> {
    let alone = record(Interlude::Nothing).await?;
    assert_eq!(alone.len(), 3, "three seconds, three chunks: {alone:?}");
    assert!(
        alone.iter().all(|chunk| chunk.started_after_first_ms == 0),
        "the capture clock times every chunk: {alone:?}"
    );
    for what in [
        Interlude::VoiceNoteSent,
        Interlude::VoiceNoteUnpaired,
        Interlude::VoiceNoteCancelled,
        Interlude::QuestionSent,
        Interlude::QuestionNotSent,
        Interlude::QuestionAbandoned,
        Interlude::CallRefused,
    ] {
        assert_eq!(record(what).await?, alone, "{what:?}");
    }
    Ok(())
}
