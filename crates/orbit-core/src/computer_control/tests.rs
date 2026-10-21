#![allow(clippy::unwrap_used)]
use super::*;
use crate::protocol;
use serde_json::Value;

#[test]
fn dei12_a4_the_engine_path_carries_any_tool_the_engine_declares() {
    // Nothing here names a tool: an engine that grows one reaches Deimos
    // without a change in Orbit.
    for name in [
        "launch_app",
        "click",
        "type_text",
        "a_tool_invented_by_a_newer_engine",
    ] {
        let call = ToolCall {
            tool: name.into(),
            arguments: serde_json::json!({"bundle_id": "com.apple.calculator"}),
        };
        assert!(call.validate().is_ok(), "{name}");
    }
}

#[test]
fn dei12_a4_a_malformed_tool_call_is_refused_by_shape_not_by_name() {
    let oversize = "x".repeat(MAX_ACTION_BYTES + 1);
    let cases = [
        ToolCall {
            tool: String::new(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            tool: "tiene espacio".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            tool: "click".into(),
            arguments: serde_json::json!(["not", "an", "object"]),
        },
        ToolCall {
            tool: "type_text".into(),
            arguments: serde_json::json!({ "text": oversize }),
        },
    ];
    for case in cases {
        assert!(case.validate().is_err(), "{case:?}");
    }
}

/// DEI-16 A6/A7: a tool the engine describes at length is still a tool. The
/// 4 KiB bound that used to delete it here removed it from the catalogue with
/// no notice to Mars, none to the assistant and nothing in a log.
#[test]
fn dei16_a6_a_long_description_no_longer_makes_a_tool_invalid() {
    let verbose = EngineTool {
        name: "a_tool_the_engine_describes_at_length".into(),
        description: "á".repeat(8_000),
        input_schema: serde_json::json!({"type": "object"}),
    };
    assert!(verbose.description.len() > 4_096);
    assert!(verbose.is_valid(), "la descripción no decide si existe");

    // Only a name Orbit could not forward still disqualifies an entry.
    for name in ["", "tiene espacio", &"x".repeat(MAX_TOOL_NAME_BYTES + 1)] {
        let nameless = EngineTool {
            name: name.into(),
            description: "corta".into(),
            input_schema: Value::Null,
        };
        assert!(!nameless.is_valid(), "{name:?}");
    }
}

/// REF-11 A8/A9: the record a previous version kept may name an action with
/// the retired approval word. It still loads — the whole record, and with it
/// every new action, would be unreadable otherwise — and the word reads as an
/// end nobody saw, never as a state this version can produce.
#[test]
fn ref11_a_recorded_approval_word_loads_as_an_unknown_end() {
    let recorded = serde_json::json!({
        "operation_id": "op-1",
        "requested": "launch_app",
        "application": "com.apple.Safari",
        "at_ms": 1,
        "outcome": "awaiting_approval",
    });
    let entry: ControlHistoryEntry = serde_json::from_value(recorded).unwrap();
    assert_eq!(entry.outcome, ControlOutcome::Unknown);
    let written = serde_json::to_value(&entry).unwrap();
    assert_eq!(written["outcome"], "unknown");
    // A word that is not a string is still a malformed record, not a guess.
    let malformed = serde_json::json!({
        "operation_id": "op-1",
        "requested": "launch_app",
        "application": "com.apple.Safari",
        "at_ms": 1,
        "outcome": 7,
    });
    assert!(serde_json::from_value::<ControlHistoryEntry>(malformed).is_err());
}

#[test]
fn ref31_a3_history_names_the_request_channel_and_old_rows_remain_readable() {
    let current =
        ControlHistoryEntry::new("op-1", "click", "Safari", 1, ControlOutcome::Unknown).unwrap();
    let encoded = serde_json::to_value(&current).unwrap();
    assert_eq!(encoded["channel"], "local");

    let old = serde_json::json!({
        "operation_id": "op-2", "requested": "click", "application": "Safari",
        "at_ms": 2, "outcome": "awaiting_confirmation"
    });
    let restored: ControlHistoryEntry = serde_json::from_value(old).unwrap();
    assert_eq!(restored.outcome, ControlOutcome::Unknown);
    assert_eq!(
        serde_json::to_value(restored).unwrap()["channel"],
        "desconocido"
    );
}

/// REF-11: the control command set carries no switch any more.
#[test]
fn ref11_there_is_no_control_switch_command() {
    let frame = serde_json::json!({"command": "computer", "request": {"action": "set_control", "enabled": true}});
    assert!(serde_json::from_value::<protocol::Command>(frame).is_err());
    let remote =
        serde_json::json!({"command": "remote_switch", "observation": true, "action": null});
    assert!(serde_json::from_value::<protocol::Command>(remote).is_err());
}
