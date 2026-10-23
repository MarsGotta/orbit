//! «Silenciar» on a meeting's microphone, through the app's order and the
//! helper's answers. The helper is synthetic: while muted it sends silence,
//! as `AudioCapture.swift` does, and these tests check what the daemon does
//! with that (both tracks kept whole and in time, the span in the closure),
//! never that a Mac produced it.
use crate::daemon::Runtime;
use crate::daemon::audio::tests::{assert_no_helper_command, fixture, settle_seal_receipts};
use crate::daemon::turn::said;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_core::protocol::{Command, MeetingAudio};
use orbit_core::{CapturePhase, Event};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// One second of 8 kHz audio: one chunk. `sample` 0 is the helper's silence.
fn second(source: &str, stream: &str, sequence: u64, sample: i16) -> Value {
    json!({"event":"audio", "source":source, "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":8000, "sequence":sequence,
        "monotonic_ns":sequence * 1_000_000_000,
        "pcm_s16le_base64":STANDARD.encode(sample.to_le_bytes().repeat(8000))})
}

/// The next order to the helper, past the `status` reads it gets anyway.
fn next_order(helper: &mut mpsc::Receiver<Value>) -> Result<Value> {
    loop {
        let command = helper.try_recv().context("helper order absent")?;
        if command["command"] != "status" {
            return Ok(command);
        }
    }
}

async fn acknowledge(runtime: &mut Runtime, order: &Value, result: Value) -> Result<()> {
    runtime
        .native_event(json!({"event":"response","id":order["id"],"ok":true,"result":result}))
        .await
}

/// A meeting recording both tracks, both confirmed: (microphone, remote).
async fn meeting(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    id: &str,
) -> Result<(String, String)> {
    runtime
        .apply(Event::StartMeeting {
            id: id.into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::System,
        })
        .await?;
    let (mut microphone, mut remote) = (None, None);
    for _ in 0..2 {
        let start = next_order(helper)?;
        assert_eq!(start["command"], "audio_start");
        assert!(
            start.get("muted").is_none(),
            "a new recording starts unmuted"
        );
        acknowledge(runtime, &start, json!({"stream_id":start["stream_id"]})).await?;
        let stream = start["stream_id"].as_str().context("stream")?.to_owned();
        if start["source"] == "remote_app" {
            remote = Some(stream);
        } else {
            microphone = Some(stream);
        }
    }
    Ok((
        microphone.context("microphone start")?,
        remote.context("remote start")?,
    ))
}

/// «Silenciar» (or «Reactivar») answered by the helper as it applied it.
async fn set_muted(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    microphone: &str,
    muted: bool,
) -> Result<()> {
    runtime.command(Command::MicrophoneMute { muted }).await?;
    let order = next_order(helper)?;
    assert_eq!(order["command"], "audio_mute");
    assert_eq!(order["stream_id"], microphone);
    assert_eq!(order["muted"], muted);
    assert_eq!(
        runtime.microphone_muted(),
        !muted,
        "nothing changes before the helper answers"
    );
    acknowledge(
        runtime,
        &order,
        json!({"stream_id":microphone,"muted":muted}),
    )
    .await?;
    assert_eq!(runtime.microphone_muted(), muted);
    Ok(())
}

fn pcm_of(runtime: &Runtime, id: &str) -> Result<Vec<u8>> {
    // A sealed chunk is a WAV file: its 44-byte header, then the samples.
    Ok(runtime.fixture_store.read_chunk(id)?[44..].to_vec())
}

