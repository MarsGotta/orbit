//! The assistant topic's projection, tested on states built by hand.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::daemon::WorkResult;
use orbit_core::{Message, State, Turn};
use serde_json::Value;

fn source(state: &State, now_ms: u64) -> TopicSource<'_> {
    TopicSource {
        state,
        runtime: None,
        now_ms,
    }
}

fn assistant_frame(state: &State) -> serde_json::Value {
    let mut board = super::super::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _receiver = channels[&TopicId::Assistant].subscribe();
    board.publish(&source(state, 0));
    serde_json::to_value(channels[&TopicId::Assistant].borrow().data.clone()).unwrap()
}

fn message(id: &str, role: &str, text: &str, complete: bool, at_ms: u64) -> Message {
    Message {
        attachments: Vec::new(),
        id: id.into(),
        role: role.into(),
        text: text.into(),
        complete,
        at_ms,
        voice: None,
    }
}

fn turn_state(user_text: &str, assistant_text: &str) -> State {
    let mut state = State::default();
    crate::daemon::authority_fixture::set(
        &mut state,
        "session_id",
        serde_json::json!("session-topic"),
    );
    state.durable.assistant.active_request = Some("request-topic".into());
    state.durable.assistant.turn = Some(Turn {
        user: message("request-topic", "user", user_text, true, 5),
        answer: Some(message(
            "request-topic:assistant",
            "assistant",
            assistant_text,
            false,
            6,
        )),
    });
    state
}

fn answer_mut(state: &mut State) -> &mut Message {
    state
        .durable
        .assistant
        .turn
        .as_mut()
        .unwrap()
        .answer
        .as_mut()
        .unwrap()
}

#[test]
fn the_turn_in_flight_is_projected_with_activity_and_messages() {
    let data = assistant_frame(&turn_state("Responde breve", "Respuesta parcial"));
    assert_eq!(data["topic"], "assistant");
    assert_eq!(
        data["knowledge"]["state"], "present",
        "the daemon must publish what it knows about the turn, not NotObservedYet"
    );
    let value = &data["knowledge"]["value"];
    assert_eq!(value["turn_in_flight"], true);
    assert_eq!(value["activity"], "responding");
    assert_eq!(
        value["session_id"], "session-topic",
        "the active session travels with the turn"
    );
    assert_eq!(value["turn"]["user"]["text"], "Responde breve");
    assert_eq!(value["turn"]["answer"]["text"], "Respuesta parcial");
    assert_eq!(value["turn"]["answer"]["complete"], false);
    assert!(
        value.get("messages").is_none(),
        "the topic carries no list of messages"
    );
}

#[test]
fn activity_comes_from_facts_not_from_notice_text() {
    // Same facts, unrelated notices: the topic must not move with the
    // notices, and must not need them to name the activity.
    let mut thinking = turn_state("Pregunta", "");
    thinking.durable.assistant.turn.as_mut().unwrap().answer = None;
    thinking.set_notice(orbit_core::notice::NoticeCode::AssistantThinking);
    let data = assistant_frame(&thinking);
    assert_eq!(
        data["knowledge"]["value"]["activity"], "thinking",
        "a turn in flight with no incomplete message is thinking"
    );

    let mut quiet = turn_state("Pregunta", "");
    quiet.durable.assistant.turn.as_mut().unwrap().answer = None;
    quiet.set_notice(orbit_core::notice::NoticeCode::AudioGap);
    let quiet_data = assistant_frame(&quiet);
    assert_eq!(
        quiet_data["knowledge"]["value"]["activity"], "thinking",
        "the activity does not depend on the notice text"
    );

    let mut idle = turn_state("Pregunta", "Lista");
    idle.durable.assistant.active_request = None;
    answer_mut(&mut idle).complete = true;
    let idle_data = assistant_frame(&idle);
    assert_eq!(
        idle_data["knowledge"]["value"]["activity"],
        Value::Null,
        "no turn in flight means no activity"
    );
    assert_eq!(idle_data["knowledge"]["value"]["turn_in_flight"], false);
    assert_eq!(
        idle_data["knowledge"]["value"]["turn"]["answer"]["text"], "Lista",
        "the finished turn stays until the next one, so the screen \
         keeps it while it reads the conversation again"
    );
}

#[test]
fn an_incomplete_old_turn_does_not_mark_the_new_turn_responding() {
    let mut state = turn_state("Pregunta anterior", "Respuesta interrumpida");
    state.durable.assistant.active_request = Some("new-request".into());
    let data = assistant_frame(&state);
    assert_eq!(data["knowledge"]["value"]["activity"], "thinking");
}

