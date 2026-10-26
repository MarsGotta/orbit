//! The native helper and what it observed: whether it runs, the four
//! permissions and the calendar's authorization. Its part of the state is
//! [`Capabilities`].
//!
//! The microphone guard reads the privacy stop besides this part.
use crate::calendar::CalendarAuthorization;
use crate::model::{Capabilities, DurableState, PermissionState};
use crate::refusal::{Permission, Refusal};

pub(super) fn ready(capabilities: &mut Capabilities) {
    capabilities.native_helper = true;
}

/// Nothing the helper observed holds without it.
pub(super) fn lost(capabilities: &mut Capabilities) {
    *capabilities = Capabilities::default();
}

/// The permissions the helper reported: the microphone, the screen and
/// accessibility, in that order, and the calendar's authorization.
pub(super) fn observed(
    capabilities: &mut Capabilities,
    [microphone, screen, accessibility]: [PermissionState; 3],
    calendar: CalendarAuthorization,
) {
    capabilities.microphone_state = microphone;
    capabilities.screen_state = screen;
    capabilities.accessibility_state = accessibility;
    capabilities.calendar = Some(calendar);
}

pub(super) fn calendar(capabilities: &mut Capabilities, authorization: CalendarAuthorization) {
    capabilities.calendar = Some(authorization);
}

/// The helper did not take a stop: it is not there to take anything else.
pub(super) fn stop_undelivered(capabilities: &mut Capabilities) {
    capabilities.native_helper = false;
}

/// Whether the microphone may be used: Orbit working, the helper present
/// and the permission granted.
pub(super) fn ensure_microphone(before: &DurableState) -> Result<(), Refusal> {
    before.privacy.ensure_active()?;
    // Named apart on purpose: the helper being gone is a different problem from
    // a permission being refused, and only one of them is fixed in Settings.
    if !before.capabilities.native_helper {
        return Err(Refusal::HelperNotRunning);
    }
    permission_refusal(before.capabilities.microphone_state, Permission::Microphone)
        .map_or(Ok(()), Err)
}

/// Whether the screen's audio may be recorded.
pub(super) fn ensure_screen(before: &DurableState) -> Result<(), Refusal> {
    permission_refusal(
        before.capabilities.screen_state,
        Permission::ScreenRecording,
    )
    .map_or(Ok(()), Err)
}

fn permission_refusal(state: PermissionState, permission: Permission) -> Option<Refusal> {
    match state {
        PermissionState::Granted => None,
        PermissionState::Denied => Some(Refusal::PermissionDenied { permission }),
        PermissionState::NotGranted => Some(Refusal::PermissionNotGranted { permission }),
        PermissionState::NotDetermined | PermissionState::Unknown => {
            Some(Refusal::PermissionUnknown { permission })
        }
    }
}

/// Observed helper facts, routed together to keep the coordinator small.
pub(super) fn event(capabilities: &mut crate::Capabilities, event: crate::Event) {
    match event {
        crate::Event::HelperReady => ready(capabilities),
        crate::Event::HelperLost => lost(capabilities),
        crate::Event::PermissionObserved {
            microphone,
            screen,
            accessibility,
            calendar,
        } => observed(capabilities, [microphone, screen, accessibility], calendar),
        crate::Event::CalendarAuthorization(value) => calendar(capabilities, value),
        crate::Event::HelperStopUndelivered => stop_undelivered(capabilities),
        _ => {}
    }
}
