//! The hub link: one frame each way and one HTTP response.
use super::Limit;

// The largest real inbound WebSocket frame is not measured. Its existing 1 MiB
// receiver gate bounds one incoming hub message, not the outbound 128 KiB frame.
pub const MAX_HUB_INCOMING_FRAME_BYTES: usize = 1024 * 1024;
// The largest real hub HTTP body is not measured. Its existing 1 MiB streaming
// gate bounds one response allocation, not the inbound WebSocket or IPC State.
pub const MAX_HUB_HTTP_BODY_BYTES: usize = 1024 * 1024;
// The hub config admits a 131,072-byte frame. This protects a frame sent to
// the server, not its larger inbound HTTP or WebSocket messages.
pub const MAX_HUB_OUTGOING_FRAME_BYTES: usize = 128 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        item!(
            MAX_HUB_INCOMING_FRAME_BYTES,
            "bytes",
            "hub inbound WebSocket frame"
        ),
        item!(MAX_HUB_HTTP_BODY_BYTES, "bytes", "hub HTTP response body"),
        item!(
            MAX_HUB_OUTGOING_FRAME_BYTES,
            "bytes",
            "hub outbound WebSocket frame"
        ),
    ]
}