#[test]
fn chat_deltas_publish_the_assistant_at_most_ten_times_per_second() {
    let mut state = turn_state("Pregunta", "");
    let mut board = super::super::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _receiver = channels[&TopicId::Assistant].subscribe();
    assert!(
        board
            .publish(&source(&state, 0))
            .contains(&TopicId::Assistant)
    );
    let mut publications = 0_u32;
    for frame in 0..1_000_u64 {
        board.state_changed();
        // Each delta grows the streamed message, so each projection
        // that runs sees a different value and really publishes.
        answer_mut(&mut state).text.push('x');
        let before = channels[&TopicId::Assistant].borrow().revision;
        board.publish(&source(&state, frame));
        if channels[&TopicId::Assistant].borrow().revision != before {
            publications += 1;
        }
    }
    assert_eq!(
        publications, 9,
        "1000 deltas in 1000 ms at the 100 ms cadence reproject at 100..=900 ms"
    );
}

/// The turn at its ceilings — a 32 KiB question, a 96 KiB
/// answer and a 16 KB pending description — in characters that escape to
/// two bytes travels whole; in control characters (six bytes each) it is
/// refused with its size.
#[test]
fn a_turn_at_its_ceilings_fits_the_topic() {
    let mut state = turn_state(
        &"\"".repeat(orbit_core::MAX_TEXT_BYTES),
        &"\n".repeat(orbit_core::MAX_TEXT_BYTES * 3),
    );
    state.durable.assistant.pending_approval = Some(orbit_core::Approval {
        request_id: "request-topic".into(),
        description: "\"".repeat(16_000),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
        deletes: None,
    });
    let data = assistant_frame(&state);
    let bytes = serde_json::to_vec(&data).unwrap().len();
    eprintln!("a turn at its ceilings is {bytes} topic bytes");
    assert_eq!(data["knowledge"]["state"], "present", "{bytes} bytes");
    assert!(bytes <= TopicId::Assistant.max_bytes());
    let turn = state.durable.assistant.turn.as_ref().unwrap();
    assert_eq!(
        data["knowledge"]["value"]["turn"]["user"]["text"],
        turn.user.text
    );
    assert_eq!(
        data["knowledge"]["value"]["turn"]["answer"]["text"],
        turn.answer.as_ref().unwrap().text
    );

    let controls = turn_state("", &"\u{0001}".repeat(orbit_core::MAX_TEXT_BYTES * 3));
    let data = assistant_frame(&controls);
    assert_eq!(data["knowledge"]["state"], "absent");
    assert_eq!(data["knowledge"]["refusal"]["code"], "too_large");
    assert!(
        data["knowledge"]["refusal"]["bytes"].as_u64().unwrap()
            > TopicId::Assistant.max_bytes() as u64
    );
}

#[tokio::test]
async fn voice_transcription_in_progress_is_transcribing_without_a_turn() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _native) = crate::daemon::audio::tests::fixture(directory.path()).unwrap();
    runtime.state.durable.assistant.active_request = None;
    runtime.state.durable.assistant.turn = Some(Turn {
        user: Message {
            voice: Some(orbit_core::VoiceNote {
                duration_ms: 2_000,
                transcript: None,
                failure: None,
                audio_available: true,
            }),
            ..message("voice-topic", "user", "", true, 1)
        },
        answer: None,
    });
    let generation = runtime.state.generation();
    runtime
        .assistant
        .voice_task
        .attach_test(generation, (), tokio::spawn(async {}));
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    let mut board = super::super::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _receiver = channels[&TopicId::Assistant].subscribe();
    board.publish(&source);
    let data = serde_json::to_value(channels[&TopicId::Assistant].borrow().data.clone()).unwrap();
    assert_eq!(
        data["knowledge"]["value"]["activity"], "transcribing",
        "a voice note being transcribed is in Transcribing even with no request in flight"
    );
}

#[tokio::test]
async fn privacy_stop_and_network_cancellation_invalidate_transcribing() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _native) = crate::daemon::audio::tests::fixture(directory.path()).unwrap();
    let generation = runtime.state.generation();
    runtime.state.durable.assistant.turn = Some(Turn {
        user: Message {
            voice: Some(orbit_core::VoiceNote {
                duration_ms: 2_000,
                transcript: None,
                failure: None,
                audio_available: true,
            }),
            ..message("voice-privacy", "user", "", true, 1)
        },
        answer: None,
    });
    runtime.assistant.voice_operation = Some("voice-privacy".into());
    runtime.assistant.voice_task.attach_test(
        generation,
        (),
        tokio::spawn(std::future::pending::<()>()),
    );
    let mut board = super::super::TopicBoard::new("synthetic".into());
    let channels = board.channels();
    let _receiver = channels[&TopicId::Assistant].subscribe();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime
        .cancel_voice_transcription(crate::daemon::audio::VoiceCancellationCause::Authority(
            orbit_core::authority::AuthorityChange::UserStop,
        ))
        .await
        .unwrap();
    board.state_changed();
    assert!(
        board
            .publish(&TopicSource {
                state: &runtime.state,
                runtime: Some(&runtime),
                now_ms: TopicId::Assistant.min_interval_ms()
            })
            .contains(&TopicId::Assistant)
    );
    let data = serde_json::to_value(channels[&TopicId::Assistant].borrow().data.clone()).unwrap();
    assert!(data["knowledge"]["value"]["activity"].is_null());
    assert_eq!(
        data["knowledge"]["value"]["turn"]["user"]["voice"]["failure"]["code"],
        "voice_transcription_privacy_stopped"
    );
}

