#![allow(clippy::expect_used)]

use orbit_core::privacy_stop::StopRequester;
use orbit_core::{
    Connection, State,
    transition::{Event, decide},
};

#[test]
fn pressing_stop_again_attributes_it_without_retiring_twice() {
    let adopted = decide(&State::default(), Event::HelperLatched, 1)
        .expect("helper latch is a valid event")
        .state;
    let repeated = decide(&adopted, Event::PrivacyStop, 2)
        .expect("Alina may press stop while it is already latched");
    assert_eq!(repeated.state.privacy_stop_by(), Some(StopRequester::Mars));
    assert_eq!(repeated.state.generation(), adopted.generation());
    assert!(repeated.effects.is_empty());
}

#[test]
fn connecting_cannot_discard_a_revocation_or_start_while_stopped() {
    let revoked = decide(&State::default(), Event::Connect(Connection::Revoked), 1)
        .expect("revocation is a valid observation")
        .state;
    assert!(decide(&revoked, Event::Connecting, 2).is_err());
    assert!(decide(&revoked, Event::LocalRearm, 3).is_err());

    let stopped = decide(&State::default(), Event::PrivacyStop, 1)
        .expect("local stop is valid")
        .state;
    assert!(decide(&stopped, Event::Connecting, 2).is_err());
}

#[test]
fn helper_latch_drops_revocation_attribution_without_new_authority() {
    let revoked = decide(&State::default(), Event::Connect(Connection::Revoked), 1)
        .expect("revocation is valid")
        .state;
    assert_eq!(revoked.privacy_stop_by(), Some(StopRequester::Revocation));
    let repeated = decide(&revoked, Event::HelperLatched, 2).expect("helper latch is valid");
    assert!(repeated.state.privacy_stopped());
    assert_eq!(repeated.state.privacy_stop_by(), None);
    assert_eq!(repeated.state.generation(), revoked.generation());
    assert!(repeated.effects.is_empty());
    assert!(repeated.authority_change.is_none());
}

#[test]
fn published_0312_state_recovers_with_the_same_wire_fields() {
    // This fixture has the same git blob in f972121 (0.3.12) and this base.
    let wire: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/p4_01_response_with_two_views.json"))
            .expect("published state fixture");
    let state: orbit_core::State =
        serde_json::from_value(wire["state"].clone()).expect("0.3.12 state loads");
    // The meeting proposal view the fixture carries was retired in P9-08,
    // the publication view and proposal fields in SIM-03a, and the messages
    // in SIM-30: they are dropped, and every other field keeps its exact
    // wire shape.
    let mut expected = wire;
    let expected_state = expected["state"].as_object_mut().expect("state object");
    for retired in [
        "messages",
        "meeting_proposal",
        "publication",
        "proposal_review",
        "proposal_progress",
        "proposal_receipt",
    ] {
        expected_state
            .remove(retired)
            .expect("the fixture keeps its retired fields");
    }
    assert_eq!(
        serde_json::to_value(&state).expect("state serializes"),
        expected["state"]
    );
    let recovered = state.durable().recovered();
    assert_eq!(recovered.generation(), state.generation() + 1);
    assert!(!recovered.privacy_stopped());
    assert_eq!(recovered.connection(), orbit_core::Connection::Offline);
    let restored: orbit_core::DurableState = serde_json::from_value(
        serde_json::to_value(&recovered).expect("recovered state serializes"),
    )
    .expect("recovered state loads");
    assert_eq!(restored, recovered);
}
