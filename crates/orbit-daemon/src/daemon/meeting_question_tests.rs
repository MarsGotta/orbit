//! Questions to Deimos during a meeting, through the daemon's own paths: a
//! synthetic helper (`assistant_question`, `voice_trigger`) and a synthetic
//! hub channel. The meeting's transcript is the hub's (it adds it to the
//! turn). Nothing native, no real Deimos.
use super::*;
use crate::{daemon::audio::tests::fixture, hub::HubEvent};
use orbit_core::{
    Event,
    meeting_question::{
        BusyTurn, FAILURE_SHOWN_MS, QuestionCause, QuestionOrigin, QuestionStage, VoiceTrigger,
    },
    protocol::Command,
    topics::{Knowledge, TopicData, TopicId},
};
use tokio::sync::mpsc;

const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const MIC: &str = "mic-stream-1";
const REMOTE: &str = "remote-stream-1";
const START: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;
/// Ten minutes and a half into the meeting.
const NOW: i64 = START + 10 * MINUTE + 30_000;

fn recording(runtime: &mut Runtime, phase: &str) {
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "recording",
        json!({"id": RECORDING, "title": "Comité de presupuesto", "started_at_ms": START,
            "ended_at_ms": null, "expires_at_ms": START + 3_600_000, "application_pid": null,
            "audio": "system", "phase": phase, "sealed_chunks": 0, "gaps": 0}),
    );
    runtime.capture.mic_stream = Some(MIC.into());
    runtime.capture.remote_stream = Some(REMOTE.into());
}

/// The runtime, its hub frames and the helper's commands (kept open: a
/// dropped helper is a disconnected one).
type Meeting = (
    tempfile::TempDir,
    Runtime,
    mpsc::Receiver<Value>,
    mpsc::Receiver<Value>,
);

async fn meeting_with_hub() -> Result<Meeting> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, helper) = fixture(directory.path())?;
    let (hub, hub_frames) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime.features.question.clock_ms = Some(NOW);
    recording(&mut runtime, "recording");
    Ok((directory, runtime, hub_frames, helper))
}

/// The next frame to the hub. A stall guard, not a budget: the frame is
/// sent before the command returns, so a passing run never waits; a turn
/// that was never sent fails here instead of hanging the suite.
async fn next_frame(frames: &mut mpsc::Receiver<Value>) -> Option<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(10), frames.recv())
        .await
        .ok()
        .flatten()
}

fn heard(stream: &str, text: &str) -> Value {
    json!({"v": 1, "event": "assistant_question", "stream_id": stream, "text": text,
        "started_ms": NOW - 4_000, "ended_ms": NOW - 500})
}

fn question(runtime: &Runtime) -> Result<orbit_core::meeting_question::MeetingQuestion> {
    runtime
        .features
        .question
        .current
        .clone()
        .context("no question in the recording topic")
}

/// The recording topic as the app receives it.
fn recording_topic(runtime: &Runtime) -> Result<Value> {
    let mut board = crate::daemon::topics::TopicBoard::new("meeting-question".into());
    let channels = board.channels();
    let _receiver = channels[&TopicId::Recording].subscribe();
    board.publish(&crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    });
    let data = channels[&TopicId::Recording].borrow().data.clone();
    match data {
        TopicData::Recording(Knowledge::Present { value, .. }) => Ok(serde_json::to_value(value)?),
        other => bail!("recording topic not present: {other:?}"),
    }
}

/// The phrase heard on the call's track, or on a stream
/// that is not the meeting's microphone, starts nothing.
#[tokio::test]
async fn the_phrase_on_the_calls_track_starts_nothing() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    for stream in [REMOTE, "another-mic"] {
        runtime
            .native_event(heard(stream, "¿Qué dijo Lucía del presupuesto?"))
            .await?;
    }
    assert!(hub_frames.try_recv().is_err(), "nothing went to Deimos");
    assert!(runtime.features.question.current.is_none());
    assert!(runtime.state.durable.active_request.is_none());

    // Paused: the microphone is not the meeting's any more.
    recording(&mut runtime, "paused");
    runtime.native_event(heard(MIC, "¿Y ahora?")).await?;
    assert!(hub_frames.try_recv().is_err());
    // The user's stop: the helper should not send it; if it does, it is dropped.
    recording(&mut runtime, "recording");
    crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(true));
    runtime.native_event(heard(MIC, "¿Y ahora?")).await?;
    assert!(hub_frames.try_recv().is_err());
    assert!(runtime.features.question.current.is_none());
    Ok(())
}

