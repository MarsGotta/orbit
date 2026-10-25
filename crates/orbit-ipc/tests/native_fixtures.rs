use orbit_core::wire::native::{NativeCommand, NativeEvent};
use serde_json::Value;
use std::{collections::BTreeSet, error::Error, fs, path::Path};

#[test]
fn latching_stops_match_the_shared_fixture() -> Result<(), Box<dyn Error>> {
    let expected: BTreeSet<String> = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native-latching-stops.json"
    ))?;
    let actual: BTreeSet<String> = orbit_core::privacy_stop::LATCHING_HELPER_CODES
        .iter()
        .map(|code| (*code).into())
        .collect();
    assert_eq!(actual, expected);
    Ok(())
}

fn fixture_names(directory: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    fs::read_dir(directory)?
        .map(|entry| {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                Ok(path
                    .file_stem()
                    .ok_or("fixture without name")?
                    .to_string_lossy()
                    .into_owned())
            } else {
                Err(format!("unexpected fixture file: {}", path.display()).into())
            }
        })
        .collect()
}

fn command_name(command: &NativeCommand) -> &'static str {
    match command {
        NativeCommand::Hello { .. } => "hello",
        NativeCommand::Heartbeat { .. } => "heartbeat",
        NativeCommand::Status { .. } => "status",
        NativeCommand::CalendarDisconnect { .. } => "calendar_disconnect",
        NativeCommand::CalendarSources { .. } => "calendar_sources",
        NativeCommand::CalendarSnapshot { .. } => "calendar_snapshot",
        NativeCommand::CalendarSave { .. } => "calendar_save",
        NativeCommand::CalendarDelete { .. } => "calendar_delete",
        NativeCommand::PrivacyStop { .. } => "privacy_stop",
        NativeCommand::CaptureStop { .. } => "capture_stop",
        NativeCommand::PlaybackStop { .. } => "playback_stop",
        NativeCommand::PlaybackPause { .. } => "playback_pause",
        NativeCommand::PlaybackResume { .. } => "playback_resume",
        NativeCommand::Rearm { .. } => "rearm",
        NativeCommand::Shutdown { .. } => "shutdown",
        NativeCommand::Permissions { .. } => "permissions",
        NativeCommand::AudioStart { .. } => "audio_start",
        NativeCommand::AudioPurpose { .. } => "audio_purpose",
        NativeCommand::AudioMute { .. } => "audio_mute",
        NativeCommand::AudioWakePhrase { .. } => "audio_wake_phrase",
        NativeCommand::AudioStop { .. } => "audio_stop",
        NativeCommand::Playback { .. } => "playback",
        NativeCommand::PlaybackClip { .. } => "playback_clip",
        NativeCommand::ApplicationSources { .. } => "application_sources",
        NativeCommand::ScreenSources { .. } => "screen_sources",
        NativeCommand::Screenshot { .. } => "screenshot",
        NativeCommand::SelectedText { .. } => "selected_text",
        NativeCommand::AssistantFolderPick { .. } => "assistant_folder_pick",
        NativeCommand::AssistantFolders { .. } => "assistant_folders",
        NativeCommand::AssistantFilesList { .. } => "assistant_files_list",
        NativeCommand::AssistantFilesSearch { .. } => "assistant_files_search",
        NativeCommand::AssistantFilesRead { .. } => "assistant_files_read",
        NativeCommand::WindowText { .. } => "window_text",
        NativeCommand::PhotoJpeg { .. } => "photo_jpeg",
    }
}

fn event_name(event: &NativeEvent) -> &'static str {
    match event {
        NativeEvent::Ready { .. } => "ready",
        NativeEvent::State { .. } => "state",
        NativeEvent::Response { .. } => "response",
        NativeEvent::Error { .. } => "error",
        NativeEvent::Hotkey { .. } => "hotkey",
        NativeEvent::PrivacyStop { .. } => "privacy_stop",
        NativeEvent::CaptureStopped { .. } => "capture_stopped",
        NativeEvent::LocalRearm { .. } => "local_rearm",
        NativeEvent::CalendarInvalidated { .. } => "calendar_invalidated",
        NativeEvent::Gap { .. } => "gap",
        NativeEvent::Audio { .. } => "audio",
        NativeEvent::PlaybackProgress { .. } => "playback_progress",
        NativeEvent::MicrophoneActivity { .. } => "microphone_activity",
        NativeEvent::AssistantQuestion { .. } => "assistant_question",
        NativeEvent::VoiceTrigger { .. } => "voice_trigger",
        NativeEvent::Notice { .. } => "notice",
        NativeEvent::HelperDisconnected { .. } => "helper_disconnected",
    }
}

