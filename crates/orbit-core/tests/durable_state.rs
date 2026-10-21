#![allow(clippy::unwrap_used)]

use orbit_core::{DurableState, Views, privacy_stop::StopRequester};
use serde_json::Value;

const GOLDEN_RESPONSE: &str = include_str!("fixtures/golden_response_with_two_views.json");
const IPC_VIEW_FIELDS: [&str; 7] = [
    "speaker_review",
    "calendar",
    "native_adapters",
    "app_capabilities",
    "computer_control",
    "computer_task",
    "recording_signal",
];
/// Fields a 0.3.14 daemon still served or saved that later work retired:
/// the meeting proposal (`meeting_proposal`), the meeting detection
/// (`meeting_detection`), publications and proposals, and the messages
/// (`messages`, now read from the hub). A snapshot that carries them must keep
/// loading; the fields are ignored, never restored.
const RETIRED_VIEWS: [&str; 7] = [
    "messages",
    "meeting_proposal",
    "meeting_detection",
    "publication",
    "proposal_review",
    "proposal_progress",
    "proposal_receipt",
];

fn legacy_snapshot() -> Value {
    let mut state = serde_json::from_str::<Value>(GOLDEN_RESPONSE).unwrap()["state"].clone();
    // The published fixture predates the detection view; a 0.3.14 daemon
    // with its screen open served it like this.
    state["meeting_detection"] = serde_json::json!({
        "request_id": "", "stage": "inactive", "detail": "Detección desactivada.",
        "rules": [], "activation": null, "inspection": null, "grants": []
    });
    state
}

#[test]
fn legacy_snapshot_ignores_ipc_views_when_loaded_as_durable_state() {
    let snapshot = legacy_snapshot();
    for retired in RETIRED_VIEWS {
        assert!(snapshot.get(retired).is_some(), "{retired}");
    }

    let durable: DurableState = serde_json::from_value(snapshot.clone()).unwrap();
    let views: Views = serde_json::from_value(snapshot).unwrap();
    let value = serde_json::to_value(durable).unwrap();
    let views = serde_json::to_value(views).unwrap();

    assert_eq!(value["generation"], 7);
    assert_eq!(value["session_id"], "sesion-durable");
    for retired in RETIRED_VIEWS {
        assert!(views.get(retired).is_none(), "{retired}");
    }
    for field in IPC_VIEW_FIELDS.into_iter().chain(RETIRED_VIEWS) {
        assert!(
            value.get(field).is_none(),
            "durable state serialized IPC-only field {field}"
        );
    }
}

#[test]
fn recovered_durable_state_contains_no_ipc_view_fields() {
    let durable: DurableState = serde_json::from_value(legacy_snapshot()).unwrap();
    let recovered = serde_json::to_value(durable.recovered()).unwrap();

    // The snapshot was not stopped, and a restart invents no stop.
    assert_eq!(recovered["privacy_stopped"], false);
    assert_eq!(recovered["generation"], 8);
    assert_eq!(recovered["connection"], "offline");
    for field in IPC_VIEW_FIELDS.into_iter().chain(RETIRED_VIEWS) {
        assert!(
            recovered.get(field).is_none(),
            "recovery serialized IPC-only field {field}"
        );
    }
}

/// A state 0.9.0 saved while the user's stop was in force loads as the
/// user's stop and is saved again with the same bytes.
#[test]
fn a_stop_saved_by_0_9_0_round_trips_unchanged() {
    let mut snapshot = legacy_snapshot();
    snapshot["privacy_stopped"] = Value::Bool(true);
    snapshot["privacy_stop_by"] = Value::String("mars".into());
    let durable: DurableState = serde_json::from_value(snapshot).unwrap();
    assert_eq!(durable.privacy_stop_by(), Some(StopRequester::User));
    let saved = serde_json::to_value(&durable).unwrap();
    assert_eq!(saved["privacy_stop_by"], "mars");
    let again: DurableState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(again).unwrap(), saved);
}
