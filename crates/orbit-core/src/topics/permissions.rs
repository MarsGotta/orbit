//! Last observed system permissions. Unknown is never a denial.
use crate::PermissionState;
use serde::{Deserialize, Serialize};

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// 317-byte measured permissions; 2 KiB catches growth, not a permission gate.
pub const MAX_BYTES: usize = 2 * 1024;
/// Published as soon as it changes.
pub const MIN_INTERVAL_MS: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub microphone: PermissionState,
    /// Both system audio and screen recording use macOS ScreenCapture preflight.
    pub screen_audio: PermissionState,
    pub screen_recording: PermissionState,
    pub accessibility: PermissionState,
    pub calendar: PermissionState,
}
