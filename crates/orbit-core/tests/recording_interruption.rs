#![allow(clippy::unwrap_used)]
use orbit_core::notice::{DataValue, NoticeCode, NoticeKey, Token};
use orbit_core::{
    CapturePhase, Connection, Event, PermissionState, State, decide, protocol::MeetingAudio,
};

/// Every recording's safety ceiling, in milliseconds.
const CEILING_MS: i64 = orbit_core::RECORDING_CEILING_MINUTES as i64 * 60_000;

fn microphone_ready() -> State {
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let ready = decide(&online, Event::HelperReady, 0).unwrap().state;
    decide(
        &ready,
        Event::PermissionObserved {
            microphone: PermissionState::Granted,
            screen: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        0,
    )
    .unwrap()
    .state
}

#[test]
fn a_saved_interruption_with_a_future_cause_remains_readable() {
    let decoded = serde_json::from_value::<orbit_core::RecordingInterruption>(
        serde_json::json!({"cause":"future_capture_reason","audio_available":false}),
    );
    assert!(
        decoded.is_ok(),
        "a saved future cause could not be read: {decoded:?}"
    );
    assert_eq!(
        decoded.unwrap().cause,
        orbit_core::RecordingInterruptionCause::Unknown
    );
}

#[test]
fn pending_recording_refusal_names_identity_phase_and_numbers() {
    let mut state = microphone_ready();
    state.set_voice_processing_supported(true);
    let state = decide(
        &state,
        Event::StartMeeting {
            id: "pending-307".into(),
            title: "Reunión pendiente".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        1_000,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::ChunkSealed, 1_500).unwrap().state;
    let state = decide(&state, Event::ChunkSealed, 1_501).unwrap().state;
    let mut state = decide(&state, Event::CaptureGap, 1_502).unwrap().state;
    state.durable.recording.as_mut().unwrap().phase = CapturePhase::Interrupted;
    let refusal = decide(
        &state,
        Event::CallStart {
            id: "call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        },
        2_000,
    )
    .unwrap_err();
    let encoded = serde_json::to_value(&refusal).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({
            "code":"pending_recording", "id":"pending-307", "title":"Reunión pendiente", "phase":"interrupted",
            "sealed_chunks":2, "gaps":1, "expires_at_ms":1_000 + CEILING_MS, "retry":{"kind":"after_fix"}
        })
    );
}

#[test]
fn empty_capture_closure_keeps_the_capture_cut_cause_in_notice_history() {
    let state = microphone_ready();
    let state = decide(
        &state,
        Event::StartMeeting {
            id: "empty-cut".into(),
            title: "Corte de micrófono".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        1_000,
    )
    .unwrap()
    .state;
    let state = decide(
        &state,
        Event::CaptureCut {
            cause: "Se desconectó el micrófono.".into(),
        },
        2_000,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::EmptyCaptureClosed, 2_001)
        .unwrap()
        .state;
    assert_eq!(
        state.durable.recording.as_ref().unwrap().ended_at_ms,
        Some(2_001)
    );
    // The cut is said with what it cut, then the empty close.
    let codes: Vec<NoticeCode> = state
        .durable
        .notice_log
        .records()
        .iter()
        .map(|n| n.code)
        .collect();
    assert_eq!(
        &codes[codes.len() - 2..],
        [NoticeCode::CaptureCut, NoticeCode::RecordingEmpty]
    );
    let cut = &state.durable.notice_log.records()[codes.len() - 2];
    assert_eq!(
        cut.data.get(&NoticeKey::Recording),
        Some(&DataValue::Bool(true))
    );
    assert_eq!(
        cut.data.get(&NoticeKey::Fragments),
        Some(&DataValue::Int(0))
    );
    assert_eq!(
        state.durable.notice_log.latest(),
        state.durable.notice.as_ref()
    );
}

fn recording_for_tick_copy() -> State {
    let state = microphone_ready();
    let started = decide(
        &state,
        Event::StartMeeting {
            id: "tick-copy".into(),
            title: "Tick copy".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        1_000,
    )
    .unwrap()
    .state;
    decide(&started, Event::CaptureStarted, 1_500)
        .unwrap()
        .state
}

#[test]
fn expiry_names_saved_fragments_only_when_there_are_any() {
    let recording = recording_for_tick_copy();
    for has_chunk in [false, true] {
        let recording = if has_chunk {
            decide(&recording, Event::ChunkSealed, 1_600).unwrap().state
        } else {
            recording.clone()
        };
        let expired = decide(&recording, Event::Tick, 1_000 + CEILING_MS)
            .unwrap()
            .state;
        let notice = expired.durable.notice.clone().unwrap();
        assert_eq!(notice.code, NoticeCode::RecordingCeilingReached);
        assert_eq!(
            notice.data.get(&NoticeKey::Fragments),
            Some(&DataValue::Bool(has_chunk)),
            "expiry with has_chunk={has_chunk}: {notice:?}"
        );
        if !has_chunk {
            let closed = decide(&expired, Event::EmptyCaptureClosed, 1_001 + CEILING_MS)
                .unwrap()
                .state;
            assert_eq!(closed.notice_code(), Some(NoticeCode::RecordingEmpty));
        }
    }
}

#[test]
fn finalize_deadline_names_saved_fragments_only_when_there_are_any() {
    let recording = recording_for_tick_copy();
    for has_chunk in [false, true] {
        let recording = if has_chunk {
            decide(&recording, Event::ChunkSealed, 1_600).unwrap().state
        } else {
            recording.clone()
        };
        let finalizing = decide(&recording, Event::FinishMeeting, 2_000)
            .unwrap()
            .state;
        let timed_out = decide(
            &finalizing,
            Event::Tick,
            2_000 + orbit_core::FINALIZING_TIMEOUT_MS + 1,
        )
        .unwrap()
        .state;
        let notice = timed_out.durable.notice.clone().unwrap();
        assert_eq!(notice.code, NoticeCode::RecordingCloseUnconfirmed);
        assert_eq!(
            notice.data.get(&NoticeKey::Fragments),
            Some(&DataValue::Bool(has_chunk)),
            "finalize deadline with has_chunk={has_chunk}: {notice:?}"
        );
        if !has_chunk {
            let closed = decide(&timed_out, Event::EmptyCaptureClosed, 20_000)
                .unwrap()
                .state;
            assert_eq!(closed.notice_code(), Some(NoticeCode::RecordingEmpty));
        }
    }
}

#[test]
fn refused_voice_start_ends_the_voice_without_naming_a_recording() {
    let state = microphone_ready();
    let voice = decide(&state, Event::VoiceStart, 1_000).unwrap().state;
    let refused = decide(
        &voice,
        Event::CaptureRefused {
            reason: "microphone_permission_required".into(),
        },
        1_001,
    )
    .unwrap()
    .state;
    assert!(!refused.durable.voice_active);
    // A missing permission is the fixable notice, whether for a voice note
    // or a recording; it names no recording.
    assert_eq!(
        refused.notice_code(),
        Some(NoticeCode::MicrophonePermissionMissing)
    );
    let other = decide(
        &voice,
        Event::CaptureRefused {
            reason: "audio_engine_failed".into(),
        },
        1_001,
    )
    .unwrap()
    .state;
    let notice = other.durable.notice.unwrap();
    assert_eq!(notice.code, NoticeCode::VoiceNoteRefused);
    assert_eq!(
        notice.data.get(&NoticeKey::Cause),
        Some(&DataValue::Token(
            Token::new("audio_engine_failed").unwrap()
        ))
    );
}

/// Only the person, Orbit closing and the Mac going to sleep end a capture;
/// what the Mac or Orbit suffered leaves the meeting going on.
#[test]
fn only_the_person_closing_and_sleep_end_a_cut_recording() {
    use orbit_core::privacy_stop::StopCause;
    for cause in [
        StopCause::HelperLost,
        StopCause::CaptureEventFailed,
        StopCause::CaptureMaintenanceFailed,
        StopCause::HelperIntegrityFailure,
        StopCause::AudioStopUnconfirmed,
        StopCause::CaptureStopUndelivered,
    ] {
        assert!(cause.resumes_cut_recording(cause.name()), "{cause:?}");
    }
    assert!(StopCause::HelperCaptureStop.resumes_cut_recording("daemon_heartbeat_expired"));
    assert!(!StopCause::HelperCaptureStop.resumes_cut_recording("session_inactive"));
    for cause in [
        StopCause::Order,
        StopCause::Hotkey,
        StopCause::HelperUserStop,
        StopCause::HelperLatch,
        StopCause::Shutdown,
        StopCause::ListenerStopped,
    ] {
        assert!(!cause.resumes_cut_recording(cause.name()), "{cause:?}");
    }
}

/// A cut recording that will not resume is closed at the end of what was
/// recorded, never before its start nor after now, and says its cause.
#[test]
fn a_cut_recording_that_cannot_go_on_is_closed_at_its_last_audio() {
    let state = decide(
        &microphone_ready(),
        Event::StartMeeting {
            id: "reunion-cortada".into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        1_000,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::CaptureStarted, 1_100).unwrap().state;
    let state = decide(&state, Event::ChunkSealed, 2_000).unwrap().state;
    let cut = decide(
        &state,
        Event::CaptureCut {
            cause: "session_inactive".into(),
        },
        3_000,
    )
    .unwrap()
    .state;
    let closed = decide(
        &cut,
        Event::CutRecordingClosed {
            ended_at_ms: 2_500,
            cause: "session_inactive".into(),
        },
        9_000,
    )
    .unwrap()
    .state;
    let recording = closed.durable.recording.as_ref().unwrap();
    assert_eq!(recording.ended_at_ms, Some(2_500));
    assert_eq!(recording.phase, CapturePhase::Idle);
    let notice = closed.durable.notice.as_ref().unwrap();
    assert_eq!(notice.code, NoticeCode::RecordingClosedAfterCut);
    assert_eq!(
        notice.data.get(&NoticeKey::Cause),
        Some(&DataValue::Token(Token::new("session_inactive").unwrap()))
    );
    // A clock that ran backwards cannot end it before its start.
    let early = decide(
        &cut,
        Event::CutRecordingClosed {
            ended_at_ms: 10,
            cause: "helper_lost".into(),
        },
        9_000,
    )
    .unwrap()
    .state;
    assert_eq!(early.durable.recording.unwrap().ended_at_ms, Some(1_000));
    // Only an interrupted recording still open is closed this way.
    assert!(
        decide(
            &state,
            Event::CutRecordingClosed {
                ended_at_ms: 2_500,
                cause: "helper_lost".into(),
            },
            9_000,
        )
        .is_err()
    );
}
