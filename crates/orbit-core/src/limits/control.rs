//! Control of the Mac: the engine's actions and receipts, the windows the
//! assistant may select and the captures it uploads.
use super::Limit;
use serde::Serialize;

// Existing screen observations and action receipts were both cut at 32 KiB.
// This separate receipt bound protects an engine result in the republished
// State; it is not a screen observation or a limit on the engine catalogue.
pub const MAX_ACTION_RECEIPT_BYTES: usize = 32 * 1024;

/// Checks one engine receipt before it enters the repeatedly command response.
/// The caller supplies the fact to check; this makes no I/O or retry decision.
pub fn bounded_action_receipt(value: &impl Serialize) -> Result<serde_json::Value, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| "Could not serialize the action receipt".to_owned())?;
    if bytes.len() > MAX_ACTION_RECEIPT_BYTES {
        return Err(format!(
            "Action receipt is {} bytes; maximum {MAX_ACTION_RECEIPT_BYTES}",
            bytes.len()
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Could not read the action receipt".to_owned())
}
// A JSON action has its own 16,384-byte contract, independent of the hub's
// request envelope. The hub-only bounds live in contracts/constants.json.
pub const MAX_ACTION_BYTES: usize = 16 * 1024;
// The upload ends before the hub's 30-second waiter. This bounds one upload,
// not a retry or a deadline for user consent.
pub const CAPTURE_UPLOAD_TIMEOUT_SECONDS: u64 = 20;
// A 160 KiB inventory leaves 2 KiB for the JSON envelope after bounded rows.
// It protects one window listing, not the image captured from a window.
pub const MAX_WINDOW_INVENTORY_BYTES: usize = 160 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        // One capture's PNG, mirrored by the hub's upload route
        // (server/orbit_hub/captures.py) and the helper's encoder
        // (Limits.swift); scripts/tests/test_constants_copies.py compares them.
        Limit {
            name: "MAX_SOURCE_BYTES",
            value: crate::sources::MAX_SOURCE_BYTES as u64,
            unit: "bytes",
            resource: "capture PNG",
        },
        item!(MAX_ACTION_RECEIPT_BYTES, "bytes", "engine action receipt"),
        item!(MAX_ACTION_BYTES, "bytes", "serialized engine action"),
        item!(CAPTURE_UPLOAD_TIMEOUT_SECONDS, "seconds", "capture upload"),
        item!(
            MAX_WINDOW_INVENTORY_BYTES,
            "bytes",
            "window selection inventory"
        ),
    ]
}
