//! The hub's warnings for the open conversation reach the chat the app paints.
#![allow(clippy::expect_used)]

use orbit_app_core::{
    AppSnapshot, DaemonConnection,
    features::{FeatureInput, assistant},
    topics::TopicCache,
};
use orbit_core::topics::{Knowledge, TopicData, TopicFrame, assistant::Value};

fn chat(value: serde_json::Value) -> serde_json::Value {
    let value: Value = serde_json::from_value(value).expect("an assistant topic value");
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "warnings".into(),
            revision: 1,
            data: TopicData::Assistant(Knowledge::Present {
                value,
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let snapshot = AppSnapshot::empty(1, DaemonConnection::Ready);
    serde_json::to_value(assistant::project(&FeatureInput::new(&snapshot, &topics)))
        .expect("the chat encodes")
}

fn topic(warnings: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "session_id": "session-warnings",
        "voice_active": false,
        "turn_in_flight": true,
        "voice": {"state": "present"},
        "warnings": warnings,
    })
}

#[test]
fn the_warnings_reach_the_chat_as_the_hub_wrote_them() {
    let said = [
        "Hermes withdrew its clarify question (session_closed); it can no longer be answered",
        "Hermes asked for sudo, which Orbit cannot answer",
    ];
    assert_eq!(chat(topic(&said))["warnings"], serde_json::json!(said));
}

#[test]
fn a_chat_without_warnings_says_none() {
    let chat = chat(topic(&[]));
    assert!(
        chat["warnings"].is_null()
            || chat["warnings"]
                .as_array()
                .is_some_and(std::vec::Vec::is_empty),
        "{}",
        chat["warnings"]
    );
}
