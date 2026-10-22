#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_core::{AccountSummary, Message, PermissionState, State, Turn, VoiceNote};
use serde_json::Value;

const GOLDEN_RESPONSE: &str = include_str!("fixtures/golden_response_with_two_views.json");

/// The state a daemon with voice notes and tri-state permissions serves: a voice note with its
/// moment and duration, a reply after it, the account Ajustes shows, the vault publication policy, and the tri-state
/// permissions in the model; snapshots retain legacy permission fields.
fn state_with_new_fields() -> State {
    let mut state = State::default();
    state.durable.turn = Some(Turn {
        user: Message {
            attachments: Vec::new(),
            id: "voice-turn".into(),
            role: "user".into(),
            text: "Nota de voz sintética".into(),
            complete: true,
            at_ms: 1_700_000_000_000,
            voice: Some(VoiceNote {
                duration_ms: 1_500,
                ..VoiceNote::default()
            }),
        },
        answer: Some(Message {
            attachments: Vec::new(),
            id: "voice-turn:assistant".into(),
            role: "assistant".into(),
            text: "Respuesta sintética".into(),
            complete: true,
            at_ms: 1_700_000_001_000,
            voice: None,
        }),
    });
    state.views.account = Some(AccountSummary {
        hub_url: Some("https://hub.synthetic.invalid".into()),
        device_name: Some("Mac de la fixture".into()),
        device_id_short: Some("20000000".into()),
    });
    let mut wire = serde_json::to_value(state).unwrap();
    wire["capabilities"]["native_helper"] = serde_json::json!(true);
    wire["capabilities"]["microphone_state"] = serde_json::json!("granted");
    wire["capabilities"]["screen_state"] = serde_json::json!("denied");
    wire["capabilities"]["accessibility_state"] = serde_json::json!("not_determined");
    serde_json::from_value(wire).unwrap()
}

/// Removes one field from the JSON object at `pointer`, proving it was there.
fn removed(mut value: Value, pointer: &str, key: &str) -> Value {
    let field = value
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .and_then(|object| object.remove(key));
    assert!(field.is_some(), "field {key} missing under {pointer}");
    value
}

#[test]
fn a_snapshot_without_additive_fields_loads_with_defaults() {
    let mut served = serde_json::to_value(state_with_new_fields()).unwrap();
    for (pointer, key) in [
        ("/turn/user", "at_ms"),
        ("/turn/user", "voice"),
        ("/turn/answer", "at_ms"),
        ("/capabilities", "microphone_state"),
        ("/capabilities", "screen_state"),
        ("/capabilities", "accessibility_state"),
        ("", "account"),
    ] {
        served = removed(served, pointer, key);
    }
    let older: State = serde_json::from_value(served).unwrap();
    let turn = older.durable.turn.as_ref().unwrap();
    let answer = turn.answer.as_ref().unwrap();
    assert_eq!(turn.user.at_ms, 0);
    assert!(turn.user.voice.is_none());
    assert_eq!(answer.at_ms, 0);
    assert!(answer.voice.is_none());
    assert_eq!(
        older.capabilities().microphone_state(),
        PermissionState::Unknown
    );
    assert_eq!(
        older.capabilities().screen_state(),
        PermissionState::Unknown
    );
    assert_eq!(
        older.capabilities().accessibility_state(),
        PermissionState::Unknown
    );
    assert!(older.views.account.is_none());
    // Unrelated stored fields are untouched by the removal.
    assert_eq!(turn.user.text, "Nota de voz sintética");
    assert!(answer.complete);
    assert!(older.capabilities().native_helper());
}

#[test]
fn an_old_state_fixture_loads_in_the_new_daemon_with_defaults() {
    let golden: Value = serde_json::from_str(GOLDEN_RESPONSE).unwrap();
    let restored: State = serde_json::from_value(golden["state"].clone()).unwrap();
    // A state saved with its messages loads without a turn; the
    // conversation is the hub's.
    assert!(restored.durable.turn.is_none());
    assert!(restored.views.account.is_none());
}

/// The turn in progress is all the Mac keeps of a conversation, and it
/// survives a restart as before: the user's message stays,
/// the turn is no longer in flight and the notice says it was lost.
#[test]
fn the_turn_in_progress_survives_a_restart() {
    let mut state = state_with_new_fields();
    if let Some(turn) = state.durable.turn.as_mut() {
        // A typed question: a voice note without its transcript is marked
        // interrupted on restart, which the voice note contract tests cover.
        turn.user.voice = None;
        turn.answer = None;
    }
    state.durable.active_request = Some("voice-turn".into());
    let saved = serde_json::to_value(&state.durable).unwrap();
    assert!(
        saved.get("messages").is_none(),
        "no list of messages is saved"
    );
    let loaded: orbit_core::DurableState = serde_json::from_value(saved).unwrap();
    let recovered = State::from(loaded.recovered());
    assert_eq!(recovered.durable.turn, state.durable.turn);
    assert!(recovered.durable.active_request.is_none());
    assert_eq!(
        recovered.notice_code(),
        Some(orbit_core::notice::NoticeCode::AssistantTurnLostByRestart)
    );
}
