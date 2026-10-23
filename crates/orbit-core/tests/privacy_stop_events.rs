//! The privacy stop in the pure core: which events may latch the stop, who
//! asked for it, and what every former automatic stop does now.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use orbit_core::{
    Approval, CallPhase, CapturePhase, Connection, Effect, Event, State, decide,
    failure::DegradedKind,
    privacy_stop::{StopCause, StopClass, StopRequester, latches_helper},
    protocol::MeetingAudio,
};

/// The variant of every event, by an exhaustive match: a new event does not
/// compile until someone says whether it may latch the stop. The index is how
/// the test below proves its samples cover every variant.
fn classify(event: &Event) -> (usize, bool) {
    match event {
        Event::Connect(connection) => (0, *connection == Connection::Revoked),
        Event::Connecting => (40, false),
        Event::Submit { .. } => (1, false),
        Event::SubmitVoice { .. } => (2, false),
        Event::VoiceStored { .. } => (3, false),
        Event::VoiceFailed { .. } => (39, false),
        Event::Session { .. } => (4, false),
        Event::HubTransportReady => (38, false),
        Event::HubDisconnected => (37, false),
        Event::NewSession { .. } => (34, false),
        Event::OpenSession { .. } => (35, false),
        Event::DeviceChanged => (36, false),
        Event::Delta { .. } => (5, false),
        Event::Completed { .. } => (6, false),
        Event::Approval { .. } => (7, false),
        Event::Interrupt => (8, false),
        Event::StartMeeting { .. } => (9, false),
        Event::CaptureStarted => (10, false),
        Event::CaptureFinalized => (11, false),
        Event::PauseMeeting => (12, false),
        Event::ResumeMeeting => (13, false),
        Event::FinishMeeting => (14, false),
        Event::ChunkSealed => (15, false),
        Event::CaptureGap => (16, false),
        Event::VoiceStart => (17, false),
        Event::VoiceEnd => (18, false),
        Event::CallStart { .. } => (19, false),
        Event::CallStop => (20, false),
        Event::CallReady => (21, false),
        Event::CallPhase(_) => (22, false),
        Event::CallTurnStarted => (23, false),
        // The user's stop, and the helper's record of one.
        Event::PrivacyStop => (24, true),
        Event::HelperLatched => (25, true),
        Event::LocalRearm => (26, false),
        Event::HelperReady => (41, false),
        Event::HelperLost => (42, false),
        Event::PermissionObserved { .. } => (43, false),
        Event::CalendarAuthorization(_) => (44, false),
        Event::HelperStopUndelivered => (45, false),
        Event::Tick => (28, false),
        Event::Failure { .. } => (29, false),
        Event::Degraded(_) => (30, false),
        Event::CaptureRefused { .. } => (31, false),
        // The two that replace automatic stops: they cut, they never latch.
        Event::CaptureCut { .. } => (32, false),
        Event::Close { .. } => (33, false),
        Event::EmptyCaptureClosed => (27, false),
        Event::CutRecordingClosed { .. } => (49, false),
        Event::VoiceCancel => (46, false),
        Event::VoiceRetry { .. } => (47, false),
        Event::VoiceTooShort(_) => (48, false),
        Event::VoiceDiscard { .. } => (50, false),
        Event::VoiceUnheard { .. } => (51, false),
        // «Terminar sin guardar» stops the capture; it never latches.
        Event::DiscardMeeting { .. } => (52, false),
    }
}
const VARIANTS: usize = 53;

#[test]
fn voice_transcription_retires_with_authority_but_survives_hub_link_changes() {
    use orbit_core::authority::{AuthorityChange, RetireSlot, retire};
    for change in [
        AuthorityChange::UserStop,
        AuthorityChange::Interrupt,
        AuthorityChange::Closing,
        AuthorityChange::Conversation,
        AuthorityChange::Device,
        AuthorityChange::Revoked,
    ] {
        assert!(
            retire(change)
                .slots
                .contains(&RetireSlot::VoiceTranscription),
            "{change:?}"
        );
    }
    for change in [
        AuthorityChange::HubLink,
        AuthorityChange::HubLinkOffline,
        AuthorityChange::HubDisconnected,
    ] {
        assert!(
            !retire(change)
                .slots
                .contains(&RetireSlot::VoiceTranscription),
            "{change:?}"
        );
    }
}

