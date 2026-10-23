//! A voice note the hub could not take (offline, or its transcription
//! failed) is kept on the Mac as a pending message, shown, retried and never
//! blocks the next recording. Synthetic audio, hub and helper only.
#![allow(clippy::unwrap_used)]

use super::capability_notice_tests::TOKEN;
use super::meeting_continuity_tests::{frame, start};
use super::*;
use crate::daemon::audio::tests::fixture;

/// The helper's side of a voice note: it starts, delivers `frames` samples
/// at 8 kHz, and confirms the stop that `end` asks for.
async fn record(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
    frames: usize,
    end: Command,
) -> Result<()> {
    runtime.command(Command::VoiceStart).await?;
    helper.recv().await.context("playback_stop absent")?;
    let start = helper.recv().await.context("audio_start absent")?;
    let stream = start["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned();
    runtime
        .native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    runtime.native_event(frame(&stream, 0, frames)).await?;
    runtime.command(end).await?;
    let stop = helper.recv().await.context("audio_stop absent")?;
    runtime
        .handle_native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await;
    Ok(())
}

fn voice_manifests(runtime: &Runtime) -> Result<usize> {
    Ok(runtime
        .fixture_store
        .list_jobs(100)?
        .iter()
        .filter(|job| {
            job.kind == "recording_manifest" && job.payload["recording_kind"] == "voice_turn"
        })
        .count())
}

/// A note spoken with the hub away is kept: it is the turn's message, shown
/// with why it waits, and stored once on the Mac.
#[tokio::test]
async fn a_voice_note_spoken_while_the_hub_is_offline_is_kept_and_shown() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    let stored = voice_manifests(&runtime)?;
    let voice = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .and_then(|turn| turn.user.voice.as_ref());
    assert!(
        voice.is_some(),
        "the offline voice note is in no turn ({stored} copies stored on the Mac); notice {:?}",
        runtime.state.durable.notice
    );
    assert_eq!(stored, 1, "one note, one copy on the Mac");
    assert_eq!(
        voice
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some(orbit_core::VOICE_HUB_UNREACHABLE),
        "the note says it waits for the hub"
    );
    Ok(())
}

/// The note spoken offline is sent by itself once the hub is back: its
/// transcription is asked for, from the WAV the Mac kept.
#[tokio::test]
async fn an_offline_voice_note_is_sent_when_the_hub_is_back() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    let message_id = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("note absent")?
        .user
        .id
        .clone();
    runtime.run_maintenance().await;
    assert_eq!(
        runtime.conversation.voice_operation, None,
        "nothing is sent while the hub is away"
    );
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Online),
    );
    runtime.run_maintenance().await;
    assert_eq!(
        runtime.conversation.voice_operation.as_deref(),
        Some(message_id.as_str()),
        "the waiting note is being transcribed"
    );
    let voice = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .and_then(|turn| turn.user.voice.as_ref())
        .context("note absent")?;
    assert_eq!(voice.failure, None);
    assert_eq!(voice_manifests(&runtime)?, 1, "sent from the same copy");
    Ok(())
}

/// A note spoken offline leaves nothing hanging: once the hub is back a
/// meeting starts (it once got `busy/audio_drain` until a restart).
#[tokio::test]
async fn after_an_offline_voice_note_a_meeting_can_start() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Online),
    );
    for _ in 0..3 {
        runtime.run_maintenance().await;
    }
    let started = runtime
        .apply(Event::StartMeeting {
            id: "reunion-siguiente".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await;
    assert!(
        started.is_ok(),
        "the hub is back and a meeting cannot start: {started:?}"
    );
    Ok(())
}

/// The user's stop after an offline note does not store it a second time,
/// as cancelled, under another id.
#[tokio::test]
async fn an_offline_voice_note_is_not_stored_twice_and_cancelled_at_the_stop() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    while let Ok(command) = helper.try_recv() {
        if command["command"] == "privacy_stop" {
            runtime
                .handle_native_event(json!({"event":"response","id":command["id"],"ok":true,
                    "result":{"drain_complete":true,"privacy_stopped":true}}))
                .await;
        }
    }
    for _ in 0..3 {
        runtime.run_maintenance().await;
    }
    let states: Vec<String> = runtime
        .fixture_store
        .list_jobs(100)?
        .iter()
        .filter(|job| {
            job.kind == "recording_manifest" && job.payload["recording_kind"] == "voice_turn"
        })
        .map(|job| {
            job.payload["voice_state"]
                .as_str()
                .unwrap_or("?")
                .to_owned()
        })
        .collect();
    assert!(
        states.len() <= 1 && !states.iter().any(|state| state == "cancelled"),
        "one spoken note, stored as {states:?}"
    );
    Ok(())
}