/// The button and the shortcut ask with the same
/// context as the voice.
#[tokio::test]
async fn the_button_and_the_shortcut_ask_with_the_same_context() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Qué dijo Lucía del presupuesto?".into(),
            minutes: 2,
        })
        .await?;
    let typed = next_frame(&mut hub_frames)
        .await
        .context("typed chat.send")?;
    let meeting =
        json!({"recording_id": RECORDING, "minutes": 2, "title": "Comité de presupuesto"});
    assert_eq!(typed["params"]["text"], "¿Qué dijo Lucía del presupuesto?");
    assert_eq!(typed["params"]["meeting"], meeting);
    assert_eq!(question(&runtime)?.origin, QuestionOrigin::Typed);
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn, "data": {"text": "Un 10 %."}
        })))
        .await?;

    let dictated = runtime
        .ask_meeting_by_dictation(
            "voice-message-1".into(),
            "¿Qué dijo Lucía del presupuesto?".into(),
            4_000,
        )
        .await?;
    assert!(
        dictated,
        "during a recording the dictation is a meeting question"
    );
    let spoken = next_frame(&mut hub_frames)
        .await
        .context("dictated chat.send")?;
    assert_eq!(question(&runtime)?.origin, QuestionOrigin::Hotkey);
    assert_eq!(question(&runtime)?.id, "voice-message-1");
    assert_eq!(spoken["params"]["text"], typed["params"]["text"]);
    assert_eq!(spoken["params"]["meeting"], meeting);
    assert_eq!(spoken["params"]["client_id"], "voice-message-1");
    assert_eq!(
        spoken["params"]["voice"],
        json!({"duration_ms": 4_000}),
        "the dictated question is a voice note"
    );

    // Another window from the button: five minutes reach further back.
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn, "data": {"text": "Sí."}
        })))
        .await?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y antes?".into(),
            minutes: 5,
        })
        .await?;
    let wide = next_frame(&mut hub_frames).await.context("five minutes")?;
    assert_eq!(wide["params"]["meeting"]["minutes"], 5);

    // Without a recording the shortcut is an ordinary voice note.
    crate::daemon::authority_fixture::set(&mut runtime.state, "recording", Value::Null);
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn, "data": {"text": "Sí."}
        })))
        .await?;
    assert!(
        !runtime
            .ask_meeting_by_dictation("voice-message-2".into(), "Hola".into(), 1_000)
            .await?
    );
    Ok(())
}

