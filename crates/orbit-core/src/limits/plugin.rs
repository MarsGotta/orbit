//! Plugins: a call through the device and its answer (`contracts/plugin-v1.md`,
//! «Limits»).
use super::Limit;

/// The longest plugin `id`, the hub's `PLUGIN_ID_MAX_CHARS`: the call route
/// carries it as one path segment. Not a bound on a call's `name`.
pub const PLUGIN_ID_MAX_CHARS: usize = 32;
/// Copy of the hub's `PLUGIN_CALL_MAX_BYTES` (`server/orbit_hub/plugins.py`):
/// one call's body as the device writes it. It protects the hub's memory and
/// keeps a call within the 1 MiB a local order has; the device refuses past
/// it without opening a connection, so the app hears the cause from here.
/// It does not bound the answer or the data a plugin keeps.
pub const PLUGIN_CALL_MAX_BYTES: usize = 1_048_576;
/// One answer's whole body (value or refusal) as the device reads it from
/// the hub. It protects the device's memory while it holds the answer whole;
/// 4 MiB is the largest answer of the Notes feature today. It does not bound
/// the call or what a plugin keeps.
pub const PLUGIN_RESPONSE_MAX_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn limits() -> Vec<Limit> {
    vec![item!(
        PLUGIN_RESPONSE_MAX_BYTES,
        "bytes",
        "one plugin call's answer, read whole by the device"
    )]
}
