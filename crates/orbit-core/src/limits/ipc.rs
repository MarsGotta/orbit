//! The app's IPC with the daemon: one line, one response, one command, and
//! the pages topics and reads put on it.
use super::Limit;

// The largest real IPC line is not measured. The existing 1 MiB socket reader
// and byte-exact synthetic boundary tests fix today's allocation ceiling.
// This protects one app IPC line, not a helper command or a hub frame.
pub const MAX_IPC_LINE_BYTES: usize = 1024 * 1024;
// The existing 1 MiB budget bounds a command response, matching the client
// reader. Its largest real payload is not measured; this preserves the
// allocation ceiling, not a command-body or agenda-topic limit.
pub const MAX_COMMAND_RESPONSE_BYTES: usize = 1024 * 1024;
// The largest real default command is not measured. The app IPC reader admits
// up to 1 MiB, so that existing admission bounds a default command, not the
// State or commands with their own tighter limits.
pub const MAX_DEFAULT_COMMAND_BYTES: usize = 1024 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        // Page ceilings are owned by the topic/read registries and calculated
        // here, never copied as another literal. They protect one page, not
        // retained topic or read history; see the measurements beside each.
        Limit {
            name: "TOPIC_TRANSPORT_MAX_BYTES",
            value: crate::topics::transport_max_bytes() as u64,
            unit: "bytes",
            resource: "largest topic transport page",
        },
        Limit {
            name: "READ_MAX_PAGE_BYTES",
            value: crate::reads::ReadId::ALL
                .iter()
                .map(|id| id.max_bytes() as u64)
                .fold(0, u64::max),
            unit: "bytes",
            resource: "largest paged read",
        },
        item!(MAX_IPC_LINE_BYTES, "bytes", "app IPC line"),
        item!(MAX_COMMAND_RESPONSE_BYTES, "bytes", "command response"),
        item!(MAX_DEFAULT_COMMAND_BYTES, "bytes", "default command"),
    ]
}