/// Every failure of the turn reaches the panel with its
/// cause, and stays 8 s.
#[tokio::test]
async fn each_failure_of_the_turn_reaches_the_panel_with_its_cause() -> Result<()> {
    // Deimos (Hermes) not reachable from the hub.
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Qué dijo Lucía?".into(),
            minutes: 2,
        })
        .await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({"event": "error", "id": turn,
            "data": {"code": "upstream_unavailable", "message": "Hermes no está conectado"}})))
        .await?;
    let failed = question(&runtime)?;
    assert_eq!(failed.stage, QuestionStage::Failed);
    assert_eq!(
        failed.cause,
        Some(QuestionCause::DeimosUnavailable {
            code: "upstream_unavailable".into()
        })
    );
    assert_eq!(failed.shown_until_ms(), Some(NOW + FAILURE_SHOWN_MS));
    let topic = recording_topic(&runtime)?;
    assert_eq!(topic["question"]["cause"]["cause"], "deimos_unavailable");

    // Deimos ended the turn with an error.
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 2,
        })
        .await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(
            json!({"event": "chat.completed", "id": turn,
            "data": {"text": "", "status": "error"}}),
        ))
        .await?;
    assert_eq!(question(&runtime)?.cause, Some(QuestionCause::TurnFailed));

    // The previous turn is still running.
    runtime
        .command(Command::Chat {
            attachments: Vec::new(),
            text: "Otra cosa".into(),
        })
        .await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 2,
        })
        .await?;
    assert!(hub_frames.try_recv().is_err(), "nothing sent over a turn");
    assert_eq!(
        question(&runtime)?.cause,
        Some(QuestionCause::TurnInFlight {
            turn: BusyTurn::AssistantTurn
        })
    );

    // The user interrupts a question in flight.
    let turn = runtime
        .state
        .durable
        .active_request
        .clone()
        .context("turn")?;
    runtime
        .hub_event(HubEvent::Frame(json!({
            "event": "chat.completed", "id": turn, "data": {"text": "Hecho."}
        })))
        .await?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 2,
        })
        .await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    runtime.command(Command::Interrupt).await?;
    runtime.follow_meeting_question();
    assert_eq!(question(&runtime)?.cause, Some(QuestionCause::Interrupted));

    // The Mac lost the hub.
    let hub_lost = |runtime: &mut Runtime| {
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("offline"));
    };
    hub_lost(&mut runtime);
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 2,
        })
        .await?;
    assert_eq!(
        question(&runtime)?.cause,
        Some(QuestionCause::HubUnreachable {
            connection: Connection::Offline
        })
    );

    // No meeting; a window the button does not offer; too long.
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("online"));
    crate::daemon::authority_fixture::set(&mut runtime.state, "recording", Value::Null);
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 2,
        })
        .await?;
    assert_eq!(question(&runtime)?.cause, Some(QuestionCause::NoMeeting));
    recording(&mut runtime, "recording");
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y ahora?".into(),
            minutes: 7,
        })
        .await?;
    assert!(matches!(
        question(&runtime)?.cause,
        Some(QuestionCause::WindowNotOffered { minutes: 7, .. })
    ));
    runtime
        .command(Command::MeetingAsk {
            question: "a".repeat(1_025),
            minutes: 2,
        })
        .await?;
    assert_eq!(
        question(&runtime)?.cause,
        Some(QuestionCause::QuestionTooLong {
            bytes: 1_025,
            max_bytes: 1_024
        })
    );
    assert!(
        hub_frames.try_recv().is_err(),
        "no refused question was sent"
    );

    // The user closes it; closing another one changes nothing.
    let shown = question(&runtime)?.id;
    runtime
        .command(Command::MeetingQuestionClose {
            id: "another".into(),
        })
        .await?;
    assert!(runtime.features.question.current.is_some());
    runtime
        .command(Command::MeetingQuestionClose { id: shown })
        .await?;
    assert!(runtime.features.question.current.is_none());
    Ok(())
}

/// The commands the helper received, `status` polls left out.
fn helper_commands(helper: &mut mpsc::Receiver<Value>) -> Vec<Value> {
    let mut commands = Vec::new();
    while let Ok(command) = helper.try_recv() {
        if command["command"] != "status" {
            commands.push(command);
        }
    }
    commands
}

fn wake_orders(commands: &[Value]) -> Vec<(String, bool)> {
    commands
        .iter()
        .filter(|command| command["command"] == "audio_wake_phrase")
        .map(|command| {
            (
                command["stream_id"].as_str().unwrap_or_default().to_owned(),
                command["enabled"].as_bool().unwrap_or_default(),
            )
        })
        .collect()
}

/// «Oye Deimos» goes with the meeting recording, not with the stream.
/// A voice note opened the microphone first: the meeting reuses that stream
/// and still turns the phrase on; pausing while the note is dictated turns
/// it off although the stream stays open; resuming turns it on again and
/// finishing turns it off.
#[tokio::test]
async fn the_phrase_follows_the_meeting_even_on_a_voice_notes_stream() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.features.question.helper_listens = true;
    runtime.apply(Event::VoiceStart).await?;
    let start = helper_commands(&mut helper)
        .into_iter()
        .find(|command| command["command"] == "audio_start")
        .context("the voice note's audio_start")?;
    assert!(start.get("wake_phrase").is_none(), "{start}");
    let voice_stream = start["stream_id"].as_str().context("stream")?.to_owned();
    runtime
        .native_event(json!({"event": "response", "id": start["id"], "ok": true,
            "result": {"stream_id": voice_stream}}))
        .await?;
    assert!(
        wake_orders(&helper_commands(&mut helper)).is_empty(),
        "a voice note does not listen"
    );

    runtime
        .apply(Event::StartMeeting {
            id: RECORDING.into(),
            title: "Comité".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60 * MINUTE,
        })
        .await?;
    runtime.maintain_meeting_question().await?;
    let started = helper_commands(&mut helper);
    assert!(
        !started
            .iter()
            .any(|command| command["command"] == "audio_start"),
        "the meeting reuses the voice note's stream: {started:?}"
    );
    assert_eq!(wake_orders(&started), [(voice_stream.clone(), true)]);

    runtime.apply(Event::PauseMeeting).await?;
    let paused = helper_commands(&mut helper);
    assert_eq!(wake_orders(&paused), [(voice_stream.clone(), false)]);
    assert!(
        !paused
            .iter()
            .any(|command| command["command"] == "audio_stop"),
        "the note keeps the stream open: {paused:?}"
    );

    runtime.apply(Event::ResumeMeeting).await?;
    runtime.maintain_meeting_question().await?;
    assert_eq!(
        wake_orders(&helper_commands(&mut helper)),
        [(voice_stream.clone(), true)]
    );
    runtime.apply(Event::FinishMeeting).await?;
    assert_eq!(
        wake_orders(&helper_commands(&mut helper)),
        [(voice_stream, false)]
    );
    Ok(())
}

