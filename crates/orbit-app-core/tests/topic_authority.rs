#![allow(clippy::unwrap_used, clippy::expect_used)]
use orbit_app_core::{
    PermissionState,
    features::{FeatureInput, assistant, control, permissions, recording, status},
    topics::TopicCache,
};
use orbit_core::topics::{Knowledge, Pending, TopicData, TopicFrame, TopicId};

#[test]
fn review_pending_causes_use_stable_codes_with_their_numbers() {
    let snapshot = orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready);
    let cases = [
        (Pending::NotObservedYet, "pending:not_observed_yet"),
        (Pending::HelperStarting, "pending:helper_starting"),
        (Pending::Reading { since_ms: 123 }, "pending:reading:123"),
        (
            Pending::Transcription {
                completed: 2,
                pending: 3,
                failed: 4,
                queued: true,
            },
            "pending:transcription:2:3:4:true",
        ),
        (Pending::DaemonStarting, "pending:daemon_starting"),
    ];
    for (why, expected) in cases {
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "cause-test".into(),
                revision: 1,
                data: TopicData::Status(Knowledge::Unknown { why }),
            },
            Some(1_000),
        );
        let view = status::project(&FeatureInput::new(&snapshot, &topics));
        assert!(matches!(
            view.availability,
            orbit_app_core::features::FeatureAvailability::Unknown { cause } if cause == expected
        ));
    }
}

#[test]
fn unknown_topics_have_explicit_causes_in_all_daemon_owned_views() {
    let mut snapshot =
        orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready);
    snapshot.privacy_stopped = true;
    snapshot.meeting.phase = orbit_app_core::MeetingPhase::Recording;
    snapshot.notice = Some(orbit_core::notice::Notice::new(
        orbit_core::notice::NoticeCode::RecordingStarted,
    ));
    let mut topics = TopicCache::default();
    for id in [
        TopicId::Status,
        TopicId::Permissions,
        TopicId::Recording,
        TopicId::Assistant,
        TopicId::Control,
        TopicId::Notices,
    ] {
        topics.observe_at(
            TopicFrame {
                epoch: "current".into(),
                revision: 2,
                data: TopicData::not_observed(id),
            },
            Some(1_000),
        );
    }
    let input = FeatureInput::new(&snapshot, &topics);
    let status = status::project(&input);
    assert_eq!(
        serde_json::to_value(&status).unwrap()["availability"]["state"],
        "unknown"
    );
    assert!(!status.privacy_stopped);
    let permissions = permissions::project(&input);
    assert_eq!(
        serde_json::to_value(&permissions).unwrap()["availability"]["state"],
        "unknown"
    );
    assert_eq!(permissions.microphone, PermissionState::Unknown);
    let recording = recording::project(&input);
    let recording_json = serde_json::to_value(&recording).unwrap();
    assert_eq!(recording_json["availability"]["state"], "unknown");
    assert_eq!(recording_json["call_availability"]["state"], "unknown");
    assert_ne!(
        recording.meeting.phase,
        orbit_app_core::MeetingPhase::Recording
    );
    let assistant = assistant::project(&input);
    assert!(assistant.session_id.is_none());
    let assistant_json = serde_json::to_value(&assistant).unwrap();
    assert_eq!(assistant_json["availability"]["state"], "unknown");
    assert!(assistant_json["availability"]["cause"].as_str().is_some());
    assert_eq!(assistant_json["voice"]["availability"]["state"], "unknown");
    let control = control::project(&input).expect("unknown control view");
    assert_eq!(
        serde_json::to_value(&control).unwrap()["availability"]["state"],
        "unknown"
    );
    // The notice travels in the status function since the retirement of the
    // activity function that used to carry it.
    assert_eq!(
        serde_json::to_value(&status).unwrap()["notice_availability"]["state"],
        "unknown"
    );
    assert!(status.notice.is_none());
}
