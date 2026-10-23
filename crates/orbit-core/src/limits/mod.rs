//! Shared transport and device bounds, each in its domain's file. Topic and
//! read page ceilings remain in `topics` and `reads`: their registries name
//! the resource and measured page. A new bound goes in its domain's file,
//! with its entry in that file's `limits()` when another language copies it.

use serde::Serialize;

#[derive(Serialize)]
pub struct Limit {
    pub name: &'static str,
    pub value: u64,
    pub unit: &'static str,
    pub resource: &'static str,
}

/// A registry entry for a constant of the file that uses it.
macro_rules! item {
    ($name:ident, $unit:literal, $resource:literal) => {
        Limit {
            name: stringify!($name),
            value: $name as u64,
            unit: $unit,
            resource: $resource,
        }
    };
}

mod assistant;
mod calendar;
mod control;
mod helper;
mod hub;
mod ipc;
mod notes;
mod recording;
mod voice;

pub use assistant::{MAX_ATTACHMENT_BYTES, MAX_SESSION_ID_BYTES};
pub use control::{
    CAPTURE_UPLOAD_TIMEOUT_SECONDS, MAX_ACTION_BYTES, MAX_ACTION_RECEIPT_BYTES,
    MAX_WINDOW_INVENTORY_BYTES, bounded_action_receipt,
};
pub use helper::{MAX_HELPER_COMMAND_LINE_BYTES, MAX_HELPER_EVENT_LINE_BYTES};
pub use hub::{
    MAX_HUB_HTTP_BODY_BYTES, MAX_HUB_INCOMING_FRAME_BYTES, MAX_HUB_OUTGOING_FRAME_BYTES,
};
pub use ipc::{MAX_COMMAND_RESPONSE_BYTES, MAX_DEFAULT_COMMAND_BYTES, MAX_IPC_LINE_BYTES};
pub use voice::MAX_SPEECH_CLIP_BYTES;

/// Every bound another language copies, domain by domain
/// (`contracts/constants.json` keeps its own order and finds each by name).
pub fn registry() -> Vec<Limit> {
    [
        ipc::limits(),
        helper::limits(),
        hub::limits(),
        control::limits(),
        voice::limits(),
        assistant::limits(),
        recording::limits(),
        notes::limits(),
        calendar::limits(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
