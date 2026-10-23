//! Fixture setup only; the production code in `daemon.rs` never panics.
#![allow(clippy::expect_used)]
use super::*;
use serde_json::json;

#[test]
fn a_hotkey_press_carries_what_the_mac_had_running() {
    let event = json!({
        "event": "hotkey",
        "action": "meeting_toggle",
        "applications": [
            {"application_pid": 42, "bundle_id": "us.zoom.xos", "name": "zoom.us"}
        ]
    });
    let applications = meeting_applications(&event);
    assert_eq!(applications.len(), 1);
    assert_eq!(applications[0].application_pid, 42);
}

#[test]
fn a_press_without_applications_chooses_nothing_instead_of_failing() {
    // Every earlier release sent exactly this, and an older Orbit.app paired
    // with a newer daemon still will.
    let event = json!({"event": "hotkey", "action": "meeting_toggle"});
    assert!(meeting_applications(&event).is_empty());
    assert_eq!(
        orbit_core::meeting_audio::choose(&meeting_applications(&event)),
        orbit_core::meeting_audio::AudioChoice::NoCandidate
    );
}

#[test]
fn a_malformed_row_is_dropped_rather_than_recorded_by_accident() {
    // A blank name or a zero PID would travel on as a real candidate and
    // could end up being the thing an hour of audio is captured from.
    let event = json!({
        "applications": [
            {"application_pid": 0, "bundle_id": "us.zoom.xos", "name": "zoom.us"},
            {"application_pid": 7, "bundle_id": "", "name": "sin bundle"},
            {"application_pid": 8, "bundle_id": "us.zoom.xos", "name": ""},
            {"application_pid": "nueve", "bundle_id": "us.zoom.xos", "name": "zoom.us"},
            {"bundle_id": "us.zoom.xos", "name": "zoom.us"}
        ]
    });
    assert!(meeting_applications(&event).is_empty());
}
