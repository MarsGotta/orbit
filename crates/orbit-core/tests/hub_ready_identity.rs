//! Which hub answers: `ready` names the hub's own id, and a hub from before
//! that id still connects, with none.
use orbit_core::wire::hub::{HubEvent, HubEventKind, ReadyData};
use serde_json::{Value, json};

const DEVICE: &str = "5d7c1e2a-3b4f-4a6d-8e9f-0a1b2c3d4e5f";
const HUB: &str = "9e8d7c6b-5a49-4382-a716-f5e4d3c2b1a0";

fn ready(data: Value) -> ReadyData {
    let event = HubEvent::decode(json!({"v": 1, "event": "ready", "data": data}))
        .unwrap_or_else(|error| panic!("the ready did not decode: {error}"));
    match event.kind {
        HubEventKind::Ready(data) => data,
        other => panic!("not a ready: {other:?}"),
    }
}

#[test]
fn a_ready_from_an_older_hub_has_no_hub_id() {
    let data = ready(json!({
        "device_id": DEVICE,
        "hub_protocol": 1,
        "hub_version": "0.15.0",
    }));
    assert_eq!(data.hub_id, None);
    assert_eq!(data.device_id.as_deref(), Some(DEVICE));
}

#[test]
fn a_ready_says_which_hub_answers() {
    let data = ready(json!({
        "device_id": DEVICE,
        "hub_protocol": 1,
        "hub_version": "0.16.0",
        "hub_id": HUB,
    }));
    assert_eq!(data.hub_id.as_deref(), Some(HUB));
    assert_eq!(data.device_id.as_deref(), Some(DEVICE));
}