/// The user's stop turns the phrase off in the helper by itself; the daemon
/// forgets it and sends nothing a latched helper would refuse. A ready from
/// the bundled helper needs no capability negotiation.
#[tokio::test]
async fn user_stop_gets_no_order_and_the_bundled_helper_needs_no_negotiation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.features.question.helper_listens = true;
    recording(&mut runtime, "recording");
    runtime.maintain_meeting_question().await?;
    assert_eq!(
        wake_orders(&helper_commands(&mut helper)),
        [(MIC.to_owned(), true)]
    );
    crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(true));
    runtime.maintain_meeting_question().await?;
    assert!(wake_orders(&helper_commands(&mut helper)).is_empty());
    assert_eq!(runtime.features.question.wake_stream, None);

    crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(false));
    runtime.features.question.helper_listens = false;
    runtime
        .native_event(json!({"v": 1, "event": "ready", "protocol": 1,
        "platform": "macos", "selected_text_available": false,
        "supported_audio_processing": ["unprocessed"], "voice_processing_available": false,
        "playback_reference_available": false, "launch_mode": "managed_stdio"}))
        .await?;
    assert!(runtime.features.question.helper_listens);
    Ok(())
}

/// A refused order turns into the status's cause, with its code.
#[tokio::test]
async fn a_refused_order_says_its_code_in_the_status() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.features.question.helper_listens = true;
    recording(&mut runtime, "recording");
    runtime.maintain_meeting_question().await?;
    let order = helper_commands(&mut helper)
        .into_iter()
        .find(|command| command["command"] == "audio_wake_phrase")
        .context("audio_wake_phrase")?;
    runtime
        .native_event(json!({"event": "response", "id": order["id"], "ok": false,
            "error": {"code": "stream_not_found", "message": "No se pudo completar la operación nativa."}}))
        .await?;
    let topic = recording_topic(&runtime)?;
    assert_eq!(topic["voice_trigger"]["state"], "not_listening", "{topic}");
    assert_eq!(topic["voice_trigger"]["why"]["cause"], "failed");
    assert_eq!(topic["voice_trigger"]["why"]["code"], "stream_not_found");
    Ok(())
}

/// The topic says whether «Oye Deimos» listens during the recording and,
/// when not, why; outside a recording it says nothing.
#[tokio::test]
async fn the_detectors_state_and_its_causes_reach_the_topic() -> Result<()> {
    let (_directory, mut runtime, _hub_frames, _helper) = meeting_with_hub().await?;
    runtime.features.question.helper_listens = true;
    runtime.maintain_meeting_question().await?;
    assert_eq!(
        recording_topic(&runtime)?["voice_trigger"]["state"],
        "starting"
    );
    for (event, state) in [
        (
            json!({"v": 1, "event": "voice_trigger", "state": "downloading", "percent": 140}),
            "downloading",
        ),
        (
            json!({"v": 1, "event": "voice_trigger", "state": "listening"}),
            "listening",
        ),
    ] {
        runtime.native_event(event).await?;
        assert_eq!(recording_topic(&runtime)?["voice_trigger"]["state"], state);
    }
    assert_eq!(
        runtime.features.question.voice_trigger,
        Some(VoiceTrigger::Listening)
    );
    runtime
        .native_event(json!({"v": 1, "event": "voice_trigger", "state": "failed",
            "cause": "assets_download_failed", "detail": "x".repeat(400)}))
        .await?;
    let topic = recording_topic(&runtime)?;
    assert_eq!(
        topic["voice_trigger"]["why"]["code"],
        "assets_download_failed"
    );
    assert_eq!(
        topic["voice_trigger"]["why"]["detail"]
            .as_str()
            .map(str::len),
        Some(256)
    );
    runtime
        .native_event(
            json!({"v": 1, "event": "voice_trigger", "state": "unavailable",
            "cause": "macos_version", "os_version": "15.4"}),
        )
        .await?;
    let topic = recording_topic(&runtime)?;
    assert_eq!(topic["voice_trigger"]["why"]["cause"], "macos_version");
    assert_eq!(topic["voice_trigger"]["why"]["os_version"], "15.4");
    crate::daemon::authority_fixture::set(&mut runtime.state, "recording", Value::Null);
    assert!(recording_topic(&runtime)?.get("voice_trigger").is_none());
    Ok(())
}

