//! The call shows whether it can see the screen from the macOS permission
//! and the local privacy stop.
#[path = "support/assistant_topic.rs"]
mod assistant_topic;

use orbit_core::{CallPhase, CallState, PermissionState, topics::Knowledge};

/// What the call shows about the screen follows the macOS permission and the
/// privacy stop, not a switch of Orbit.
#[test]
fn the_call_sees_the_screen_with_the_permission() {
    let call = |screen, privacy_stopped| {
        let mut value = assistant_topic::assistant_value();
        value.call = Knowledge::Present {
            value: Some(CallState {
                id: "20000000-0000-4000-8000-000000000011".into(),
                phase: CallPhase::Listening,
                started_at_ms: 1_000,
                expires_at_ms: 61_000,
                turns: 0,
                origin: orbit_core::CallOrigin::App,
                ceiling_notice_given: false,
            }),
            as_of_ms: None,
        };
        (if privacy_stopped {
            assistant_topic::snapshot_with_privacy(value, screen, true)
        } else {
            assistant_topic::snapshot(value, screen)
        })
        .call
        .map(|call| call.seeing_screen)
    };
    assert_eq!(call(PermissionState::Granted, false), Some(true));
    assert_eq!(call(PermissionState::Denied, false), Some(false));
    assert_eq!(
        call(PermissionState::Unknown, false),
        Some(false),
        "an unknown permission is not announced as granted"
    );
    assert_eq!(call(PermissionState::Granted, true), Some(false));
}

/// «Silenciar» during a call: the call view says it once the daemon's topic
/// does, and not before.
#[test]
fn the_call_says_muted_only_when_the_daemon_does() {
    let muted = |microphone_muted| {
        let mut value = assistant_topic::assistant_value();
        value.call = Knowledge::Present {
            value: Some(CallState {
                id: "20000000-0000-4000-8000-000000000012".into(),
                phase: CallPhase::Listening,
                started_at_ms: 1_000,
                expires_at_ms: 61_000,
                turns: 0,
                origin: orbit_core::CallOrigin::App,
                ceiling_notice_given: false,
            }),
            as_of_ms: None,
        };
        value.microphone_muted = microphone_muted;
        assistant_topic::snapshot(value, PermissionState::Granted)
            .call
            .map(|call| call.microphone_muted)
    };
    assert_eq!(muted(false), Some(false));
    assert_eq!(muted(true), Some(true));
}