fn samples() -> Vec<Event> {
    vec![
        Event::Connecting,
        Event::HelperReady,
        Event::HelperLost,
        Event::PermissionObserved {
            microphone: orbit_core::PermissionState::Granted,
            screen: orbit_core::PermissionState::Granted,
            accessibility: orbit_core::PermissionState::Granted,
            calendar: orbit_core::calendar::CalendarAuthorization::FullAccess,
        },
        Event::CalendarAuthorization(orbit_core::calendar::CalendarAuthorization::FullAccess),
        Event::HelperStopUndelivered,
        Event::Connect(Connection::Online),
        Event::Connect(Connection::Revoked),
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        Event::SubmitVoice {
            id: "v".into(),
            text: "hola".into(),
            duration_ms: 1_000,
        },
        Event::VoiceStored {
            id: "v".into(),
            duration_ms: 1_000,
        },
        Event::VoiceFailed {
            id: "v".into(),
            failure: orbit_core::VoiceTranscriptionFailure {
                code: "transcription_timeout".into(),
                provider_status: None,
                hub_status: None,
                timeout_ms: Some(90_000),
                bytes: None,
                max_bytes: None,
                attempts: Some(1),
            },
        },
        Event::Session {
            id: "s".into(),
            agent_id: None,
        },
        Event::HubTransportReady,
        Event::HubDisconnected,
        Event::NewSession { agent_id: None },
        Event::OpenSession {
            id: "saved".into(),
            agent_id: None,
        },
        Event::DeviceChanged,
        Event::Delta {
            request_id: "a".into(),
            text: "x".into(),
        },
        Event::Completed {
            request_id: "a".into(),
            text: Some("x".into()),
        },
        Event::Approval {
            turn_id: "a".into(),
            approval: Approval {
                request_id: "a".into(),
                description: "d".into(),
                clarification: false,
                choices: Vec::new(),
                questions: Vec::new(),
                deletes: None,
            },
        },
        Event::Interrupt,
        Event::StartMeeting {
            id: "m".into(),
            title: "Reunión".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        Event::CaptureStarted,
        Event::CaptureFinalized,
        Event::PauseMeeting,
        Event::ResumeMeeting,
        Event::FinishMeeting,
        Event::DiscardMeeting {
            recording_id: "r".into(),
        },
        Event::ChunkSealed,
        Event::CaptureGap,
        Event::VoiceStart,
        Event::VoiceEnd,
        Event::VoiceCancel,
        Event::VoiceRetry { id: "v".into() },
        Event::VoiceTooShort(orbit_core::voice::ShortVoiceNote {
            duration_ms: 90,
            minimum_ms: 160,
        }),
        Event::VoiceDiscard { id: "v".into() },
        Event::VoiceUnheard { id: "v".into() },
        Event::CallStart {
            id: "c".into(),
            duration_ms: 600_000,
            origin: orbit_core::CallOrigin::App,
        },
        Event::CallStop,
        Event::CallReady,
        Event::CallPhase(CallPhase::Listening),
        Event::CallTurnStarted,
        Event::PrivacyStop,
        Event::HelperLatched,
        Event::LocalRearm,
        Event::Tick,
        Event::Failure {
            code: "x".into(),
            numbers: Vec::new(),
        },
        Event::Degraded(DegradedKind::AudioGap),
        Event::CaptureRefused {
            reason: "microphone_permission_required".into(),
        },
        Event::CaptureCut {
            cause: "Fallo sintético.".into(),
        },
        Event::Close {
            cause: "Cierre sintético.".into(),
        },
        Event::EmptyCaptureClosed,
        Event::CutRecordingClosed {
            ended_at_ms: 3,
            cause: "helper_lost".into(),
        },
    ]
}

fn live_recording() -> State {
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let ready = decide(&online, Event::HelperReady, 0).unwrap().state;
    let base = decide(
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
    let started = decide(
        &base,
        Event::StartMeeting {
            id: "live-recording".into(),
            title: "Reunión".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        0,
    )
    .unwrap()
    .state;
    let mut recording = decide(&started, Event::CaptureStarted, 1).unwrap().state;
    recording.durable.recording.as_mut().unwrap().sealed_chunks = 7;
    recording
}

#[test]
fn only_the_user_stop_the_helper_latch_and_a_revocation_ever_latch() {
    let mut covered = [false; VARIANTS];
    let mut variants: [Option<std::mem::Discriminant<Event>>; VARIANTS] = [None; VARIANTS];
    for event in samples() {
        let (index, may_latch) = classify(&event);
        let variant = std::mem::discriminant(&event);
        if let Some(previous) = variants[index] {
            assert_eq!(
                previous, variant,
                "variant index reused: {index} by {event:?}"
            );
        }
        variants[index] = Some(variant);
        covered[index] = true;
        for state in [State::default(), live_recording()] {
            let Ok(decision) = decide(&state, event.clone(), 5) else {
                continue;
            };
            assert_eq!(
                decision.state.privacy_stopped(),
                may_latch,
                "{event:?} latched={} but may_latch={may_latch}",
                decision.state.privacy_stopped()
            );
        }
    }
    let missing: Vec<usize> = (0..VARIANTS).filter(|index| !covered[*index]).collect();
    assert!(missing.is_empty(), "variants without a sample: {missing:?}");
}

#[test]
fn only_the_user_local_rearm_lifts_a_stop() {
    let stopped = decide(&State::default(), Event::PrivacyStop, 0)
        .unwrap()
        .state;
    for event in samples() {
        let Ok(decision) = decide(&stopped, event.clone(), 5) else {
            continue;
        };
        assert_eq!(
            decision.state.privacy_stopped(),
            !matches!(event, Event::LocalRearm),
            "{event:?}"
        );
    }
}

#[test]
fn the_stop_records_who_asked_for_it() {
    let by_user = decide(&State::default(), Event::PrivacyStop, 0)
        .unwrap()
        .state;
    assert_eq!(by_user.privacy_stop_by(), Some(StopRequester::User));
    assert!(latches_helper(&by_user));
    let adopted = decide(&State::default(), Event::HelperLatched, 0)
        .unwrap()
        .state;
    assert!(adopted.privacy_stopped());
    assert_eq!(
        adopted.privacy_stop_by(),
        None,
        "a latch found on disk cannot say who pressed it"
    );
    assert!(latches_helper(&adopted));
    let revoked = decide(&State::default(), Event::Connect(Connection::Revoked), 0)
        .unwrap()
        .state;
    assert!(revoked.privacy_stopped());
    assert_eq!(revoked.privacy_stop_by(), Some(StopRequester::Revocation));
    assert!(
        !latches_helper(&revoked),
        "a revocation never writes the helper's durable latch"
    );
    // The user's stop wins over a revocation that arrives later, and stays theirs.
    let both = decide(&by_user, Event::Connect(Connection::Revoked), 1)
        .unwrap()
        .state;
    assert_eq!(both.privacy_stop_by(), Some(StopRequester::User));
    // Rearming forgets who asked.
    let rearmed = decide(&by_user, Event::LocalRearm, 2).unwrap().state;
    assert_eq!(rearmed.privacy_stop_by(), None);
}

#[test]
fn a_revocation_is_asked_again_after_a_restart_not_carried() {
    let revoked = decide(&State::default(), Event::Connect(Connection::Revoked), 0)
        .unwrap()
        .state;
    let recovered = revoked.durable().recovered();
    assert!(!recovered.privacy_stopped());
    assert_eq!(recovered.privacy_stop_by(), None);
    let by_user = decide(&State::default(), Event::PrivacyStop, 0)
        .unwrap()
        .state;
    let recovered = by_user.durable().recovered();
    assert!(recovered.privacy_stopped());
    assert_eq!(recovered.privacy_stop_by(), Some(StopRequester::User));
    // The user's, and the interface's entry for it names the way out (Rearmar).
    assert_eq!(
        recovered.notice,
        Some(orbit_core::notice::Notice {
            sequence: Some(2),
            ..orbit_core::notice::Notice::new(orbit_core::notice::NoticeCode::PrivacyStopKept)
                .with(orbit_core::notice::NoticeKey::Attributed, true)
        }),
        "the user's kept stop says it is theirs: {:?}",
        recovered.notice
    );
}

#[test]
fn a_cut_interrupts_the_capture_says_why_and_latches_nothing() {
    let state = live_recording();
    let decision = decide(
        &state,
        Event::CaptureCut {
            cause: "El almacén local rechazó el fragmento 8.".into(),
        },
        10,
    )
    .unwrap();
    assert!(!decision.state.privacy_stopped());
    assert_eq!(decision.state.privacy_stop_by(), None);
    let recording = decision.state.durable.recording.as_ref().unwrap();
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert_eq!(recording.sealed_chunks, 7);
    assert_eq!(
        decision.state.generation(),
        state.generation(),
        "a cut is not a new authority: live hub work keeps its generation"
    );
    assert_eq!(decision.state.connection(), Connection::Online);
    assert!(decision.effects.contains(&Effect::CutCapture));
    assert!(
        !decision.effects.contains(&Effect::CancelNetwork),
        "Orbit keeps working"
    );
    assert!(
        !decision
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartCapture { .. })),
        "the microphone is not reopened"
    );
    // The cause sentence stays in the daemon's log; the notice says
    // what was cut and counts what was kept, and offers no rearm (its class
    // is a failure, not a stop of the user's).
    let notice = decision.state.durable.notice.clone().unwrap();
    assert_eq!(notice.code, orbit_core::notice::NoticeCode::CaptureCut);
    assert_eq!(
        notice.data.get(&orbit_core::notice::NoticeKey::Fragments),
        Some(&orbit_core::notice::DataValue::Int(7)),
        "the notice counts what was kept: {notice:?}"
    );
    assert_eq!(
        notice.data.get(&orbit_core::notice::NoticeKey::Recording),
        Some(&orbit_core::notice::DataValue::Bool(true))
    );
}

#[test]
fn closing_cuts_and_retires_network_without_latching() {
    let state = live_recording();
    let decision = decide(
        &state,
        Event::Close {
            cause: "Orbit se está cerrando.".into(),
        },
        10,
    )
    .unwrap();
    assert!(!decision.state.privacy_stopped());
    assert_eq!(
        decision.state.durable.recording.as_ref().unwrap().phase,
        CapturePhase::Interrupted
    );
    assert_eq!(decision.state.generation(), state.generation() + 1);
    for effect in [
        Effect::CutCapture,
        Effect::StopPlayback,
        Effect::CancelNetwork,
    ] {
        assert!(decision.effects.contains(&effect), "missing {effect:?}");
    }
    assert!(
        !decision.effects.contains(&Effect::StopCapture),
        "closing never asks the helper for a privacy stop"
    );
}

#[test]
fn every_former_automatic_stop_has_its_class() {
    // The same classification table, fixed here cause by cause.
    let expected = [
        (StopCause::Order, StopClass::UserStop),
        (StopCause::Hotkey, StopClass::UserStop),
        (StopCause::HelperUserStop, StopClass::UserStop),
        (StopCause::HelperLatch, StopClass::UserStop),
        (StopCause::HelperCaptureStop, StopClass::CutCapture),
        (StopCause::HelperIntegrityFailure, StopClass::CutCapture),
        (StopCause::HelperRefusedStopped, StopClass::Notice),
        (StopCause::HelperLost, StopClass::CutCapture),
        (StopCause::CaptureEventFailed, StopClass::CutCapture),
        // A fault of another vertical is said and never cuts a recording.
        (StopCause::NativeEventFailed, StopClass::Notice),
        (StopCause::CaptureMaintenanceFailed, StopClass::CutCapture),
        (StopCause::VerticalMaintenanceFailed, StopClass::Notice),
        (StopCause::AudioStopUnconfirmed, StopClass::CutCapture),
        (StopCause::CaptureStopUndelivered, StopClass::CutCapture),
        (StopCause::PrivacyStopUndelivered, StopClass::Notice),
        (StopCause::Shutdown, StopClass::CutCapture),
        (StopCause::ListenerStopped, StopClass::CutCapture),
        (StopCause::AudioInventoryUnverifiable, StopClass::Notice),
    ];
    assert_eq!(expected.len(), StopCause::ALL.len());
    for (cause, class) in expected {
        assert_eq!(cause.class(), class, "{cause:?}");
    }
}