/// A note whose transcription failed survives the next message: it moves to
/// the notes waiting on the Mac, still listed, and «Reintentar» sends it.
#[tokio::test]
async fn a_failed_voice_note_survives_the_next_message() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let (hub, _hub_rx) = mpsc::channel(4);
    runtime.hub.install(hub);
    // Unpaired when the note is recorded: its transcription fails, its WAV stays.
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    let message_id = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("note absent")?
        .user
        .id
        .clone();
    runtime.token = Some(TOKEN.into());
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "Otra pregunta sintética".into(),
            primary: false,
        })
        .await?;
    // The hub answers that turn, as it does, so the next order is admitted.
    let turn = runtime
        .state
        .durable
        .assistant
        .active_request
        .clone()
        .context("no turn")?;
    for event in [
        json!({"event": "chat.session", "id": turn, "data": {"session_id": "sesion-sintetica"}}),
        json!({"event": "chat.completed", "id": turn,
            "data": {"session_id": "sesion-sintetica", "text": "hecho", "status": "completed"}}),
    ] {
        runtime.hub_event(HubEvent::Frame(event)).await?;
    }
    assert!(
        runtime
            .state
            .durable
            .assistant
            .pending_voice
            .iter()
            .any(|note| note.message.id == message_id),
        "the replaced note is kept and listed"
    );
    let retry = runtime
        .command(Command::VoiceRetry {
            message_id: message_id.clone(),
        })
        .await;
    let still_named = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .is_some_and(|turn| turn.user.id == message_id);
    let audio_kept = runtime.voice_note_audio(&message_id).is_ok();
    assert!(
        retry.is_ok() || still_named,
        "the failed voice note can no longer be retried nor seen (its WAV still on the Mac: {audio_kept}): {retry:?}"
    );
    Ok(())
}

/// The same inside a meeting (PTT shares its microphone): the meeting goes
/// on, and the next note of that meeting can start.
async fn meeting_with_offline_voice_note(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime.token = Some(TOKEN.into());
    let stream = start(runtime, helper).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    runtime.command(Command::VoiceStart).await?;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    let ended = runtime.command(Command::VoiceEnd).await;
    runtime.run_maintenance().await;
    runtime.handle_native_event(frame(&stream, 2, 8_000)).await;
    runtime.run_maintenance().await;
    while helper.try_recv().is_ok() {}
    Ok(format!("{ended:?}"))
}

#[tokio::test]
async fn after_an_offline_voice_note_the_next_one_can_start() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let ended = meeting_with_offline_voice_note(&mut runtime, &mut helper).await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Online),
    );
    let next = runtime.command(Command::VoiceStart).await;
    assert!(
        next.is_ok(),
        "the first note's VoiceEnd answered {ended}; the next note is refused: {next:?}"
    );
    Ok(())
}

