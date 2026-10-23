//! «Terminar sin guardar» through `decide`, the core's one entry point.
#![allow(clippy::unwrap_used)]
use crate::model::*;
use crate::notice::NoticeCode;
use crate::protocol::MeetingAudio;
use crate::refusal::{Expected, Parameter, Refusal};
use crate::{Effect, Event, decide};

const ID: &str = "4f1c7a2e-9b3d-4e8f-a6c5-2d1e0f9a8b7c";

fn recording(phase: CapturePhase) -> Recording {
    Recording {
        id: ID.into(),
        title: "Revisión de Lumen".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 1_000_000,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase,
        sealed_chunks: 4,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
    }
}

fn state(phase: CapturePhase) -> State {
    let mut state = State::default();
    state.durable.capabilities.native_helper = true;
    state.durable.capabilities.microphone_state = PermissionState::Granted;
    state.durable.recording = Some(recording(phase));
    state
}

fn said(state: &State) -> Vec<NoticeCode> {
    state
        .durable
        .notice_log
        .records()
        .iter()
        .map(|notice| notice.code)
        .collect()
}

fn discard(id: &str) -> Event {
    Event::DiscardMeeting {
        recording_id: id.into(),
    }
}

#[test]
fn a_recording_or_paused_meeting_drains_and_then_is_gone_without_being_saved() {
    for phase in [CapturePhase::Recording, CapturePhase::Paused] {
        let asked = decide(&state(phase.clone()), discard(ID), 5_000).unwrap();
        assert!(asked.effects.contains(&Effect::StopCapture), "{phase:?}");
        let draining = asked.state.durable.recording.as_ref().unwrap();
        assert_eq!(draining.phase, CapturePhase::Finalizing, "{phase:?}");
        assert_eq!(asked.state.durable.discarding(), Some(ID));
        let drained = decide(&asked.state, Event::CaptureFinalized, 6_000).unwrap();
        assert_eq!(drained.state.durable.recording, None, "{phase:?}");
        assert_eq!(drained.state.durable.discarding(), None);
        assert_eq!(
            drained.state.notice_code(),
            Some(NoticeCode::RecordingDiscarded)
        );
        // «Grabación guardada» is never said of it.
        assert!(
            !said(&drained.state).contains(&NoticeCode::RecordingFinished),
            "{:?}",
            said(&drained.state)
        );
    }
}

#[test]
fn only_the_named_recording_while_it_records_or_pauses_is_thrown_away() {
    let refused = Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::OpenRecording,
    };
    let other = decide(&state(CapturePhase::Recording), discard("otra"), 5_000);
    assert_eq!(other.unwrap_err(), refused);
    for phase in [
        CapturePhase::Starting,
        CapturePhase::Finalizing,
        CapturePhase::Idle,
        CapturePhase::Interrupted,
    ] {
        let refusal = decide(&state(phase.clone()), discard(ID), 5_000).unwrap_err();
        assert_eq!(refusal, refused, "{phase:?}");
    }
    let nothing = decide(&State::default(), discard(ID), 5_000).unwrap_err();
    assert_eq!(nothing, refused);
}

#[test]
fn under_a_privacy_stop_nothing_drains_and_it_is_gone_at_once() {
    let stopped = crate::authority_fixture::stopped_state(state(CapturePhase::Paused));
    let decision = decide(&stopped, discard(ID), 5_000).unwrap();
    assert_eq!(decision.state.durable.recording, None);
    assert!(!decision.effects.contains(&Effect::StopCapture));
    assert_eq!(
        decision.state.notice_code(),
        Some(NoticeCode::RecordingDiscarded)
    );
}

#[test]
fn a_drain_that_never_confirms_does_not_close_it_as_a_meeting() {
    let asked = decide(&state(CapturePhase::Recording), discard(ID), 5_000).unwrap();
    let late = 5_000 + crate::FINALIZING_TIMEOUT_MS + 1;
    let decision = decide(&asked.state, Event::Tick, late).unwrap();
    assert_eq!(decision.state.durable.recording, None);
    assert_eq!(
        decision.state.notice_code(),
        Some(NoticeCode::RecordingDiscarded)
    );
    assert!(!said(&decision.state).contains(&NoticeCode::RecordingCloseUnconfirmed));
}

#[test]
fn a_cut_while_it_drains_never_leaves_it_to_resume() {
    let asked = decide(&state(CapturePhase::Recording), discard(ID), 5_000).unwrap();
    let cut = decide(
        &asked.state,
        Event::CaptureCut {
            cause: "helper_lost".into(),
        },
        5_500,
    )
    .unwrap();
    assert_eq!(cut.state.durable.recording, None);
    assert_eq!(cut.state.durable.discarding(), None);
}

#[test]
fn a_restart_while_it_drains_forgets_it() {
    let asked = decide(&state(CapturePhase::Recording), discard(ID), 5_000).unwrap();
    let restored = asked.state.recovered();
    assert_eq!(restored.durable.recording, None);
    assert_eq!(restored.durable.discarding(), None);
}

#[test]
fn a_finished_meeting_still_closes_as_before() {
    let asked = decide(&state(CapturePhase::Recording), Event::FinishMeeting, 5_000).unwrap();
    let closed = decide(&asked.state, Event::CaptureFinalized, 6_000).unwrap();
    let recording = closed.state.durable.recording.as_ref().unwrap();
    assert_eq!(recording.phase, CapturePhase::Idle);
    assert_eq!(recording.ended_at_ms, Some(6_000));
    assert_eq!(
        closed.state.notice_code(),
        Some(NoticeCode::RecordingFinished)
    );
}