#[tokio::test]
async fn a_muted_stretch_keeps_both_tracks_whole_and_closes_with_its_span() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (microphone, remote) = meeting(&mut runtime, &mut helper, "reunion-silenciada").await?;
    runtime
        .native_event(second("mic_local", &microphone, 0, 900))
        .await?;
    runtime
        .native_event(second("remote_app", &remote, 0, 700))
        .await?;
    set_muted(&mut runtime, &mut helper, &microphone, true).await?;
    // Muting changes nobody's hold on the microphone: the lane stays armed.
    runtime.settle_capture_lane().await;
    assert!(runtime.capture.lane.lock().gate.armed);
    runtime
        .native_event(second("mic_local", &microphone, 1, 0))
        .await?;
    runtime
        .native_event(second("remote_app", &remote, 1, 700))
        .await?;
    set_muted(&mut runtime, &mut helper, &microphone, false).await?;
    runtime
        .native_event(second("mic_local", &microphone, 2, 900))
        .await?;
    runtime
        .native_event(second("remote_app", &remote, 2, 700))
        .await?;
    runtime.apply(Event::FinishMeeting).await?;
    for _ in 0..2 {
        let stop = next_order(&mut helper)?;
        assert_eq!(stop["command"], "audio_stop");
        acknowledge(
            &mut runtime,
            &stop,
            json!({"stream_id":stop["stream_id"],"drain_complete":true}),
        )
        .await?;
    }
    settle_seal_receipts(&mut runtime).await?;
    let mut chunks = runtime
        .fixture_store
        .list_chunks("reunion-silenciada", None, 20)?;
    chunks.sort_by_key(|chunk| (chunk.metadata.track.clone(), chunk.metadata.sequence));
    let shape: Vec<(String, u64, u64)> = chunks
        .iter()
        .map(|chunk| {
            let metadata = &chunk.metadata;
            (metadata.track.clone(), metadata.sequence, metadata.frames)
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("mic_local".to_owned(), 0, 8000),
            ("mic_local".to_owned(), 1, 8000),
            ("mic_local".to_owned(), 2, 8000),
            ("remote_app".to_owned(), 0, 8000),
            ("remote_app".to_owned(), 1, 8000),
            ("remote_app".to_owned(), 2, 8000),
        ],
        "both tracks last the same, with no gap"
    );
    let muted = pcm_of(&runtime, &chunks[1].metadata.id)?;
    assert!(
        muted.iter().all(|byte| *byte == 0),
        "the muted second is silence"
    );
    assert!(
        pcm_of(&runtime, &chunks[0].metadata.id)?
            .iter()
            .any(|byte| *byte != 0)
    );
    assert!(
        pcm_of(&runtime, &chunks[4].metadata.id)?
            .iter()
            .any(|byte| *byte != 0),
        "the other side of the call kept recording"
    );
    assert_eq!(
        chunks[1].metadata.monotonic_ns - chunks[0].metadata.monotonic_ns,
        chunks[4].metadata.monotonic_ns - chunks[3].metadata.monotonic_ns,
        "the tracks keep their time"
    );
    let closure = runtime
        .fixture_store
        .get_job("finalize-reunion-silenciada")?
        .context("closure absent")?;
    let manifest = &closure.payload["manifest"];
    let spans = manifest["muted_spans"].as_array().context("muted spans")?;
    assert_eq!(spans.len(), 1, "{manifest}");
    let length = manifest["finalized_at_ms"]
        .as_i64()
        .zip(manifest["started_at_ms"].as_i64())
        .map(|(end, start)| end - start)
        .context("closure times")?;
    let (start, end) = (
        spans[0]["start_ms"].as_i64().context("span start")?,
        spans[0]["end_ms"].as_i64().context("span end")?,
    );
    assert!(
        0 <= start && start <= end && end <= length,
        "{spans:?} in {length} ms"
    );
    assert!(!runtime.microphone_muted());
    Ok(())
}

#[tokio::test]
async fn a_mute_the_helper_does_not_confirm_is_never_shown() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (microphone, _) = meeting(&mut runtime, &mut helper, "reunion-sin-silencio").await?;
    // A helper that does not know the order refuses it.
    runtime
        .command(Command::MicrophoneMute { muted: true })
        .await?;
    let order = next_order(&mut helper)?;
    runtime
        .native_event(json!({"event":"response","id":order["id"],"ok":false,
            "error":{"code":"invalid_native_command","message":"Comando nativo desconocido"}}))
        .await?;
    assert!(!runtime.microphone_muted());
    assert!(said::has(&runtime.state, NoticeCode::MicrophoneMuteRefused));
    assert!(said::mentions(&runtime.state, "invalid_native_command"));
    // An answer that does not say it muted this stream mutes nothing.
    runtime
        .command(Command::MicrophoneMute { muted: true })
        .await?;
    let order = next_order(&mut helper)?;
    acknowledge(&mut runtime, &order, json!({"stream_id":microphone})).await?;
    assert!(!runtime.microphone_muted());
    assert!(said::has(&runtime.state, NoticeCode::MicrophoneMuteRefused));
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording")?;
    assert!(recording.muted_spans.is_empty());
    Ok(())
}

