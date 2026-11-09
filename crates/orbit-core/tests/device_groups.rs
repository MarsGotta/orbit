//! What a device is and which groups of orders it handles, as every language
//! that holds them must say it. `contracts/constants.json` registers the
//! kinds, the groups in the order they are listed, and the group of each
//! order the hub relays to a device; this compares each entry with Rust's.
//! The names are the hub's (a device says them at pairing) and the helper's
//! (its `ready` says them).
use orbit_core::device::{DeviceGroup, DeviceKind};
use orbit_core::device_bridge::Capability;
use serde_json::{Map, Value, json};
use std::error::Error;

type Outcome = Result<(), Box<dyn Error>>;

/// The value `contracts/constants.json` registers under `name`.
fn registered(name: &str) -> Result<Value, Box<dyn Error>> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/constants.json");
    let entries: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let entry = entries
        .into_iter()
        .find(|entry| entry["name"] == name)
        .ok_or_else(|| format!("contracts/constants.json registers no {name}"))?;
    Ok(entry["value"].clone())
}

#[test]
fn the_registry_names_the_kinds_of_device_as_rust_does() -> Outcome {
    let rust = serde_json::to_value(DeviceKind::ALL)?;
    assert_eq!(rust, json!(["mac", "phone"]));
    assert_eq!(registered("DEVICE_KINDS")?, rust);
    Ok(())
}

#[test]
fn the_registry_lists_the_device_groups_in_the_order_rust_has_them() -> Outcome {
    let rust = serde_json::to_value(DeviceGroup::ALL)?;
    // The microphone is not a group: every device has one.
    assert_eq!(
        rust,
        json!([
            "system_audio",
            "screen",
            "selected_text",
            "folders",
            "calendar",
            "call_detection",
            "shortcuts",
            "control"
        ])
    );
    assert_eq!(registered("DEVICE_GROUPS")?, rust);
    Ok(())
}

#[test]
fn the_registry_names_the_group_of_every_order_the_hub_relays_as_rust_does() -> Outcome {
    let mut rust = Map::new();
    for capability in Capability::ALL {
        rust.insert(
            capability.wire_name().to_owned(),
            serde_json::to_value(capability.device_group())?,
        );
    }
    let rust = Value::Object(rust);
    // A device's groups are not an agent's access groups: the selected text
    // is its own group here, not the screen's.
    assert_eq!(
        rust,
        json!({
            "screen_sources": "screen",
            "screenshot": "screen",
            "selected_text": "selected_text",
            "control_tools": "control",
            "perform_action": "control",
            "calendar_read": "calendar",
            "files_list": "folders",
            "files_search": "folders",
            "files_read": "folders"
        })
    );
    assert_eq!(registered("DEVICE_CAPABILITY_GROUPS")?, rust);
    Ok(())
}