/// The ids of the waiting voice notes the app is given for the open
/// conversation, as the assistant topic publishes them.
fn published_pending_voice(runtime: &Runtime) -> Result<Vec<String>> {
    use orbit_core::topics::TopicId;
    let mut board = crate::daemon::topics::TopicBoard::new("pending-voice".into());
    let receiver = board.channels()[&TopicId::Assistant].subscribe();
    board.publish(&crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    });
    let value = serde_json::to_value(&receiver.borrow().data)?;
    Ok(value["knowledge"]["value"]["pending_voice"]
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .filter_map(|note| note["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

async fn open_conversation(runtime: &mut Runtime, id: &str) -> Result<()> {
    runtime
        .apply(Event::OpenSession {
            id: id.into(),
            agent_id: None,
        })
        .await
}

/// Orbit 0.11.0 (2026-10-23): three failed notes of one conversation were
/// shown at the end of every conversation, days-old ones included. A note
/// waiting on the Mac is shown only in the conversation it was spoken in.
#[tokio::test]
async fn a_waiting_note_is_shown_in_its_conversation_and_not_in_another() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    open_conversation(&mut runtime, "conversacion-a").await?;
    runtime
        .apply(Event::VoiceStored {
            id: "nota-de-a".into(),
            duration_ms: 1_500,
        })
        .await?;
    runtime
        .apply(Event::VoiceFailed {
            id: "nota-de-a".into(),
            failure: orbit_core::VoiceTranscriptionFailure::local("transcription_http_error"),
        })
        .await?;
    open_conversation(&mut runtime, "conversacion-b").await?;
    assert_eq!(
        published_pending_voice(&runtime)?,
        Vec::<String>::new(),
        "conversation B shows the note of conversation A"
    );
    open_conversation(&mut runtime, "conversacion-a").await?;
    assert_eq!(published_pending_voice(&runtime)?, ["nota-de-a"]);
    Ok(())
}

/// A note spoken offline in one conversation is sent when the hub is back
/// only while that conversation is open: never into another.
#[tokio::test]
async fn a_waiting_note_is_sent_only_into_its_own_conversation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    open_conversation(&mut runtime, "conversacion-a").await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    let note = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("note absent")?
        .user
        .id
        .clone();
    open_conversation(&mut runtime, "conversacion-b").await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Online),
    );
    runtime.run_maintenance().await;
    assert_eq!(
        runtime.conversation.voice_operation, None,
        "the note of conversation A was sent into conversation B"
    );
    open_conversation(&mut runtime, "conversacion-a").await?;
    runtime.run_maintenance().await;
    assert_eq!(runtime.conversation.voice_operation, Some(note));
    Ok(())
}

/// «Descartar», confirmed: the failed note leaves the conversation and its
/// audio leaves this Mac.
#[tokio::test]
async fn discarding_a_failed_note_removes_it_and_its_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.token = Some(TOKEN.into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    record(&mut runtime, &mut helper, 8_000, Command::VoiceEnd).await?;
    let note = runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("note absent")?
        .user
        .id
        .clone();
    assert_eq!(voice_manifests(&runtime)?, 1);
    runtime
        .command(Command::VoiceDiscard {
            message_id: note.clone(),
        })
        .await?;
    assert!(runtime.state.durable.assistant.turn.is_none());
    assert!(runtime.state.durable.assistant.pending_voice.is_empty());
    assert_eq!(
        voice_manifests(&runtime)?,
        0,
        "its audio is still on the Mac"
    );
    assert!(runtime.voice_note_audio(&note).is_err());
    Ok(())
}

/// A voice note during a recording never touches the recording's lane:
/// discarding a failed note or dropping one in which nothing was heard
/// changes no input of the lane's arming (the microphone stays the
/// recording's), so the capture task keeps serving the meeting, and only
/// the note's own audio is deleted, through the same store actor that
/// writes the lane's seals.
#[tokio::test]
async fn a_note_discarded_during_a_recording_leaves_the_lane_armed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    start(&mut runtime, &mut helper).await?;
    runtime.settle_capture_lane().await;
    assert!(
        runtime.capture.lane.lock().gate.armed,
        "the lane never armed"
    );
    let meeting_jobs = |runtime: &Runtime| -> Result<usize> {
        Ok(runtime
            .fixture_store
            .list_jobs(100)?
            .iter()
            .filter(|job| job.payload["recording_id"] == "reunion-sintetica")
            .count())
    };
    let before = meeting_jobs(&runtime)?;
    assert!(before > 0, "the meeting left no job to compare");
    for id in ["nota-descartada", "nota-sin-voz"] {
        runtime
            .apply(Event::VoiceStored {
                id: id.into(),
                duration_ms: 1_500,
            })
            .await?;
    }
    runtime
        .apply(Event::VoiceFailed {
            id: "nota-descartada".into(),
            failure: orbit_core::VoiceTranscriptionFailure::local("transcription_http_error"),
        })
        .await?;
    runtime
        .command(Command::VoiceDiscard {
            message_id: "nota-descartada".into(),
        })
        .await?;
    runtime
        .drop_unheard_voice_note("nota-sin-voz", "no_words")
        .await?;
    assert!(
        runtime.capture.lane.lock().gate.armed,
        "a voice note's discard disarmed the recording's lane"
    );
    assert_eq!(
        meeting_jobs(&runtime)?,
        before,
        "the meeting's jobs went with the note"
    );
    Ok(())
}
