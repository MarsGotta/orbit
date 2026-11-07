//! What this device is and which groups of orders it handles, as its helper's
//! `ready` says (`contracts/native-v1.md`). The names are the ones a device
//! says when it pairs with the hub (`contracts/hub-v1.md`, «Device groups»),
//! registered in `contracts/constants.json`.
//!
//! A helper that says nothing is a Mac that handles every group, as every
//! helper before these fields; an empty list is a device that handles none.
//! These are not an agent's access groups (Settings › Assistants): those say
//! what an agent may use, these what a device can answer.
use serde::{Deserialize, Serialize};

/// What kind of device this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Mac,
    Phone,
}

impl DeviceKind {
    pub const ALL: [Self; 2] = [Self::Mac, Self::Phone];
}

/// A group of orders a device can handle. The microphone is not a group:
/// every device has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceGroup {
    /// System and application audio.
    SystemAudio,
    /// Windows and their screenshots.
    Screen,
    /// The text the person has selected.
    SelectedText,
    /// The folders the user gave the assistant.
    Folders,
    /// The local calendars.
    Calendar,
    /// Which applications use the microphone, to notice a call.
    CallDetection,
    /// Global keyboard shortcuts.
    Shortcuts,
    /// Acting on the computer through the engine.
    Control,
}

impl DeviceGroup {
    /// Every group, in the order the registry and the hub list them.
    pub const ALL: [Self; 8] = [
        Self::SystemAudio,
        Self::Screen,
        Self::SelectedText,
        Self::Folders,
        Self::Calendar,
        Self::CallDetection,
        Self::Shortcuts,
        Self::Control,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

// A profile keeps one bit per group in a `u8`: a ninth group needs a wider set.
const _: () = assert!(DeviceGroup::ALL.len() <= u8::BITS as usize);

/// What the device said it is and handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    kind: DeviceKind,
    /// One bit per [`DeviceGroup`] it handles.
    groups: u8,
}

impl Default for DeviceProfile {
    /// Today's helper, which says neither: a Mac that handles every group.
    fn default() -> Self {
        Self::declared(None, None)
    }
}

impl DeviceProfile {
    /// The profile a device declares. Without a kind it is a Mac; without a
    /// list it handles every group; an empty list, none.
    pub fn declared(kind: Option<DeviceKind>, groups: Option<&[DeviceGroup]>) -> Self {
        let listed = groups.unwrap_or(&DeviceGroup::ALL);
        Self {
            kind: kind.unwrap_or(DeviceKind::Mac),
            groups: listed.iter().fold(0, |set, group| set | group.bit()),
        }
    }

    pub fn kind(self) -> DeviceKind {
        self.kind
    }

    pub fn handles(self, group: DeviceGroup) -> bool {
        self.groups & group.bit() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_says_nothing_is_a_mac_that_handles_every_group() {
        let profile = DeviceProfile::default();
        assert_eq!(profile.kind(), DeviceKind::Mac);
        assert!(DeviceGroup::ALL.iter().all(|group| profile.handles(*group)));
        let phone = DeviceProfile::declared(Some(DeviceKind::Phone), None);
        assert_eq!(phone.kind(), DeviceKind::Phone);
        assert!(DeviceGroup::ALL.iter().all(|group| phone.handles(*group)));
    }

    #[test]
    fn an_empty_list_handles_nothing_and_a_list_only_what_it_names() {
        let nothing = DeviceProfile::declared(Some(DeviceKind::Phone), Some(&[]));
        assert!(
            DeviceGroup::ALL
                .iter()
                .all(|group| !nothing.handles(*group))
        );
        let screen = DeviceProfile::declared(None, Some(&[DeviceGroup::Screen]));
        assert_eq!(screen.kind(), DeviceKind::Mac);
        for group in DeviceGroup::ALL {
            assert_eq!(
                screen.handles(group),
                group == DeviceGroup::Screen,
                "{group:?}"
            );
        }
    }
}
