//! A conversation is with one agent (hub-v1 «Agents»): the one chosen when it
//! began, or the one the hub listed it with when it was opened again.
#![allow(clippy::unwrap_used)]

use orbit_core::{Event, State, decide};

fn after(state: &State, event: Event) -> State {
    decide(state, event, 1).unwrap().state
}

#[test]
fn a_new_conversation_is_with_the_agent_chosen_and_keeps_it() {
    let state = after(
        &State::default(),
        Event::NewSession {
            agent_id: Some("atlas".into()),
        },
    );
    assert_eq!(state.agent_id(), Some("atlas"));
    // The hub names the conversation after its first turn: same agent.
    let state = after(&state, Event::Session("session-1".into()));
    assert_eq!(state.session_id().as_deref(), Some("session-1"));
    assert_eq!(state.agent_id(), Some("atlas"));
    // A conversation the hub no longer has starts again with the same agent.
    let state = after(
        &state,
        Event::Failure {
            code: "session_not_found".into(),
            numbers: Vec::new(),
        },
    );
    assert!(state.session_id().is_none());
    assert_eq!(state.agent_id(), Some("atlas"));
    // «Nueva conversación» without a choice is with the default agent.
    let state = after(&state, Event::NewSession { agent_id: None });
    assert_eq!(state.agent_id(), None);
}

#[test]
fn an_earlier_conversation_opens_with_its_own_agent() {
    let state = after(
        &State::default(),
        Event::NewSession {
            agent_id: Some("atlas".into()),
        },
    );
    let state = after(
        &state,
        Event::OpenSession {
            id: "session-vega".into(),
            agent_id: Some("vega".into()),
        },
    );
    assert_eq!(state.session_id().as_deref(), Some("session-vega"));
    assert_eq!(state.agent_id(), Some("vega"));
    // One listed by a hub without agents is the default agent's.
    let state = after(
        &state,
        Event::OpenSession {
            id: "session-old".into(),
            agent_id: None,
        },
    );
    assert_eq!(state.agent_id(), None);
}

#[test]
fn another_device_forgets_the_conversation_and_its_agent() {
    let state = after(
        &State::default(),
        Event::OpenSession {
            id: "session-vega".into(),
            agent_id: Some("vega".into()),
        },
    );
    let state = after(&state, Event::DeviceChanged);
    assert!(state.session_id().is_none());
    assert_eq!(state.agent_id(), None);
}

/// A state saved before agents loads with the default agent, and the
/// field is omitted while it is the default so such a state stays as it was.
#[test]
fn a_state_from_before_agents_is_with_the_default_agent() {
    let saved = serde_json::to_value(orbit_core::DurableState::default()).unwrap();
    assert!(saved.get("agent_id").is_none(), "{saved}");
    let loaded: orbit_core::DurableState = serde_json::from_value(saved).unwrap();
    assert_eq!(loaded.agent_id(), None);
    let chosen = after(
        &State::default(),
        Event::NewSession {
            agent_id: Some("atlas".into()),
        },
    );
    let saved = serde_json::to_value(&chosen.durable).unwrap();
    assert_eq!(saved["agent_id"], "atlas");
}

#[test]
fn a_failed_turn_keeps_its_numbers_and_closed_tokens_in_its_notice() {
    use orbit_core::notice::{DataValue, NoticeCode, NoticeKey, Token};
    let state = after(
        &State::default(),
        Event::Failure {
            code: "agent_rejected".into(),
            numbers: vec![
                (NoticeKey::AcpCode, "-32603".into()),
                (NoticeKey::StopReason, "refusal".into()),
                // Not a token: the notice keeps that there was a value, never the value.
                (NoticeKey::ExitStatus, "Hola, Vega".into()),
            ],
        },
    );
    let notice = state.durable.notice.as_ref().unwrap();
    assert_eq!(notice.code, NoticeCode::AssistantFailed);
    let token = |key| match notice.data.get(&key) {
        Some(DataValue::Token(token)) => token.clone(),
        other => panic!("{key:?} is not a token: {other:?}"),
    };
    assert_eq!(
        token(NoticeKey::Code),
        Token::new("agent_rejected").unwrap()
    );
    assert_eq!(token(NoticeKey::AcpCode), Token::new("-32603").unwrap());
    assert_eq!(token(NoticeKey::StopReason), Token::new("refusal").unwrap());
    assert_eq!(token(NoticeKey::ExitStatus), Token::unrecognized());
}
