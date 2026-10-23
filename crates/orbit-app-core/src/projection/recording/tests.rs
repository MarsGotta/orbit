use super::*;
use crate::{
    features::FeatureInput,
    projection::test_support::{recording, track},
    topics::TopicCache,
};
use orbit_core::{
    Recording,
    audio_signal::{RecordingSignalView, SignalCondition},
    topics::{Knowledge, Pending, TopicData, TopicFrame, recording as topic_recording},
};
use serde_json::json;

fn head(recording: Option<Recording>) -> topic_recording::Value {
    topic_recording::Value {
        recording: recording.map(Box::new),
        recording_signal: None,
        pending_jobs: 0,
        offline: false,
        call: None,
        question: None,
        voice_trigger: None,
        microphone_muted: false,
        applications: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
    }
}

#[test]
fn clarification_and_active_recording_are_projected() {
    let meeting = project_meeting_topic(&head(Some(recording(CapturePhase::Recording))));
    assert_eq!(meeting.phase, MeetingPhase::Recording);
    assert_eq!(meeting.title.as_deref(), Some("Revisión semanal"));
    assert_eq!(meeting.started_at_ms, Some(1_234));

    let assistant: orbit_core::topics::assistant::Value = serde_json::from_value(json!({
        "session": null, "sessions": [], "messages": [], "has_older_messages": false,
        "pending": {"request_id":"clarify-1","description":"¿Qué calendario?","clarification":true},
        "voice_active":false,"turn_in_flight":false,"activity":null,
        "voice":{"state":"unknown","why":"not_observed_yet"},
        "call":{"state":"unknown","why":"not_observed_yet"}
    }))
    .expect("typed assistant fixture");
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "recording-fixture".into(),
            revision: 1,
            data: TopicData::Assistant(Knowledge::Present {
                value: assistant,
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let local = crate::AppSnapshot::default();
    let chat = crate::features::assistant::project(&FeatureInput::new(&local, &topics));
    assert_eq!(
        chat.pending_contract,
        Some(ChatPending::Clarify {
            id: "clarify-1".into(),
            text: "¿Qué calendario?".into(),
            choices: Vec::new(),
            questions: Vec::new(),
        })
    );
}

#[test]
fn denied_permissions_and_all_capture_phases_are_total() {
    let value = orbit_core::topics::permissions::Value {
        microphone: orbit_core::PermissionState::Denied,
        screen_audio: orbit_core::PermissionState::Denied,
        screen_recording: orbit_core::PermissionState::Denied,
        accessibility: orbit_core::PermissionState::Denied,
        calendar: orbit_core::PermissionState::Denied,
    };
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "recording-fixture".into(),
            revision: 1,
            data: TopicData::Permissions(Knowledge::Present {
                value,
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let local = crate::AppSnapshot::empty(10, DaemonConnection::Lost);
    let denied = crate::features::permissions::project(&FeatureInput::new(&local, &topics));
    assert_eq!(local.connection.daemon, DaemonConnection::Lost);
    assert_eq!(denied.microphone, PermissionState::Denied);
    assert_eq!(denied.screen, PermissionState::Denied);
    assert_eq!(denied.accessibility, PermissionState::Denied);
    assert_eq!(denied.calendars, PermissionState::Denied);
    for (source, expected) in [
        (CapturePhase::Starting, MeetingPhase::Starting),
        (CapturePhase::Recording, MeetingPhase::Recording),
        (CapturePhase::Paused, MeetingPhase::Paused),
        (CapturePhase::Finalizing, MeetingPhase::Finishing),
        (CapturePhase::Interrupted, MeetingPhase::Idle),
        (CapturePhase::Idle, MeetingPhase::Idle),
    ] {
        assert_eq!(
            project_meeting_topic(&head(Some(recording(source)))).phase,
            expected
        );
    }
}

#[test]
fn projects_recording_tracks_and_marks_a_stale_track_silent() {
    let mut value = head(Some(Recording {
        sealed_chunks: 7,
        gaps: 1,
        ..recording(CapturePhase::Recording)
    }));
    value.pending_jobs = 2;
    value.recording_signal = Some(Box::new(RecordingSignalView {
        recording_id: "recording-1".into(),
        capture_epoch_id: "capture-1".into(),
        mic_local: track(SignalCondition::Receiving, 48_000, 100),
        remote_app: Some(track(SignalCondition::Receiving, 48_000, 100)),
    }));
    let fresh = serde_json::to_value(project_meeting_topic(&value)).expect("meeting wire");
    assert_eq!(fresh["phase"], "recording");
    assert_eq!(fresh["recording_id"], "recording-1");
    assert_eq!(fresh["audio"], "system");
    assert_eq!(fresh["tracks"][0]["name"], "mic_local");
    assert_eq!(fresh["tracks"][0]["level"], "ok");
    assert_eq!(fresh["tracks"][1]["name"], "remote_app");
    assert_eq!(fresh["tracks"][1]["level"], "ok");
    value
        .recording_signal
        .as_mut()
        .expect("signal fixture")
        .remote_app = Some(track(SignalCondition::DeliveryDelayed, 48_000, 20_000));
    let stale = serde_json::to_value(project_meeting_topic(&value)).expect("meeting wire");
    assert_eq!(stale["tracks"][1]["level"], "silent");
}

/// The countdown reaches the app with the deadline the daemon
/// published; the app counts down to it, it never copies the 13 s.
#[test]
fn projects_the_countdown_and_the_preference() {
    let mut value = head(Some(recording(CapturePhase::Recording)));
    value.call = Some(Box::new(topic_recording::Call {
        preference: Some(orbit_core::call_detection::CallPreference::Record),
        preference_unreadable: None,
        panel: Some(topic_recording::CallPanel::Ending {
            recording_id: "recording-1".into(),
            application: "zoom.us".into(),
            ends_at_ms: 50_000,
        }),
    }));
    let meeting = project_meeting_topic(&value);
    assert_eq!(meeting.call_preference, Some(CallPreference::Record));
    assert_eq!(
        serde_json::to_value(&meeting.call_panel).expect("panel wire"),
        json!({"state":"ending","recording_id":"recording-1","application":"zoom.us",
            "ends_at_ms":50_000})
    );
}

#[test]
fn projects_the_finished_recording_as_recent() {
    let value = head(Some(Recording {
        ended_at_ms: Some(31_234),
        phase: CapturePhase::Idle,
        ..recording(CapturePhase::Idle)
    }));
    let recent = serde_json::to_value(project_meeting_topic(&value).recent).expect("recent wire");
    assert_eq!(
        recent[0],
        json!({
            "recording_id":"recording-1","title":"Revisión semanal","started_at_ms":1_234,
            "duration_ms":30_000,"calendar_event_id":null,
            "transcription":{"pending":0,"running":0,"completed":0,"failed":0},"summary":"none"
        })
    );
}

#[test]
fn publishes_the_applications_with_audio_and_why_the_list_is_empty() {
    let mut value = head(None);
    value.applications = Knowledge::Present {
        value: vec![
            topic_recording::Application {
                name: "Meet".into(),
                pid: 4_242,
            },
            topic_recording::Application {
                name: "Zoom".into(),
                pid: 77,
            },
        ],
        as_of_ms: None,
    };
    let meeting = project_meeting_topic(&value);
    assert_eq!(
        meeting
            .applications
            .iter()
            .map(|application| (application.name.as_str(), application.pid))
            .collect::<Vec<_>>(),
        vec![("Meet", 4_242), ("Zoom", 77)]
    );
    assert_eq!(meeting.applications_state, MeetingApplicationsState::Ready);
}

#[test]
fn an_empty_list_says_which_of_the_three_reasons_it_is() {
    let mut value = head(None);
    value.applications = Knowledge::Absent {
        refusal: orbit_core::refusal::Refusal::HelperNotRunning,
    };
    let helper_missing = project_meeting_topic(&value);
    assert!(helper_missing.applications.is_empty());
    assert_eq!(
        helper_missing.applications_state,
        MeetingApplicationsState::HelperMissing
    );
    value.applications = Knowledge::Present {
        value: Vec::new(),
        as_of_ms: None,
    };
    assert_eq!(
        project_meeting_topic(&value).applications_state,
        MeetingApplicationsState::Empty
    );
    value.applications = Knowledge::Absent {
        refusal: orbit_core::refusal::Refusal::PrivacyStopped {
            since_ms: Some(123),
        },
    };
    let stopped = project_meeting_topic(&value);
    assert_eq!(
        stopped.applications_state,
        MeetingApplicationsState::PrivacyStopped
    );
    assert!(
        stopped
            .applications_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("123"))
    );
}

/// A preference the store could not read is not one that was not
/// published yet: its cause reaches the app.
#[test]
fn an_unreadable_preference_carries_its_cause() {
    let mut value = head(None);
    value.call = Some(Box::new(topic_recording::Call {
        preference: None,
        preference_unreadable: Some("la fila ocupa 10000 bytes".into()),
        panel: None,
    }));
    let meeting = project_meeting_topic(&value);
    assert_eq!(meeting.call_preference, None);
    assert_eq!(
        meeting.call_preference_unreadable.as_deref(),
        Some("la fila ocupa 10000 bytes")
    );
    let mut published_later = head(None);
    published_later.call = None;
    assert_eq!(
        project_meeting_topic(&published_later).call_preference_unreadable,
        None
    );
}

/// The limits a recording has, ahead of time: before any recording the
/// app knows the one safety ceiling («Graba hasta 4 h»), and during one,
/// until when it records at most («hasta las 15:40») — the daemon's
/// deadline, never a copy of the constant.
#[test]
fn a_recording_says_its_limit_before_starting_and_its_end_while_it_runs() {
    let manual = orbit_core::RECORDING_CEILING_MINUTES;
    assert_eq!(crate::AppSnapshot::default().meeting.limit_minutes, manual);
    let idle = project_meeting_topic(&head(None));
    assert_eq!(idle.limit_minutes, manual);
    assert_eq!(idle.ends_at_ms, None);
    let running = project_meeting_topic(&head(Some(Recording {
        started_at_ms: 1_000,
        expires_at_ms: 1_000 + 78 * 60_000,
        ..recording(CapturePhase::Recording)
    })));
    assert_eq!(running.limit_minutes, manual);
    assert_eq!(running.ends_at_ms, Some(1_000 + 78 * 60_000));
    let wire = serde_json::to_value(&running).expect("meeting wire");
    assert_eq!(wire["limit_minutes"], 240);
}

/// «Micrófono silenciado» is what the daemon's topic says, never assumed.
#[test]
fn the_recording_card_says_muted_only_when_the_topic_does() {
    let mut value = head(Some(recording(CapturePhase::Recording)));
    assert!(!project_meeting_topic(&value).microphone_muted);
    value.microphone_muted = true;
    assert!(project_meeting_topic(&value).microphone_muted);
}
