#![allow(clippy::expect_used)]
//! The short voice note reaches the composer with its length and the
//! daemon's minimum; without one the composer says nothing about it.

#[path = "support/assistant_topic.rs"]
mod assistant_topic;

use orbit_app_core::ShortVoiceNote;
use orbit_core::PermissionState;

#[test]
fn a_note_too_short_to_send_reaches_the_composer_with_both_numbers() {
    let mut value = assistant_topic::assistant_value();
    assert_eq!(
        assistant_topic::snapshot(value.clone(), PermissionState::Unknown)
            .chat
            .voice
            .too_short,
        None
    );
    value.short_voice_note = orbit_core::voice::short_voice_note(90);
    assert_eq!(
        assistant_topic::snapshot(value, PermissionState::Unknown)
            .chat
            .voice
            .too_short,
        Some(ShortVoiceNote {
            duration_ms: 90,
            minimum_ms: 160,
        })
    );
}
