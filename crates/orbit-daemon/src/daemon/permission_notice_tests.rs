//! Permission refusals through the native-event and call-detection entry points.
//! Synthetic helper events, local encrypted store and no native effects.
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::{PermissionState, call_detection::CallPreference, notice::NoticeClass};

const UNGRANTED: [PermissionState; 4] = [
    PermissionState::Denied,
    PermissionState::NotGranted,
    PermissionState::NotDetermined,
    PermissionState::Unknown,
];

fn assert_fixable(runtime: &Runtime, expected: NoticeCode) {
    assert_eq!(runtime.state.notice_code(), Some(expected));
    assert_eq!(expected.class(), NoticeClass::Fixable);
    assert!(runtime.state.durable.recording.is_none());
    assert!(!runtime.state.privacy_stopped());
}

#[tokio::test]
async fn voice_note_shortcut_names_the_microphone_permission() -> Result<()> {
    for permission in UNGRANTED {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.microphone_state",
            json!(permission),
        );
        runtime
            .native_event(json!({"event":"hotkey","action":"ptt","phase":"pressed"}))
            .await?;
        assert_fixable(&runtime, NoticeCode::MicrophonePermissionMissing);
        assert!(!runtime.state.durable.voice_active);
        while let Ok(command) = helper.try_recv() {
            assert_ne!(command["command"], "audio_start");
        }
    }
    Ok(())
}

async fn detected_recording_refusal(field: &str, expected: NoticeCode) -> Result<()> {
    for permission in UNGRANTED {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.features.call.clock_ms = Some(1_000);
        runtime.features.call.preference = Some(Ok(CallPreference::Record));
        crate::daemon::authority_fixture::set(&mut runtime.state, field, json!(permission));
        runtime
            .native_event(json!({"event":"microphone_activity","v":1,"applications":[{
                "application_pid":4242,"bundle_id":"us.zoom.xos","name":"zoom.us"
            }]}))
            .await?;
        runtime.features.call.clock_ms = Some(4_000);
        runtime.maintain_call_detection().await?;
        assert_fixable(&runtime, expected);
        while let Ok(command) = helper.try_recv() {
            assert_ne!(command["command"], "audio_start");
        }
    }
    Ok(())
}

#[tokio::test]
async fn detected_recording_names_the_microphone_permission() -> Result<()> {
    detected_recording_refusal(
        "capabilities.microphone_state",
        NoticeCode::MicrophonePermissionMissing,
    )
    .await
}

#[tokio::test]
async fn detected_recording_names_the_screen_permission() -> Result<()> {
    detected_recording_refusal(
        "capabilities.screen_state",
        NoticeCode::ScreenPermissionMissing,
    )
    .await
}