#[test]
fn every_native_variant_has_one_consumed_fixture() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/native");
    let expected_commands: BTreeSet<String> = NativeCommand::names()
        .iter()
        .map(|name| (*name).into())
        .collect();
    let expected_events: BTreeSet<String> = NativeEvent::names()
        .iter()
        .map(|name| (*name).into())
        .collect();
    assert_eq!(fixture_names(&root.join("commands"))?, expected_commands);
    assert_eq!(fixture_names(&root.join("events"))?, expected_events);
    // Result bodies are exercised by their production readers here, in the
    // core calendar and assistant-folder tests, and in the daemon's
    // audio/inventory tests.
    let consumed_results: BTreeSet<String> = [
        "hello",
        "heartbeat",
        "status",
        "privacy_stop",
        "capture_stop",
        "rearm",
        "permissions",
        "calendar_disconnect",
        "calendar_sources",
        "calendar_snapshot",
        "calendar_save",
        "calendar_delete",
        "audio_start",
        "audio_start-mic",
        "audio_mute",
        "audio_stop",
        "application_sources",
        "selected_text",
        "assistant_folder_pick",
        "assistant_folders",
        "assistant_files_list",
        "assistant_files_search",
        "assistant_files_read",
        "window_text",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(fixture_names(&root.join("results"))?, consumed_results);
    for name in expected_commands {
        let raw: Value = serde_json::from_slice(&fs::read(
            root.join("commands").join(format!("{name}.json")),
        )?)?;
        let command: NativeCommand = serde_json::from_value(raw.clone())?;
        assert_eq!(command_name(&command), name);
        assert_eq!(serde_json::to_value(command)?, raw);
    }
    for name in expected_events {
        let raw: Value =
            serde_json::from_slice(&fs::read(root.join("events").join(format!("{name}.json")))?)?;
        let event: NativeEvent = serde_json::from_value(raw.clone())?;
        assert_eq!(event_name(&event), name);
        assert_eq!(serde_json::to_value(event)?, raw);
    }
    Ok(())
}

#[test]
fn shared_result_fixtures_reach_the_production_readers() -> Result<(), Box<dyn Error>> {
    use orbit_core::calendar::CalendarAuthorization;
    use orbit_core::wire::native::{
        NativeCalendarStatusBody, NativeStateResult, decode_result_body,
    };
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/native/results");
    for command in [
        "hello",
        "heartbeat",
        "status",
        "privacy_stop",
        "capture_stop",
        "rearm",
    ] {
        let body: Value = serde_json::from_slice(&fs::read(root.join(format!("{command}.json")))?)?;
        let state: NativeStateResult = decode_result_body(command, &body)?;
        assert_eq!(state.privacy_stopped, command == "privacy_stop");
        assert_eq!(
            state.native_instance_id,
            "00000000-0000-4000-8000-000000000911"
        );
        assert_eq!(state.audio_processing.mode, "unprocessed");
        assert_eq!(
            state.calendar.authorization,
            CalendarAuthorization::FullAccess
        );
        if command == "status" {
            let permissions = orbit_ipc::permissions::observe(&body["permissions"])?;
            assert_eq!(permissions.microphone, orbit_core::PermissionState::Granted);
        }
    }
    let body: Value = serde_json::from_slice(&fs::read(root.join("calendar_disconnect.json"))?)?;
    let status: NativeCalendarStatusBody = decode_result_body("calendar_disconnect", &body)?;
    assert_eq!(status.0.authorization, CalendarAuthorization::FullAccess);
    assert!(!status.0.physical_read_pending);
    assert!(status.0.failure.is_none());
    let body: Value = serde_json::from_slice(&fs::read(root.join("permissions.json"))?)?;
    let permissions = orbit_ipc::permissions::observe(&body)?;
    assert_eq!(permissions.microphone, orbit_core::PermissionState::Granted);
    assert_eq!(permissions.calendar, CalendarAuthorization::FullAccess);
    let selected_body: Value = serde_json::from_slice(&fs::read(root.join("selected_text.json"))?)?;
    let selected: orbit_core::wire::native::SelectedTextResult =
        decode_result_body("selected_text", &selected_body)?;
    assert_eq!(selected.text, "texto sintético");
    assert_eq!(selected.binding.window_id, 42);
    assert_eq!(selected.range.length, 15);
    assert_eq!(selected.range.unit, "utf16_code_units");
    assert!(
        NativeCommand::from_parts(
            "retired".into(),
            "application_identity",
            serde_json::json!({"application_pid":4242})
        )
        .is_err()
    );
    Ok(())
}
