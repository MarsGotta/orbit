#![allow(clippy::expect_used)]
use orbit_core::vault::{VaultCall, VaultRead};
use serde_json::json;

#[test]
fn vault_calls_are_read_only_except_connection_settings() {
    let read: VaultCall = serde_json::from_value(json!({
        "call": "read", "request": {"operation": "read", "path": "People/Ada.md"}
    }))
    .expect("read contract");
    assert!(matches!(
        read,
        VaultCall::Read {
            request: VaultRead { .. }
        }
    ));
    assert!(
        serde_json::from_value::<VaultCall>(json!({"call":"write","path":"Ada.md","body":"no"}))
            .is_err()
    );
}
