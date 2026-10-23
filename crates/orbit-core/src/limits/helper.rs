//! The link with the Mac's helper: one line each way on its stdio.
use super::Limit;

// A 4 MiB playback clip becomes 5,592,408 base64 bytes; the measured JSON
// command with a UUID id and 128-byte playback/provider fields is 5,592,815
// bytes including newline, as measured.
// Six MiB protects helper stdin and is not a bound on helper events.
pub const MAX_HELPER_COMMAND_LINE_BYTES: usize = 6 * 1024 * 1024;
// The largest helper event is a screenshot response: a 700,000-byte PNG cap
// expands to 933,336 base64 bytes. Serializing its response envelope with
// maximum-width binding fields measured 934,136 bytes including newline in
// five fixture runs (89.1% of 1 MiB). This existing stdout reader ceiling is
// not stdin's playback-clip budget.
pub const MAX_HELPER_EVENT_LINE_BYTES: usize = 1024 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![
        item!(MAX_HELPER_COMMAND_LINE_BYTES, "bytes", "helper stdin line"),
        item!(MAX_HELPER_EVENT_LINE_BYTES, "bytes", "helper stdout line"),
    ]
}
