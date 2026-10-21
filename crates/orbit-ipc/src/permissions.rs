//! The names of the permissions Orbit can hold and the decoding of the
//! helper's report of them, in one place.
//!
//! Until SIM-01 this module also printed the terminal's permissions report; the
//! command line is retired and the app shows permissions from its own topic.
//!
//! Pure, so a malformed or partial payload can be replayed in a test without a
//! daemon, a Mac or a TCC prompt.

use orbit_core::PermissionState;
use orbit_core::calendar::CalendarAuthorization;
use orbit_core::wire::native::{NativeDecodeProblem, PermissionsResult, decode_result_body};
use serde_json::Value;

/// Every permission Orbit can hold, in the order a person needs them.
pub const REQUESTABLE: [&str; 4] = ["microphone", "screen", "accessibility", "calendars"];

/// Each permission's name, derived from the list above so no other file
/// spells it again. Adding a permission to `REQUESTABLE` is the only edit a
/// new permission needs; a second copy of a name is what this module exists
/// to make impossible.
pub const MICROPHONE: &str = REQUESTABLE[0];
pub const SCREEN: &str = REQUESTABLE[1];
pub const ACCESSIBILITY: &str = REQUESTABLE[2];
pub const CALENDARS: &str = REQUESTABLE[3];

/// The helper's word for a granted permission on its wire.
const AUTHORIZED: &str = "authorized";
const DENIED: &str = "denied";
const NOT_DETERMINED: &str = "notDetermined";
/// macOS restricts the permission (parental controls, a profile): a known
/// fact that Mars cannot grant from here, read as the calendar reads it.
const RESTRICTED: &str = "restricted";

fn microphone_state(value: &str) -> Result<PermissionState, NativeDecodeProblem> {
    match value {
        AUTHORIZED => Ok(PermissionState::Granted),
        DENIED | RESTRICTED => Ok(PermissionState::Denied),
        NOT_DETERMINED | "not_determined" => Ok(PermissionState::NotDetermined),
        _ => Err(NativeDecodeProblem {
            event: "response",
            command: "permissions".into(),
            field: MICROPHONE.into(),
            observed_type: "unrecognized_string",
        }),
    }
}

/// Reads one helper payload of permission observations.
///
/// Pure: no daemon, no Mac, no state of its own, so a malformed or partial
/// payload can be replayed in a test. The daemon used to index this JSON with
/// the same four names hand-copied a second time, which is how the microphone
/// string and the report's spelling of it could ever disagree.
///
/// No authority is written here: a malformed payload leaves the caller's
/// previous observation alone instead of inventing a denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedPermissions {
    pub microphone: PermissionState,
    pub screen: PermissionState,
    pub accessibility: PermissionState,
    pub calendar: CalendarAuthorization,
}

pub fn observe(payload: &Value) -> Result<ObservedPermissions, NativeDecodeProblem> {
    let result: PermissionsResult = decode_result_body("permissions", payload)?;
    let microphone = microphone_state(&result.microphone)?;
    let calendar = serde_json::from_value(Value::String(result.calendars)).map_err(|_| {
        NativeDecodeProblem {
            event: "response",
            command: "permissions".into(),
            field: CALENDARS.into(),
            observed_type: "unrecognized_string",
        }
    })?;
    // These two macOS preflights are booleans, not evidence of a TCC denial.
    let screen = if result.screen {
        PermissionState::Granted
    } else {
        PermissionState::NotGranted
    };
    let accessibility = if result.accessibility {
        PermissionState::Granted
    } else {
        PermissionState::NotGranted
    };
    Ok(ObservedPermissions {
        microphone,
        screen,
        accessibility,
        calendar,
    })
}

#[cfg(test)]
mod tests {
    //! Fixture setup only; production code above never panics.
    #![allow(clippy::expect_used)]
    use super::*;
    use orbit_core::Capabilities;

    fn set(capabilities: &mut Capabilities, field: &str, value: Value) {
        let mut wire = serde_json::to_value(&*capabilities).expect("capabilities serialize");
        if let Some(permission) = field.strip_suffix("_state") {
            wire[format!("{permission}_not_granted")] =
                Value::Bool(value == Value::String("not_granted".into()));
        }
        wire[field] = value;
        *capabilities = serde_json::from_value(wire).expect("capabilities deserialize");
    }

    fn observe(
        payload: &Value,
        capabilities: &mut Capabilities,
    ) -> Result<Option<CalendarAuthorization>, NativeDecodeProblem> {
        let observed = super::observe(payload)?;
        set(
            capabilities,
            "microphone_state",
            serde_json::json!(observed.microphone),
        );
        set(
            capabilities,
            "screen_state",
            serde_json::json!(observed.screen),
        );
        set(
            capabilities,
            "accessibility_state",
            serde_json::json!(observed.accessibility),
        );
        Ok(Some(observed.calendar))
    }

