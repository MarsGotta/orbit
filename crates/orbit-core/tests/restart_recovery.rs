//! Restarts in the pure core: a restart keeps only the stop the user asked
//! for, and a recording that was running comes back interrupted, with its
//! fragments counted and nothing that restarts it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::{
    Capabilities, CapturePhase, Connection, DurableState, Effect, Event, Recording, State, decide,
    protocol::MeetingAudio,
};

fn recording(phase: CapturePhase) -> Recording {
    Recording {
        id: "restart-recording".into(),
        title: "Reunión sintética".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 3_601_000,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase,
        sealed_chunks: 12,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    }
}

fn said(state: &DurableState) -> Vec<Notice> {
    state.notice_log.records().to_vec()
}

#[test]
fn a_restart_without_the_user_stop_comes_back_working() {
    // An update, a crash or the Mac booting: nobody pressed anything.
    let mut wire = serde_json::to_value(DurableState::default()).unwrap();
    wire["generation"] = serde_json::json!(7);
    wire["connection"] = serde_json::json!("online");
    let running: DurableState = serde_json::from_value(wire).unwrap();
    let recovered = running.recovered();
    assert!(
        !recovered.privacy_stopped(),
        "a restart must not invent a stop Alina never asked for"
    );
    for notice in said(&recovered) {
        assert_ne!(
            notice.code,
            NoticeCode::PrivacyStopKept,
            "a restart without the user's stop asks for no rearm: {notice:?}"
        );
    }
    // What a restart really changes is still true: a new generation, no
    // connection yet and no capability until the helper reports again.
    assert_eq!(recovered.generation(), 8);
    assert_eq!(recovered.connection(), Connection::Offline);
    assert_eq!(recovered.capabilities(), &Capabilities::default());
}

#[test]
fn a_recording_that_was_running_stays_interrupted_with_its_fragments() {
    for phase in [
        CapturePhase::Starting,
        CapturePhase::Recording,
        CapturePhase::Finalizing,
    ] {
        let mut running = DurableState::default();
        running.capture.recording = Some(recording(phase.clone()));
        running.voice.voice_active = true;
        let recovered = running.recovered();
        assert!(!recovered.privacy_stopped(), "{phase:?}");
        let recording = recovered
            .capture
            .recording
            .as_ref()
            .expect("the recording survives");
        assert_eq!(recording.phase, CapturePhase::Interrupted, "{phase:?}");
        assert_eq!(
            recording.sealed_chunks, 12,
            "{phase:?}: the saved fragments stay"
        );
        assert!(!recovered.voice.voice_active, "{phase:?}");
        assert!(
            said(&recovered).contains(&Notice {
                sequence: Some(1),
                ..Notice::new(NoticeCode::RecordingInterruptedByRestart)
                    .with(NoticeKey::Fragments, 12_u64)
            }),
            "{phase:?}: the notice says it was interrupted, with what it kept: {:?}",
            said(&recovered)
        );
        // Nothing restarts it: the next tick, with the helper back and the
        // permission granted, emits no capture effect.
        let state = State::from(recovered);
        let ready = decide(&state, Event::HelperReady, 0).unwrap().state;
        let permitted = decide(
            &ready,
            Event::PermissionObserved {
                microphone: orbit_core::PermissionState::Granted,
                screen: orbit_core::PermissionState::Unknown,
                accessibility: orbit_core::PermissionState::Unknown,
                calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
            },
            0,
        )
        .unwrap()
        .state;
        let state = decide(&permitted, Event::Connect(Connection::Online), 0)
            .unwrap()
            .state;
        let tick = decide(&state, Event::Tick, 2_000).unwrap();
        assert!(
            !tick
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::StartCapture { .. })),
            "{phase:?}: an interrupted recording is never resumed on its own"
        );
    }
}

#[test]
fn recovery_does_not_relabel_a_phase_that_was_not_live() {
    for phase in [
        CapturePhase::Idle,
        CapturePhase::Paused,
        CapturePhase::Interrupted,
    ] {
        let mut running = DurableState::default();
        running.capture.recording = Some(recording(phase.clone()));
        let recovered = running.recovered();
        let recording = recovered
            .capture
            .recording
            .as_ref()
            .expect("recording survives");
        assert_eq!(recording.phase, phase);
        assert_eq!(recording.gaps, 0);
        assert_ne!(
            recovered.notice.as_ref().map(|notice| notice.code),
            Some(NoticeCode::RecordingInterruptedByRestart)
        );
    }
}

#[test]
fn the_user_stop_survives_the_restart_and_only_their_rearm_lifts_it() {
    let stopped = decide(&State::default(), Event::PrivacyStop, 0)
        .unwrap()
        .state;
    let recovered = State::from(stopped.durable().recovered());
    assert!(
        recovered.privacy_stopped(),
        "the user's stop survives the restart"
    );
    // Nothing that can arrive from the hub or the helper rearms it.
    for event in [
        Event::Connect(Connection::Online),
        Event::Connect(Connection::Connecting),
        Event::Session {
            id: "hub-session".into(),
            agent_id: None,
        },
        Event::Tick,
        Event::HelperReady,
        Event::PermissionObserved {
            microphone: orbit_core::PermissionState::Granted,
            screen: orbit_core::PermissionState::Unknown,
            accessibility: orbit_core::PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        Event::Completed {
            request_id: "remote".into(),
            text: Some("rearma".into()),
        },
    ] {
        let next = decide(&recovered, event.clone(), 10).unwrap().state;
        assert!(
            next.privacy_stopped(),
            "{event:?} must not rearm the user's stop"
        );
    }
    let rearmed = decide(&recovered, Event::LocalRearm, 20).unwrap().state;
    assert!(
        !rearmed.privacy_stopped(),
        "the user's local rearm lifts it"
    );
    // And after their rearm, the next restart has nothing to keep.
    assert!(!rearmed.durable().recovered().privacy_stopped());
}

#[test]
fn a_stop_saved_before_this_version_is_kept_because_nobody_can_say_whose() {
    // A snapshot written by an older daemon says it was stopped and not who
    // asked. Absence is not a "no": the stop is kept once more.
    let mut legacy = serde_json::to_value(DurableState::default()).unwrap();
    legacy["privacy_stopped"] = serde_json::json!(true);
    legacy.as_object_mut().unwrap().remove("privacy_stop_by");
    let legacy: DurableState = serde_json::from_value(legacy).unwrap();
    let recovered = legacy.recovered();
    assert!(recovered.privacy_stopped());
    assert!(
        said(&recovered).contains(&Notice {
            sequence: Some(1),
            ..Notice::new(NoticeCode::PrivacyStopKept).with(NoticeKey::Attributed, false)
        }),
        "a stop that is kept, unattributed, says so: {:?}",
        said(&recovered)
    );
}
