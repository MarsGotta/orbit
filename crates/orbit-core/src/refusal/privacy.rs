//! Privacy and the system's permissions: the user's stop, and what macOS
//! has or has not granted.
use super::{Refusal, fmt};
use crate::{PermissionState, computer_control::engine::SystemPermissions};
use serde::{Deserialize, Serialize};

parameter_enum!(Permission { Microphone => "microphone", ScreenRecording => "screen_recording", Accessibility => "accessibility", ScreenAndAccessibility => "screen_and_accessibility", Calendar => "calendar", Automation => "automation" });

/// The one name for a set of missing system permissions, shared by the bridge
/// and the control topic. `grant` never reports an empty set; «both» is the
/// remaining real combination and the honest name for anything else built.
pub fn missing_permission_name(missing: SystemPermissions) -> Permission {
    match (missing.accessibility, missing.screen_recording) {
        (true, false) => Permission::Accessibility,
        (false, true) => Permission::ScreenRecording,
        (true, true) | (false, false) => Permission::ScreenAndAccessibility,
    }
}

/// Both the remote bridge and the control topic name the same macOS decision.
/// A permission still awaiting its first grant is not one macOS denied.
pub fn missing_system_permission_refusal(
    missing: SystemPermissions,
    accessibility: PermissionState,
    screen_recording: PermissionState,
) -> Refusal {
    let permission = missing_permission_name(missing);
    if (missing.accessibility && accessibility == PermissionState::NotGranted)
        || (missing.screen_recording && screen_recording == PermissionState::NotGranted)
    {
        Refusal::PermissionNotGranted { permission }
    } else {
        Refusal::PermissionDenied { permission }
    }
}

macro_rules! privacy_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            PrivacyStopped = 11 => "privacy_stopped" {
                since_ms: Option<i64>,
            }, retry Retry::AfterFix;
            PermissionDenied = 12 => "permission_denied" {
                permission: Permission,
            }, retry Retry::AfterFix;
            PermissionNotGranted = 13 => "permission_not_granted" {
                permission: Permission,
            }, retry Retry::AfterFix;
            PermissionUnknown = 14 => "permission_unknown" {
                permission: Permission,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use privacy_refusals;