#[tokio::test]
async fn speech_unavailable_is_a_coded_absence_independent_of_notice_text() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _native) = crate::daemon::audio::tests::fixture(directory.path()).unwrap();
    runtime
        .state
        .set_notice(orbit_core::notice::NoticeCode::AudioGap);
    let generation = runtime.state.generation();
    let ticket = crate::daemon::audio::tests::pending_work(
        &mut runtime.assistant.speech_task,
        generation,
        (),
    )
    .unwrap();
    runtime
        .work_event(WorkResult::SpeechAvailability {
            ticket,
            voice: orbit_core::agents::IMPLICIT_AGENT_ID.into(),
            available: false,
        })
        .await
        .unwrap();
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    let data = serde_json::to_value(Projector.project(&source)).unwrap();
    assert_eq!(data["knowledge"]["value"]["voice"]["state"], "absent");
    assert_eq!(
        data["knowledge"]["value"]["voice"]["code"],
        "speech_unavailable"
    );
    let ticket = crate::daemon::audio::tests::pending_work(
        &mut runtime.assistant.speech_task,
        generation,
        (),
    )
    .unwrap();
    runtime
        .work_event(WorkResult::SpeechAvailability {
            ticket,
            voice: orbit_core::agents::IMPLICIT_AGENT_ID.into(),
            available: true,
        })
        .await
        .unwrap();
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1,
    };
    let data = serde_json::to_value(Projector.project(&source)).unwrap();
    assert_eq!(data["knowledge"]["value"]["voice"]["state"], "present");
}

/// The agent's work rides the same topic as the turn: the turn at its
/// ceilings (`a_turn_at_its_ceilings_fits_the_topic`) with its work at
/// its own — every step the Mac keeps, each at the hub's ceilings in
/// four-byte characters with an id of control characters, and the one
/// plan with all its steps — still travels whole, its pending question
/// included.
#[tokio::test]
async fn a_turn_and_its_work_at_their_ceilings_fit_the_topic() {
    use orbit_core::agent_work::{
        AgentWork, MAX_ITEM_CAUSE_CHARS, MAX_ITEM_ID_CHARS, MAX_ITEM_LABEL_CHARS,
        MAX_ITEM_SUBJECT_CHARS, MAX_PLAN_STEPS, MAX_WORK_ITEMS, WorkItem, WorkKind, WorkPhase,
        WorkStep, WorkStepStatus,
    };
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path()).unwrap();
    runtime.state = turn_state(
        &"\"".repeat(orbit_core::MAX_TEXT_BYTES),
        &"\n".repeat(orbit_core::MAX_TEXT_BYTES * 3),
    );
    runtime.state.durable.assistant.pending_approval = Some(orbit_core::Approval {
        request_id: "request-topic".into(),
        description: "\"".repeat(16_000),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
        deletes: None,
    });
    let wide = |count: usize| "𝔸".repeat(count);
    let item = |index: usize, kind: WorkKind| WorkItem {
        id: format!("{index:02}{}", "\u{1}".repeat(MAX_ITEM_ID_CHARS - 2)),
        kind,
        phase: WorkPhase::Failed,
        label: wide(MAX_ITEM_LABEL_CHARS),
        subject: Some(format!("{index:02}{}", wide(MAX_ITEM_SUBJECT_CHARS - 2))),
        steps: Vec::new(),
        cause: Some(wide(MAX_ITEM_CAUSE_CHARS)),
    };
    let mut work = AgentWork::new("request-topic", 5);
    for index in 1..MAX_WORK_ITEMS {
        work.record(item(index, WorkKind::Tool));
    }
    work.record(WorkItem {
        steps: (0..MAX_PLAN_STEPS)
            .map(|_| WorkStep {
                text: wide(MAX_ITEM_LABEL_CHARS),
                status: WorkStepStatus::InProgress,
            })
            .collect(),
        ..item(0, WorkKind::Plan)
    });
    assert_eq!(work.items.len(), MAX_WORK_ITEMS);
    runtime.assistant.work = Some(work);
    let source = TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    };
    let data = Projector.project(&source);
    let bytes = serde_json::to_vec(&data).unwrap().len();
    eprintln!("a turn and its work at their ceilings are {bytes} topic bytes");
    let data = serde_json::to_value(data).unwrap();
    assert_eq!(
        data["knowledge"]["state"], "present",
        "refused whole: {}",
        data["knowledge"]["refusal"]
    );
}
