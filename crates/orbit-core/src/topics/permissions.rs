//! Last observed system permissions. Unknown is never a denial.
use crate::PermissionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub microphone: PermissionState,
    /// Both system audio and screen recording use macOS ScreenCapture preflight.
    pub screen_audio: PermissionState,
    pub screen_recording: PermissionState,
    pub accessibility: PermissionState,
    pub calendar: PermissionState,
}
