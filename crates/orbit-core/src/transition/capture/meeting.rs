//! A meeting's recording from its start to its close.
use super::{CaptureState, discard, recording_in, wrong_state};
use crate::model::{CapturePhase, Recording, TrackProgress};
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::protocol::MeetingAudio;
use crate::refusal::{Expected, Operation, Parameter, Refusal};
use crate::transition::{Effect, Step, helper};

/// A recording starts without a duration bound. Only a finish or cut stops it.
pub(in crate::transition) fn start(
    capture: &mut CaptureState,
    id: String,
    title: String,
    audio: MeetingAudio,
    cx: &mut Step,
) -> Result<(), Refusal> {
    if cx.before.voice.in_call() {
        return Err(busy(Operation::AudioSession));
    }
    helper::ensure_microphone(cx.before)?;
    if capture
        .recording
        .as_ref()
        .is_some_and(|r| !matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted))
    {
        return Err(busy(Operation::Recording));
    }
    let application_pid = match audio {
        MeetingAudio::Application { application_pid } if application_pid > 0 => {
            Some(application_pid)
        }
        MeetingAudio::Application { .. } => {
            return Err(Refusal::InvalidParameter {
                parameter: Parameter::Meeting,
                expected: Expected::ValidMeeting,
            });
        }
        MeetingAudio::System | MeetingAudio::MicrophoneOnly => None,
    };
    if !matches!(audio, MeetingAudio::MicrophoneOnly) {
        helper::ensure_screen(cx.before)?;
    }
    let now_ms = cx.now_ms;
    capture.recording = Some(Recording {
        id: id.clone(),
        title,
        started_at_ms: now_ms,
        ended_at_ms: None,
        expires_at_ms: i64::MAX,
        application_pid,
        audio,
        phase: CapturePhase::Starting,
        sealed_chunks: 0,
        gaps: 0,
        tracks: tracks(audio),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    });
    cx.effect(Effect::StartCapture {
        recording_id: id,
        audio,
    });
    cx.say(NoticeCode::CaptureStarting);
    Ok(())
}

/// The microphone's track, and the meeting's other side unless only the
/// microphone is recorded.
fn tracks(audio: MeetingAudio) -> Vec<TrackProgress> {
    let track = |name: &str| TrackProgress {
        name: name.into(),
        ..TrackProgress::default()
    };
    match audio {
        MeetingAudio::MicrophoneOnly => vec![track("mic_local")],
        MeetingAudio::System | MeetingAudio::Application { .. } => {
            vec![track("mic_local"), track("remote_app")]
        }
    }
}

pub(in crate::transition) fn started(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let recording = in_phase(capture, CapturePhase::Starting)?;
    recording.phase = CapturePhase::Recording;
    cx.say(NoticeCode::RecordingStarted);
    Ok(())
}

pub(in crate::transition) fn pause(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let recording = in_phase(capture, CapturePhase::Recording)?;
    recording.phase = CapturePhase::Paused;
    cx.effect(Effect::StopCapture);
    cx.say(NoticeCode::RecordingPaused);
    Ok(())
}

pub(in crate::transition) fn resume(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    if cx.before.voice.in_call() {
        return Err(busy(Operation::AudioSession));
    }
    helper::ensure_microphone(cx.before)?;
    let recording = recording_in(capture)?;
    if recording.ended_at_ms.is_some() {
        return Err(not_open());
    }
    if !matches!(
        recording.phase,
        CapturePhase::Paused | CapturePhase::Interrupted
    ) {
        return Err(wrong_state());
    }
    recording.phase = CapturePhase::Starting;
    cx.effect(Effect::StartCapture {
        recording_id: recording.id.clone(),
        audio: recording.audio,
    });
    Ok(())
}

pub(in crate::transition) fn finish(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let recording = recording_in(capture)?;
    if recording.ended_at_ms.is_some() {
        return Err(not_open());
    }
    if matches!(recording.phase, CapturePhase::Finalizing) {
        return Err(wrong_state());
    }
    // An interrupted capture or one finalised under the privacy brake
    // has no producer left to drain, so we close the recording locally
    // instead of parking it in `Finalizing` waiting for an ACK that will
    // never come.
    if matches!(recording.phase, CapturePhase::Interrupted) || cx.before.privacy.stopped() {
        recording.phase = CapturePhase::Idle;
        recording.ended_at_ms = Some(cx.now_ms);
        recording.finalizing_since_ms = None;
        cx.say(NoticeCode::RecordingFinished);
    } else {
        recording.phase = CapturePhase::Finalizing;
        recording.finalizing_since_ms = Some(cx.now_ms);
        cx.effect(Effect::StopCapture);
        cx.say(NoticeCode::RecordingFinishing);
    }
    Ok(())
}

/// The helper confirmed the drain: the recording is closed, or, when it
/// was being thrown away, it is gone.
pub(in crate::transition) fn finalized(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    if discard::drained(capture, cx) {
        return Ok(());
    }
    let recording = in_phase(capture, CapturePhase::Finalizing)?;
    recording.phase = CapturePhase::Idle;
    recording.ended_at_ms = Some(cx.now_ms);
    recording.finalizing_since_ms = None;
    cx.say(NoticeCode::RecordingFinished);
    Ok(())
}

/// An interrupted recording with no fragment closes empty.
pub(in crate::transition) fn empty_closed(
    capture: &mut CaptureState,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let recording = recording_in(capture)?;
    if recording.phase != CapturePhase::Interrupted
        || recording.sealed_chunks != 0
        || recording.ended_at_ms.is_some()
    {
        return Err(wrong_state());
    }
    recording.ended_at_ms = Some(cx.now_ms.max(recording.started_at_ms));
    recording.finalizing_since_ms = None;
    cx.say(NoticeCode::RecordingEmpty);
    Ok(())
}

/// Closes a cut recording that will not resume. Its end is the end of what
/// was recorded, never before its start nor after now; the closure says the
/// cut's cause and how many fragments it keeps.
pub(in crate::transition) fn cut_closed(
    capture: &mut CaptureState,
    ended_at_ms: i64,
    cause: &str,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let now_ms = cx.now_ms;
    let recording = capture
        .recording
        .as_mut()
        .filter(|recording| {
            recording.phase == CapturePhase::Interrupted && recording.ended_at_ms.is_none()
        })
        .ok_or_else(wrong_state)?;
    recording.phase = CapturePhase::Idle;
    recording.ended_at_ms =
        Some(ended_at_ms.clamp(recording.started_at_ms, now_ms.max(recording.started_at_ms)));
    recording.finalizing_since_ms = None;
    cx.say(
        Notice::new(NoticeCode::RecordingClosedAfterCut)
            .with_token(NoticeKey::Cause, cause)
            .with(NoticeKey::Fragments, recording.sealed_chunks),
    );
    Ok(())
}

fn in_phase(capture: &mut CaptureState, phase: CapturePhase) -> Result<&mut Recording, Refusal> {
    let recording = recording_in(capture)?;
    if recording.phase == phase {
        Ok(recording)
    } else {
        Err(wrong_state())
    }
}

fn busy(operation: Operation) -> Refusal {
    Refusal::Busy {
        operation,
        retry_after_ms: None,
    }
}

fn not_open() -> Refusal {
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::OpenRecording,
    }
}
