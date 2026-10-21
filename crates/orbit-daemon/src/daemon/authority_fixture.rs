//! Test-only state construction. Production has no route around `decide`.
#![allow(clippy::expect_used)]

use orbit_core::State;
use orbit_ipc::permissions::{ACCESSIBILITY, MICROPHONE, SCREEN};
use serde_json::Value;

pub(crate) fn set(state: &mut State, field: &str, value: Value) {
    let mut wire = serde_json::to_value(&*state).expect("state fixture serializes");
    let (parent, name) = match field.split_once('.') {
        Some(("capabilities", name)) => (&mut wire["capabilities"], name),
        None => (&mut wire, field),
        Some(_) => panic!("unsupported authority fixture field: {field}"),
    };
    if let Some(permission) = name.strip_suffix("_state")
        && matches!(permission, MICROPHONE | SCREEN | ACCESSIBILITY)
    {
        parent[format!("{permission}_not_granted")] =
            Value::Bool(value == Value::String("not_granted".into()));
    }
    parent[name] = value;
    *state = serde_json::from_value(wire).expect("state fixture deserializes");
}
