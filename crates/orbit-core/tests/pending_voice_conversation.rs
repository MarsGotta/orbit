#![allow(clippy::expect_used, clippy::unwrap_used)]
//! A voice note waiting on this Mac belongs to the conversation it was
//! spoken in: it is shown there and nowhere else, sent there, and can be
//! discarded. Conversations, agents and notes are invented.

use orbit_core::{
    Connection, DurableState, Event, State, VoiceTranscriptionFailure, decide, refusal::Refusal,
};

fn online() -> State {
    decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state
}

fn open(state: &State, id: &str) -> State {
    decide(
        state,
        Event::OpenSession {
            id: id.into(),
            agent_id: Some("agente-sintetico".into()),
        },
        1,
    )
    .unwrap()
    .state
}

/// A note spoken in the open conversation whose transcription failed.
fn failed_note(state: &State, id: &str) -> State {
    let stored = decide(
        state,
        Event::VoiceStored {
            id: id.into(),
            duration_ms: 1_500,
        },
        2,
    )
    .unwrap()
    .state;
    decide(
        &stored,
        Event::VoiceFailed {
            id: id.into(),
            failure: VoiceTranscriptionFailure::local("transcription_http_error"),
        },
        3,
    )
    .unwrap()
    .state
}

fn shown(state: &State) -> Vec<String> {
    state
        .durable
        .open_conversation_pending_voice()
        .map(|message| message.id.clone())
        .collect()
}

#[test]
fn a_waiting_note_shows_only_in_the_conversation_it_was_spoken_in() {
    let first = failed_note(&open(&online(), "conversacion-a"), "nota-a");
    let second = open(&first, "conversacion-b");
    assert_eq!(
        second.durable.assistant.pending_voice.len(),
        1,
        "the note is kept"
    );
    assert!(
        shown(&second).is_empty(),
        "another conversation shows no note"
    );
    let back = open(&second, "conversacion-a");
    assert_eq!(shown(&back), ["nota-a"]);
}

#[test]
fn a_note_left_by_a_new_conversation_stays_with_the_conversation_it_was_in() {
    let first = failed_note(&open(&online(), "conversacion-a"), "nota-a");
    let fresh = decide(
        &first,
        Event::NewSession {
            agent_id: Some("agente-sintetico".into()),
        },
        4,
    )
    .unwrap()
    .state;
    assert!(shown(&fresh).is_empty(), "the new conversation shows none");
    assert_eq!(shown(&open(&fresh, "conversacion-a")), ["nota-a"]);
}

#[test]
fn a_note_spoken_before_the_conversation_existed_is_the_one_that_starts() {
    let fresh = decide(
        &online(),
        Event::NewSession {
            agent_id: Some("agente-sintetico".into()),
        },
        1,
    )
    .unwrap()
    .state;
    let noted = failed_note(&fresh, "nota-previa");
    let typed = decide(
        &noted,
        Event::Submit {
            id: "pregunta".into(),
            text: "Una pregunta sintética".into(),
            attachments: Vec::new(),
        },
        4,
    )
    .unwrap()
    .state;
    assert_eq!(shown(&typed), ["nota-previa"], "still the new conversation");
    let named = decide(
        &typed,
        Event::Session {
            id: "conversacion-nueva".into(),
            agent_id: Some("agente-sintetico".into()),
        },
        5,
    )
    .unwrap()
    .state;
    assert_eq!(shown(&named), ["nota-previa"], "it started this one");
    let elsewhere = decide(
        &named,
        Event::NewSession {
            agent_id: Some("agente-sintetico".into()),
        },
        6,
    )
    .unwrap()
    .state;
    assert!(shown(&elsewhere).is_empty(), "not in the next new one");
}

#[test]
fn a_note_of_another_conversation_is_not_retried_here() {
    let first = failed_note(&open(&online(), "conversacion-a"), "nota-a");
    let second = open(&first, "conversacion-b");
    assert_eq!(
        decide(
            &second,
            Event::VoiceRetry {
                id: "nota-a".into()
            },
            7
        )
        .unwrap_err(),
        Refusal::NotFound {
            resource: orbit_core::refusal::Resource::Message,
        }
    );
    let back = open(&second, "conversacion-a");
    let retried = decide(
        &back,
        Event::VoiceRetry {
            id: "nota-a".into(),
        },
        8,
    )
    .unwrap()
    .state;
    assert_eq!(
        retried.durable.assistant.turn.as_ref().unwrap().user.id,
        "nota-a"
    );
}

#[test]
fn discarding_removes_a_failed_note_and_refuses_one_still_transcribed() {
    let failed = failed_note(&open(&online(), "conversacion-a"), "nota-a");
    let discarded = decide(
        &failed,
        Event::VoiceDiscard {
            id: "nota-a".into(),
        },
        9,
    )
    .unwrap()
    .state;
    assert!(discarded.durable.assistant.turn.is_none());
    assert!(discarded.durable.assistant.pending_voice.is_empty());

    let waiting = open(&failed, "conversacion-b");
    let gone = decide(
        &waiting,
        Event::VoiceDiscard {
            id: "nota-a".into(),
        },
        9,
    )
    .unwrap()
    .state;
    assert!(
        gone.durable.assistant.pending_voice.is_empty(),
        "a waiting note too"
    );

    let in_course = decide(
        &open(&online(), "conversacion-a"),
        Event::VoiceStored {
            id: "nota-en-curso".into(),
            duration_ms: 1_500,
        },
        2,
    )
    .unwrap()
    .state;
    assert!(matches!(
        decide(
            &in_course,
            Event::VoiceDiscard {
                id: "nota-en-curso".into()
            },
            9
        )
        .unwrap_err(),
        Refusal::Busy { .. }
    ));
}

#[test]
fn an_unheard_note_leaves_without_a_trace() {
    let in_course = decide(
        &open(&online(), "conversacion-a"),
        Event::VoiceStored {
            id: "ruido".into(),
            duration_ms: 1_200,
        },
        2,
    )
    .unwrap()
    .state;
    let unheard = decide(&in_course, Event::VoiceUnheard { id: "ruido".into() }, 3)
        .unwrap()
        .state;
    assert!(unheard.durable.assistant.turn.is_none());
    assert!(unheard.durable.assistant.pending_voice.is_empty());
    assert!(shown(&unheard).is_empty());
}

/// Orbit 0.11.0 kept waiting notes without their conversation. They load,
/// and with their conversation unknown they show only in a new one.
#[test]
fn a_note_kept_by_the_previous_version_shows_only_in_a_new_conversation() {
    let mut saved = serde_json::to_value(&online().durable).unwrap();
    saved["pending_voice"] = serde_json::json!([{
        "id": "nota-antigua", "role": "user", "text": "", "complete": true,
        "voice": {"duration_ms": 1200, "failure": {"code": "transcription_http_error",
            "provider_status": 400, "hub_status": 502}, "audio_available": true}
    }]);
    let durable: DurableState = serde_json::from_value(saved).unwrap();
    let restarted: State = durable.recovered().into();
    assert_eq!(shown(&restarted), ["nota-antigua"]);
    let existing = open(&restarted, "conversacion-a");
    assert!(shown(&existing).is_empty());
    let named = decide(
        &restarted,
        Event::Session {
            id: "conversacion-c".into(),
            agent_id: None,
        },
        5,
    )
    .unwrap()
    .state;
    assert!(
        shown(&named).is_empty(),
        "no conversation adopts a note of unknown origin"
    );
}
