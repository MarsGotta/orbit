//! What the native helper observed: its presence and the permissions.
use super::{PermissionState, is_false};
use serde::{Deserialize, Serialize};

/// Authority fields are read-only outside orbit-core; core transitions and recovery own writes.
/// `native_helper` is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::Capabilities::default();
/// state.native_helper = true;
/// ```
/// `microphone_state` is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::Capabilities::default();
/// state.microphone_state = orbit_core::PermissionState::Granted;
/// ```
/// `screen_state` is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::Capabilities::default();
/// state.screen_state = orbit_core::PermissionState::Granted;
/// ```
/// `accessibility_state` is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::Capabilities::default();
/// state.accessibility_state = orbit_core::PermissionState::Granted;
/// ```
/// `calendar` is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::Capabilities::default();
/// state.calendar = None;
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "CapabilitiesWire", into = "CapabilitiesWire")]
pub struct Capabilities {
    pub(crate) native_helper: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) microphone_state: PermissionState,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) screen_state: PermissionState,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) accessibility_state: PermissionState,
    pub wake_word: bool,
    #[serde(default)]
    pub voice_processing_supported: bool,
    #[serde(default)]
    pub voice_processing_available: bool,
    #[serde(default)]
    pub playback_reference_available: bool,
    /// Last calendar authorization the helper reported. `None` means nothing has
    /// been observed yet, which is not the same as denied: the helper sends this
    /// with every status, so a missing value means the app is not running.
    #[serde(default)]
    pub(crate) calendar: Option<crate::calendar::CalendarAuthorization>,
    /// How many calendars the Mac has, from the last complete listing: what
    /// Ajustes shows beside the permission, taken from the data and
    /// kept here rather than in the calendar view so that opening another
    /// screen cannot turn it into «0». `None` means not known yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendars: Option<u32>,
}

impl Capabilities {
    pub fn native_helper(&self) -> bool {
        self.native_helper
    }
    pub fn microphone_state(&self) -> PermissionState {
        self.microphone_state
    }
    pub fn screen_state(&self) -> PermissionState {
        self.screen_state
    }
    pub fn accessibility_state(&self) -> PermissionState {
        self.accessibility_state
    }
    pub fn calendar(&self) -> Option<crate::calendar::CalendarAuthorization> {
        self.calendar
    }
}

/// Snapshots written before the in-process app retain boolean permissions and
/// sidecars. Keep reading their observed non-grants without inferring a denial
/// from a legacy false field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CapabilitiesWire {
    native_helper: bool,
    #[serde(default)]
    microphone: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    microphone_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    microphone_not_granted: bool,
    #[serde(default)]
    screen_audio: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    screen_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    screen_not_granted: bool,
    #[serde(default)]
    accessibility: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    accessibility_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    accessibility_not_granted: bool,
    wake_word: bool,
    #[serde(default)]
    voice_processing_supported: bool,
    #[serde(default)]
    voice_processing_available: bool,
    #[serde(default)]
    playback_reference_available: bool,
    #[serde(default)]
    calendar: Option<crate::calendar::CalendarAuthorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calendars: Option<u32>,
}

impl From<Capabilities> for CapabilitiesWire {
    fn from(value: Capabilities) -> Self {
        let old_state = |state| {
            if state == PermissionState::NotGranted {
                PermissionState::Unknown
            } else {
                state
            }
        };
        Self {
            native_helper: value.native_helper,
            microphone: value.microphone_state == PermissionState::Granted,
            microphone_state: old_state(value.microphone_state),
            microphone_not_granted: value.microphone_state == PermissionState::NotGranted,
            screen_audio: value.screen_state == PermissionState::Granted,
            screen_state: old_state(value.screen_state),
            screen_not_granted: value.screen_state == PermissionState::NotGranted,
            accessibility: value.accessibility_state == PermissionState::Granted,
            accessibility_state: old_state(value.accessibility_state),
            accessibility_not_granted: value.accessibility_state == PermissionState::NotGranted,
            wake_word: value.wake_word,
            voice_processing_supported: value.voice_processing_supported,
            voice_processing_available: value.voice_processing_available,
            playback_reference_available: value.playback_reference_available,
            calendar: value.calendar,
            calendars: value.calendars,
        }
    }
}

impl From<CapabilitiesWire> for Capabilities {
    fn from(value: CapabilitiesWire) -> Self {
        // The legacy bools exist for old readers only. A missing typed state
        // stays Unknown for new readers, even when an old bool is true.
        let observed = |state, not_granted| {
            if not_granted {
                PermissionState::NotGranted
            } else {
                state
            }
        };
        Self {
            native_helper: value.native_helper,
            microphone_state: observed(value.microphone_state, value.microphone_not_granted),
            screen_state: observed(value.screen_state, value.screen_not_granted),
            accessibility_state: observed(
                value.accessibility_state,
                value.accessibility_not_granted,
            ),
            wake_word: value.wake_word,
            voice_processing_supported: value.voice_processing_supported,
            voice_processing_available: value.voice_processing_available,
            playback_reference_available: value.playback_reference_available,
            calendar: value.calendar,
            calendars: value.calendars,
        }
    }
}