/// A typed question of 20 000 bytes is refused with its size and the
/// recording topic still fits its 16 KiB head.
#[tokio::test]
async fn a_huge_question_is_refused_with_its_size_and_the_topic_fits() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    runtime
        .command(Command::MeetingAsk {
            question: "a".repeat(20_000),
            minutes: 2,
        })
        .await?;
    assert!(hub_frames.try_recv().is_err());
    let refused = question(&runtime)?;
    assert_eq!(
        refused.cause,
        Some(QuestionCause::QuestionTooLong {
            bytes: 20_000,
            max_bytes: 1_024
        })
    );
    assert!(!refused.sent);
    let bytes = serde_json::to_vec(&recording_topic(&runtime)?)?.len();
    assert!(bytes <= TopicId::Recording.max_bytes(), "{bytes}");
    Ok(())
}

/// The largest question with the longest title and two hundred audio
/// applications still fits the recording topic's 16 KiB head.
#[tokio::test]
async fn the_largest_question_fits_the_recording_topic() -> Result<()> {
    let (_directory, mut runtime, _hub_frames, _helper) = meeting_with_hub().await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "recording",
        json!({"id": RECORDING, "title": "ñ".repeat(256), "started_at_ms": START,
            "ended_at_ms": null, "expires_at_ms": START + 3_600_000, "application_pid": null,
            "audio": "system", "phase": "recording", "sealed_chunks": 0, "gaps": 0}),
    );
    runtime.features.question.current =
        Some(orbit_core::meeting_question::MeetingQuestion::refused(
            "00000000-0000-4000-8000-000000000001".into(),
            Some(RECORDING.into()),
            "\"".repeat(1_024),
            10,
            QuestionOrigin::Typed,
            QuestionCause::HubRefused {
                code: "x".repeat(64),
                message: "ñ".repeat(128),
            },
            NOW,
        ));
    runtime.features.question.voice_trigger = Some(VoiceTrigger::Failed {
        cause: "c".repeat(256),
        detail: Some("d".repeat(256)),
    });
    let topic = recording_topic(&runtime)?;
    let bytes = serde_json::to_vec(&topic)?.len();
    // Two hundred applications at the measured ~60 bytes each.
    let applications = 200 * 60;
    assert!(
        bytes + applications <= TopicId::Recording.max_bytes(),
        "{bytes} + {applications} > {}",
        TopicId::Recording.max_bytes()
    );
    Ok(())
}

/// A second question while Deimos still answers the first keeps the first in
/// the panel (its answer is on its way) and says why the second did not go:
/// Deimos still answers, which the user fixes by asking again later — never
/// the generic failure.
#[tokio::test]
async fn a_question_over_one_in_flight_keeps_the_first_and_says_why() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    runtime.native_event(heard(MIC, "¿Qué dijo Lucía?")).await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    let first = question(&runtime)?;
    runtime
        .command(Command::MeetingAsk {
            question: "¿Y Pedro?".into(),
            minutes: 2,
        })
        .await?;
    assert!(hub_frames.try_recv().is_err(), "the second did not go");
    let kept = question(&runtime)?;
    assert_eq!(kept.id, first.id);
    assert_eq!(kept.stage, QuestionStage::Thinking);
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::MeetingQuestionStillAnswering),
        "{:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        orbit_core::notice::NoticeCode::MeetingQuestionStillAnswering.class(),
        orbit_core::notice::NoticeClass::Fixable
    );
    // Another cause says itself, not «Deimos aún responde».
    runtime
        .command(Command::MeetingAsk {
            question: "a".repeat(2_000),
            minutes: 2,
        })
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .notice
            .as_ref()
            .and_then(|notice| notice.data.get(&orbit_core::notice::NoticeKey::Bytes))
            .cloned(),
        Some(orbit_core::notice::DataValue::Int(2_000)),
        "{:?}",
        runtime.state.durable.notice
    );
    assert_eq!(question(&runtime)?.id, first.id);
    Ok(())
}

