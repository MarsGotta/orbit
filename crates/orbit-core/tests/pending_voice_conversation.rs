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
/// and old short refused notes are removed before a new view is shown.
#[test]
fn an_old_short_refused_note_is_removed_on_recovery() {
    let mut saved = serde_json::to_value(&online().durable).unwrap();
    saved["pending_voice"] = serde_json::json!([{
        "id": "nota-antigua", "role": "user", "text": "", "complete": true,
        "voice": {"duration_ms": 1200, "failure": {"code": "transcription_http_error",
            "provider_status": 400, "hub_status": 502}, "audio_available": true}
    }]);
    let durable: DurableState = serde_json::from_value(saved).unwrap();
    let restarted: State = durable.recovered().into();
    assert!(shown(&restarted).is_empty());
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

#[test]
fn a_later_new_view_never_adopts_an_earlier_views_note() {
    let noted = failed_note(&online(), "earlier-note");
    let fresh = decide(&noted, Event::NewSession { agent_id: None }, 4)
        .unwrap()
        .state;
    assert!(shown(&fresh).is_empty());
    assert_eq!(fresh.durable.assistant.pending_voice.len(), 1);
    let named = decide(
        &fresh,
        Event::Session {
            id: "later-session".into(),
            agent_id: None,
        },
        5,
    )
    .unwrap()
    .state;
    assert!(shown(&named).is_empty());
    assert_eq!(named.durable.assistant.pending_voice.len(), 1);
}

#[test]
fn recovery_removes_only_sessionless_joined_and_short_refused_or_empty_notes() {
    let mut saved = serde_json::to_value(&online().durable).unwrap();
    let cases = [
        (
            "joined",
            20_000,
            "voice_transcription_joined_next_turn",
            None,
            None,
        ),
        (
            "refused",
            1_000,
            "transcription_provider_refused",
            None,
            None,
        ),
        (
            "old-refused",
            2_000,
            "transcription_http_error",
            Some(400),
            None,
        ),
        ("empty", 1_000, "voice_transcription_empty", None, None),
        ("long", 20_000, "transcription_provider_refused", None, None),
        (
            "boundary",
            3_000,
            "transcription_provider_refused",
            None,
            None,
        ),
        ("offline", 1_000, "voice_hub_unreachable", None, None),
        (
            "other-error",
            1_000,
            "transcription_http_error",
            Some(503),
            None,
        ),
        (
            "assigned",
            1_000,
            "transcription_provider_refused",
            None,
            Some("known-session"),
        ),
    ];
    saved["pending_voice"] =
        serde_json::json!(cases.map(|(id, duration, code, provider, session)| {
            serde_json::json!({"id":id,"role":"user","text":"","complete":true,
            "conversation":{"session_id":session},
            "voice":{"duration_ms":duration,"failure":{"code":code,"provider_status":provider},
                "audio_available":true}})
        }));
    let loaded: DurableState = serde_json::from_value(saved).unwrap();
    let recovered = loaded.recovered();
    let ids: Vec<_> = recovered
        .assistant
        .pending_voice
        .iter()
        .map(|note| note.message.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["long", "boundary", "offline", "other-error", "assigned"]
    );
    assert_eq!(
        recovered.clone().recovered().assistant.pending_voice,
        recovered.assistant.pending_voice,
        "migration is idempotent"
    );
}

#[test]
fn a_detached_draft_survives_restart_and_its_receipt_names_only_its_notes() {
    let noted = failed_note(&online(), "first-draft");
    let fresh = decide(&noted, Event::NewSession { agent_id: None }, 4)
        .unwrap()
        .state;
    let saved = serde_json::to_value(&fresh.durable).unwrap();
    let restarted: State = serde_json::from_value::<DurableState>(saved)
        .unwrap()
        .recovered()
        .into();
    assert!(shown(&restarted).is_empty());
    let new_note = failed_note(&online_from(&restarted), "second-draft");
    let named = decide(
        &new_note,
        Event::VoiceSession {
            message_id: "first-draft".into(),
            session_id: "background-session".into(),
        },
        8,
    )
    .unwrap()
    .state;
    assert!(named.session_id().is_none());
    assert_eq!(
        named.durable.assistant.turn.as_ref().unwrap().user.id,
        "second-draft"
    );
    assert!(named.durable.assistant.pending_voice.is_empty());
}

fn online_from(state: &State) -> State {
    decide(state, Event::Connect(Connection::Online), 5)
        .unwrap()
        .state
}

#[test]
fn background_delivery_groups_only_the_notes_of_the_abandoned_view() {
    let first = failed_note(&online(), "first-note");
    let second = failed_note(&first, "second-note");
    let fresh = decide(&second, Event::NewSession { agent_id: None }, 4)
        .unwrap()
        .state;
    let current = failed_note(&fresh, "current-note");
    let named = decide(
        &current,
        Event::VoiceSession {
            message_id: "first-note".into(),
            session_id: "old-views-session".into(),
        },
        5,
    )
    .unwrap()
    .state;
    assert!(shown(&named).is_empty());
    assert_eq!(named.durable.assistant.pending_voice.len(), 1);
    let remaining = &named.durable.assistant.pending_voice[0];
    assert_eq!(remaining.message.id, "second-note");
    assert_eq!(
        remaining
            .conversation
            .as_ref()
            .unwrap()
            .session_id
            .as_deref(),
        Some("old-views-session")
    );
    assert_eq!(
        named.durable.assistant.turn.as_ref().unwrap().user.id,
        "current-note"
    );
    assert_eq!(shown(&open(&named, "old-views-session")), ["second-note"]);
}

#[test]
fn recovery_removes_an_old_sessionless_fragment_in_the_current_turn_too() {
    for session_id in [None, Some("existing-session")] {
        let noted = failed_note(&online(), "stored-note");
        let mut saved = serde_json::to_value(&noted.durable).unwrap();
        saved["session_id"] = serde_json::json!(session_id);
        saved["turn"]["user"]["voice"]["failure"]["code"] =
            serde_json::json!("transcription_provider_refused");
        let recovered = serde_json::from_value::<DurableState>(saved)
            .unwrap()
            .recovered();
        assert_eq!(recovered.assistant.turn.is_some(), session_id.is_some());
    }
}

#[test]
fn a_default_agents_note_is_adopted_when_the_first_turn_names_its_agent() {
    let noted = failed_note(&online(), "default-agent-note");
    let typed = decide(
        &noted,
        Event::Submit {
            id: "first-text".into(),
            text: "Una pregunta sintética".into(),
            attachments: Vec::new(),
        },
        4,
    )
    .unwrap()
    .state;
    let named = decide(
        &typed,
        Event::Session {
            id: "default-session".into(),
            agent_id: Some("synthetic-default-agent".into()),
        },
        5,
    )
    .unwrap()
    .state;
    assert_eq!(shown(&named), ["default-agent-note"]);
}
