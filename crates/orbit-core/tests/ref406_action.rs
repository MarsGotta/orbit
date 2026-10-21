#![allow(clippy::expect_used)]
use orbit_core::computer_control::ToolCall;

fn call_with_size(size: usize) -> ToolCall {
    let mut call = ToolCall {
        tool: "x".into(),
        arguments: serde_json::json!({"data": ""}),
    };
    let empty = serde_json::to_vec(&call).expect("fixture serializes").len();
    call.arguments = serde_json::json!({"data": "x".repeat(size - empty)});
    assert_eq!(
        serde_json::to_vec(&call).expect("fixture serializes").len(),
        size
    );
    call
}

#[test]
fn ref406_a3_action_boundary_is_the_encoded_action_with_numbers() {
    assert!(call_with_size(16_384).validate().is_ok());
    let error = call_with_size(16_385)
        .validate()
        .expect_err("one byte over");
    assert!(error.contains("16385"), "{error}");
    assert!(error.contains("16384"), "{error}");
}

#[test]
fn ref406_a2_receipt_has_a_separate_numbered_bound() {
    let limit = orbit_core::limits::MAX_ACTION_RECEIPT_BYTES;
    let empty = serde_json::to_vec(&serde_json::json!({"receipt": ""}))
        .expect("fixture serializes")
        .len();
    let within = serde_json::json!({"receipt": "x".repeat(limit - empty)});
    assert!(orbit_core::limits::bounded_action_receipt(&within).is_ok());
    let over = serde_json::json!({"receipt": "x".repeat(limit - empty + 1)});
    let error = orbit_core::limits::bounded_action_receipt(&over).expect_err("one byte over");
    assert!(error.contains(&(limit + 1).to_string()), "{error}");
    assert!(error.contains(&limit.to_string()), "{error}");
}
