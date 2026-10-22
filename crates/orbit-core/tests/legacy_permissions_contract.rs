#![allow(clippy::unwrap_used)]

use orbit_core::State;
use serde_json::Value;

const GOLDEN_RESPONSE: &str = include_str!("fixtures/golden_response_with_two_views.json");

#[test]
fn saved_permissions_preserve_non_grants_without_inventing_legacy_observations() {
    let ready = orbit_core::decide(&State::default(), orbit_core::Event::HelperReady, 0)
        .unwrap()
        .state;
    let observed = orbit_core::decide(
        &ready,
        orbit_core::Event::PermissionObserved {
            microphone: orbit_core::PermissionState::NotGranted,
            screen: orbit_core::PermissionState::NotGranted,
            accessibility: orbit_core::PermissionState::NotGranted,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        1,
    )
    .unwrap()
    .state;
    let saved = serde_json::to_value(observed.durable()).unwrap();
    let restored: orbit_core::DurableState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.capabilities(), observed.capabilities());
    assert_eq!(
        restored.capabilities().microphone_state(),
        orbit_core::PermissionState::NotGranted
    );
    assert_eq!(
        restored.capabilities().screen_state(),
        orbit_core::PermissionState::NotGranted
    );
    assert_eq!(
        restored.capabilities().accessibility_state(),
        orbit_core::PermissionState::NotGranted
    );
    for legacy_fields in [true, false] {
        let mut legacy: Value = serde_json::from_str(GOLDEN_RESPONSE).unwrap();
        let capabilities = legacy["state"]["capabilities"].as_object_mut().unwrap();
        if legacy_fields {
            capabilities.insert("microphone".into(), Value::Bool(true));
            capabilities.insert("screen_audio".into(), Value::Bool(false));
            capabilities.insert("accessibility".into(), Value::Bool(false));
        } else {
            for field in ["microphone", "screen_audio", "accessibility"] {
                capabilities.remove(field);
            }
        }
        let state: State = serde_json::from_value(legacy["state"].clone()).unwrap();
        assert_eq!(
            state.capabilities().microphone_state(),
            orbit_core::PermissionState::Unknown
        );
        assert_eq!(
            state.capabilities().screen_state(),
            orbit_core::PermissionState::Unknown
        );
        assert_eq!(
            state.capabilities().accessibility_state(),
            orbit_core::PermissionState::Unknown
        );
    }
}

/// The audio-source and per-track fields are additive. A golden answer
/// written before they existed keeps loading with them at their defaults, and
/// a state that carries them round-trips intact. (The meeting history is gone:
/// the meetings are read from the hub.)
#[test]
fn sources_and_tracks_are_additive_on_the_golden_contract() {
    use orbit_core::{AudioSource, CapturePhase, Recording, State, TrackProgress};

    let golden: Value = serde_json::from_str(GOLDEN_RESPONSE).unwrap();
    let older: State = serde_json::from_value(golden["state"].clone()).unwrap();
    assert!(
        older.views.audio_sources.is_empty(),
        "an answer from before the audio inventory must keep loading with an empty inventory"
    );

    let mut state = State::default();
    state.views.audio_sources = vec![AudioSource {
        application_pid: 321,
        application_name: "Chrome".into(),
        bundle_id: "com.google.Chrome".into(),
    }];
    state.durable.recording = Some(Recording {
        id: "sources-round-trip".into(),
        title: "Reunión sintética".into(),
        started_at_ms: 1_700_000_000_000,
        ended_at_ms: None,
        expires_at_ms: 1_700_000_060_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::System,
        phase: CapturePhase::Recording,
        sealed_chunks: 3,
        gaps: 1,
        tracks: vec![TrackProgress {
            name: "mic_local".into(),
            sealed_chunks: 2,
            gaps: 1,
            last_chunk_at_ms: 1_700_000_000_500,
        }],
        finalizing_since_ms: None,
        ceiling_notice_given: false,
    });
    let encoded = serde_json::to_value(&state).unwrap();
    let restored: State = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored, state);
}
