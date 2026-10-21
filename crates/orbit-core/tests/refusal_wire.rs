#![allow(clippy::unwrap_used, clippy::expect_used)]
use orbit_core::protocol::Response;
use orbit_core::{
    model::{Connection, State},
    transition::{Event, decide},
};
use serde_json::json;

#[test]
#[allow(clippy::useless_conversion)] // Also executed against the pre-unification base.
fn offline_submission_is_hub_unreachable_without_inventing_a_timestamp() {
    let refusal: orbit_core::refusal::Refusal = decide(
        &State::default(),
        Event::Submit {
            id: "offline".into(),
            text: "Pregunta sintética".into(),
        },
        123,
    )
    .unwrap_err()
    .into();
    assert_eq!(
        serde_json::to_value(refusal).unwrap(),
        json!({
            "code": "hub_unreachable", "since_ms": null,
            "retry": {"kind": "check_outcome"}
        })
    );
}

#[test]
fn decision_refusal_names_actual_text_size_and_ceiling() {
    let state = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let refusal = decide(
        &state,
        Event::Submit {
            id: "synthetic".into(),
            text: "x".repeat(32_769),
        },
        0,
    )
    .expect_err("the text exceeds the accepted ceiling");
    let message = refusal.to_string();
    assert!(message.contains("32769"), "missing actual size: {message}");
    assert!(message.contains("32768"), "missing ceiling: {message}");
}

#[test]
fn refused_response_has_structured_cause_and_keeps_legacy_text() {
    let response: Response = serde_json::from_value(json!({
        "v": 1, "id": "synthetic", "ok": false,
        "error": "Demasiados suscriptores",
        "refusal": {"code": "too_many", "resource": "subscriptions", "count": 5,
                    "max": 4, "retry": {"kind": "after_fix"}}
    }))
    .expect("the additive wire field must decode");
    let encoded = serde_json::to_value(response).expect("response must encode");
    assert_eq!(encoded["refusal"]["code"], "too_many");
    assert_eq!(encoded["refusal"]["count"], 5);
    assert_eq!(encoded["error"], "Demasiados suscriptores");
}