/// The helper heard one «Oye Deimos» twice while Deimos was still
/// answering it. The repeat is the same question already on its way: it is
/// not sent, and nothing is said about it.
#[tokio::test]
async fn the_same_spoken_question_heard_again_while_it_is_answered_says_nothing() -> Result<()> {
    let (_directory, mut runtime, mut hub_frames, _helper) = meeting_with_hub().await?;
    runtime
        .native_event(heard(MIC, "¿De qué estamos hablando?"))
        .await?;
    next_frame(&mut hub_frames).await.context("chat.send")?;
    let first = question(&runtime)?;
    let before = runtime.state.durable.notice.clone();
    runtime
        .native_event(heard(MIC, "de qué estamos hablando"))
        .await?;
    assert!(hub_frames.try_recv().is_err(), "the repeat did not go");
    assert_eq!(question(&runtime)?.id, first.id);
    assert_eq!(question(&runtime)?.stage, QuestionStage::Thinking);
    assert_eq!(
        runtime.state.durable.notice, before,
        "nothing said about the repeat"
    );
    // A different spoken question is still said.
    runtime.native_event(heard(MIC, "¿Y quién habló?")).await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::MeetingQuestionStillAnswering)
    );
    Ok(())
}

#[tokio::test]
async fn hub_name_reaches_status_and_live_wake_orders_and_survives_restart() -> Result<()> {
    let (directory, mut runtime, _hub, mut helper) = meeting_with_hub().await?;
    runtime.settings.device_id = Some("device-assistant-name".into());
    runtime.features.question.helper_listens = true;
    runtime.sync_wake_phrase().await?;
    let first = helper_commands(&mut helper);
    let wake = first
        .iter()
        .find(|c| c["command"] == "audio_wake_phrase")
        .context("wake on")?;
    assert_eq!(wake["assistant_name"], "Deimos");

    let device = runtime
        .settings
        .device_id
        .clone()
        .context("fixture device")?;
    runtime
        .hub_event(HubEvent::Frame(json!({"event":"ready", "data": {
            "device_id": device, "hub_protocol": 1, "assistant_name": "Atlas"
        }})))
        .await?;
    let commands = helper_commands(&mut helper);
    let wake = commands
        .iter()
        .find(|c| c["command"] == "audio_wake_phrase" && c["enabled"] == true)
        .context("changed name reaches the current listener")?;
    assert_eq!(wake["assistant_name"], "Atlas");
    let source = crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    let mut board = crate::daemon::topics::TopicBoard::new("assistant-name".into());
    let channels = board.channels();
    let _status = channels[&TopicId::Status].subscribe();
    board.publish(&source);
    let status = serde_json::to_value(&channels[&TopicId::Status].borrow().data)?;
    assert_eq!(status["knowledge"]["value"]["assistant_name"], "Atlas");
    let saved = runtime.paths.load_settings()?;
    assert_eq!(saved.assistant_name.as_deref(), Some("Atlas"));

    runtime.hub_event(HubEvent::Disconnected).await?;
    runtime
        .hub_event(HubEvent::Frame(json!({"event":"ready", "data": {
            "device_id": device, "hub_protocol": 1
        }})))
        .await?;
    assert_eq!(runtime.settings.assistant_name.as_deref(), Some("Atlas"));
    runtime
        .hub_event(HubEvent::Frame(json!({"event":"ready", "data": {
            "device_id":"other-device", "assistant_name":"Other"
        }})))
        .await?;
    assert_eq!(
        runtime.paths.load_settings()?.assistant_name.as_deref(),
        Some("Atlas")
    );
    // A new runtime reads the same local settings before any hub connection.
    drop(runtime);
    let restarted =
        crate::config::Paths::resolve(Some(directory.path().to_owned()))?.load_settings()?;
    assert_eq!(restarted.assistant_name.as_deref(), Some("Atlas"));
    Ok(())
}