    fn running(mic_granted: bool, screen_granted: bool, ax_granted: bool) -> Capabilities {
        let mut capabilities = Capabilities::default();
        set(&mut capabilities, "native_helper", serde_json::json!(true));
        set(
            &mut capabilities,
            "microphone_state",
            serde_json::json!(if mic_granted {
                PermissionState::Granted
            } else {
                PermissionState::Unknown
            }),
        );
        set(
            &mut capabilities,
            "screen_state",
            serde_json::json!(if screen_granted {
                PermissionState::Granted
            } else {
                PermissionState::Unknown
            }),
        );
        set(
            &mut capabilities,
            "accessibility_state",
            serde_json::json!(if ax_granted {
                PermissionState::Granted
            } else {
                PermissionState::Unknown
            }),
        );
        capabilities
    }

    #[test]
    fn a_helper_payload_updates_the_three_observations_it_carries() {
        // The daemon used to index this JSON with hand-copied names; a typo in
        // the copy meant one permission silently never updated. Decoding lives
        // here now, next to the names it must agree with.
        let payload = serde_json::json!({
            "microphone": "authorized",
            "screen": true,
            "accessibility": true,
            "calendars": "full_access",
        });
        let mut capabilities = Capabilities::default();
        let calendar = observe(&payload, &mut capabilities).expect("complete permissions fixture");
        assert_eq!(capabilities.microphone_state(), PermissionState::Granted);
        assert_eq!(capabilities.screen_state(), PermissionState::Granted);
        assert_eq!(capabilities.accessibility_state(), PermissionState::Granted);
        assert!(calendar.is_some());
    }

    #[test]
    fn a_payload_without_a_string_microphone_updates_nothing() {
        // This is the guard the old `is_string` check provided: a payload
        // shaped differently (or an unrelated object) must not zero out
        // observations Orbit already holds.
        let mut capabilities = running(true, true, true);
        let before = capabilities.clone();
        for payload in [
            serde_json::json!({"screen": true}),
            serde_json::json!({"microphone": true}),
        ] {
            assert!(observe(&payload, &mut capabilities).is_err());
            assert_eq!(
                capabilities, before,
                "a non-string microphone must not touch state"
            );
        }
    }

    #[test]
    fn ref404b_wrong_permission_field_type_preserves_prior_observations() {
        let mut capabilities = running(true, true, true);
        let before = capabilities.clone();
        let payload = serde_json::json!({
            "microphone": "denied",
            "screen": "yes",
            "accessibility": true,
            "calendars": "full_access"
        });
        let problem = observe(&payload, &mut capabilities).expect_err("wrong screen type");
        assert_eq!(problem.event, "response");
        assert_eq!(problem.command, "permissions");
        assert_eq!(problem.field, "screen");
        assert_eq!(problem.observed_type, "string");
        assert_eq!(capabilities, before);
    }

    #[test]
    fn ref206_a1_negative_preflight_is_known_not_granted_without_a_tcc_cause() {
        let mut capabilities = Capabilities::default();
        let payload = serde_json::json!({
            "microphone": "authorized",
            "screen": false,
            "accessibility": false,
            "calendars": "full_access",
        });
        assert!(observe(&payload, &mut capabilities).is_ok());
        assert_eq!(capabilities.screen_state(), PermissionState::NotGranted);
        assert_eq!(
            capabilities.accessibility_state(),
            PermissionState::NotGranted
        );
    }

    #[test]
    fn ref206_a1_native_microphone_not_determined_is_preserved() {
        let mut capabilities = Capabilities::default();
        let payload = serde_json::json!({
            "microphone": "not_determined", "screen": false,
            "accessibility": false, "calendars": "not_determined"
        });
        assert!(observe(&payload, &mut capabilities).is_ok());
        assert_eq!(
            capabilities.microphone_state(),
            PermissionState::NotDetermined
        );
        let swift =
            include_str!("../../../native/macos/Sources/OrbitNative/OrbitApplication.swift");
        let emitted = swift
            .split("default: mic = \"")
            .nth(1)
            .and_then(|tail| tail.split('"').next());
        assert_eq!(emitted, Some("not_determined"));
        assert_eq!(
            emitted.and_then(|value| microphone_state(value).ok()),
            Some(PermissionState::NotDetermined)
        );
    }

    #[test]
    fn an_unreadable_calendar_leaves_the_previous_observation_alone() {
        let mut capabilities = running(true, false, false);
        let observed = observe(
            &serde_json::json!({"microphone": "denied", "screen": true,
                "accessibility": true, "calendars": "not_a_state"}),
            &mut capabilities,
        );
        assert_eq!(
            observed.expect_err("invalid calendar authorization").field,
            CALENDARS
        );
        assert_eq!(capabilities.microphone_state(), PermissionState::Granted);
        assert_eq!(capabilities.calendar(), None);
    }

    #[test]
    fn every_requestable_name_has_exactly_one_spelling_in_this_module() {
        // The names are the single source this module exists to be. If one
        // disappears from REQUESTABLE its accessor goes stale silently, so the
        // list and the accessors are pinned together here.
        assert_eq!(MICROPHONE, REQUESTABLE[0]);
        assert_eq!(SCREEN, REQUESTABLE[1]);
        assert_eq!(ACCESSIBILITY, REQUESTABLE[2]);
        assert_eq!(CALENDARS, REQUESTABLE[3]);
    }

    #[test]
    fn a_restricted_microphone_is_a_known_refusal_not_unknown() {
        assert_eq!(microphone_state("restricted"), Ok(PermissionState::Denied));
    }
}