#[tokio::test]
async fn the_next_recording_starts_unmuted() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (microphone, _) = meeting(&mut runtime, &mut helper, "reunion-primera").await?;
    set_muted(&mut runtime, &mut helper, &microphone, true).await?;
    runtime.apply(Event::FinishMeeting).await?;
    for _ in 0..2 {
        let stop = next_order(&mut helper)?;
        acknowledge(
            &mut runtime,
            &stop,
            json!({"stream_id":stop["stream_id"],"drain_complete":true}),
        )
        .await?;
    }
    settle_seal_receipts(&mut runtime).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Idle)
    );
    assert!(
        !runtime.microphone_muted(),
        "the mute ends with the recording"
    );
    // `meeting` checks that neither start of the next one says `muted`.
    meeting(&mut runtime, &mut helper, "reunion-segunda").await?;
    assert!(!runtime.microphone_muted());
    Ok(())
}

#[tokio::test]
async fn a_muted_meeting_resumes_muted_once_the_helper_says_so() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (microphone, _) = meeting(&mut runtime, &mut helper, "reunion-en-pausa").await?;
    set_muted(&mut runtime, &mut helper, &microphone, true).await?;
    runtime.apply(Event::PauseMeeting).await?;
    for _ in 0..2 {
        let stop = next_order(&mut helper)?;
        acknowledge(
            &mut runtime,
            &stop,
            json!({"stream_id":stop["stream_id"],"drain_complete":true}),
        )
        .await?;
    }
    settle_seal_receipts(&mut runtime).await?;
    assert!(runtime.microphone_muted(), "a pause does not end the mute");
    runtime.apply(Event::ResumeMeeting).await?;
    let mut starts = Vec::new();
    while let Ok(command) = helper.try_recv() {
        if command["command"] == "audio_start" {
            starts.push(command);
        }
    }
    let start = starts
        .iter()
        .find(|start| start["source"] == "mic_local")
        .context("microphone start")?;
    assert_eq!(start["muted"], true, "the microphone starts muted");
    // This helper started it without saying so: she is told, never left
    // believing it muted.
    acknowledge(&mut runtime, start, json!({"stream_id":start["stream_id"]})).await?;
    assert!(!runtime.microphone_muted());
    assert!(said::has(&runtime.state, NoticeCode::MicrophoneMuteRefused));
    assert!(said::mentions(&runtime.state, "muted_start_not_confirmed"));
    Ok(())
}

#[tokio::test]
async fn oye_deimos_is_not_heard_while_muted_and_no_note_is_dictated() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let (microphone, _) = meeting(&mut runtime, &mut helper, "reunion-sin-oye").await?;
    set_muted(&mut runtime, &mut helper, &microphone, true).await?;
    runtime
        .native_event(
            json!({"v":1,"event":"assistant_question","stream_id":microphone,
            "text":"¿Qué dijo Lucía?","started_ms":1_800_000_000_000_i64,
            "ended_ms":1_800_000_004_200_i64}),
        )
        .await?;
    // Logged with its cause, never its words.
    let told = said::told(&runtime);
    assert!(told.contains(r#""ignored":"microphone_muted""#), "{told}");
    assert!(
        runtime.state.durable.assistant.active_request.is_none(),
        "no turn left"
    );
    assert!(runtime.apply(Event::VoiceStart).await.is_err());
    assert!(!runtime.state.durable.voice.voice_active);
    assert_no_helper_command(&mut helper)?;
    Ok(())
}

/// An order the helper never answered (it went away or started again)
/// blocks nothing: the next «Silenciar» is a new order.
#[tokio::test]
async fn a_helper_gone_with_the_order_lets_the_next_one_through() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (microphone, _) = meeting(&mut runtime, &mut helper, "reunion-sin-respuesta").await?;
    runtime
        .command(Command::MicrophoneMute { muted: true })
        .await?;
    assert_eq!(next_order(&mut helper)?["command"], "audio_mute");
    assert!(
        runtime
            .command(Command::MicrophoneMute { muted: true })
            .await
            .is_err(),
        "one order at a time"
    );
    runtime.end_helper_waits();
    set_muted(&mut runtime, &mut helper, &microphone, true).await?;
    Ok(())
}
