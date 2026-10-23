//! «Terminar sin guardar»: a recording ends without becoming a meeting.
//!
//! While it records or is paused, the person can throw it away. Its capture
//! stops as a «Terminar» stops it, so the helper's last frames still drain,
//! but nothing of it is closed, uploaded or written up: the daemon drops
//! what drains and deletes what it had kept (`daemon/discard.rs`). Once the
//! drain ends, or cannot end, the recording leaves the state and the person
//! is told «Reunión descartada».
use super::CaptureState;
use crate::model::CapturePhase;
use crate::notice::NoticeCode;
use crate::refusal::{Expected, Parameter, Refusal};
use crate::transition::{Effect, Step};

/// The recording `recording_id`, recording or paused, is thrown away.
/// Under a privacy stop nothing is left to drain: it is gone at once.
pub(in crate::transition) fn discard(
    capture: &mut CaptureState,
    recording_id: &str,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let recording = capture
        .recording
        .as_mut()
        .filter(|recording| {
            recording.id == recording_id
                && recording.ended_at_ms.is_none()
                && matches!(
                    recording.phase,
                    CapturePhase::Recording | CapturePhase::Paused
                )
        })
        .ok_or(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::OpenRecording,
        })?;
    if cx.before.privacy.stopped() {
        capture.recording = None;
        cx.say(NoticeCode::RecordingDiscarded);
        return Ok(());
    }
    recording.phase = CapturePhase::Finalizing;
    recording.finalizing_since_ms = Some(cx.now_ms);
    capture.discarding = Some(recording_id.to_owned());
    cx.effect(Effect::StopCapture);
    Ok(())
}

/// The capture of the recording being thrown away is over, drained or
/// given up on: the recording leaves the state, and that is said. False
/// when nothing is being thrown away, so the caller goes on as before.
pub(super) fn drained(capture: &mut CaptureState, cx: &mut Step) -> bool {
    let Some(discarded) = capture.discarding.take() else {
        return false;
    };
    if capture
        .recording
        .as_ref()
        .is_some_and(|recording| recording.id == discarded)
    {
        capture.recording = None;
    }
    cx.say(NoticeCode::RecordingDiscarded);
    true
}

/// The wait for the drain of a recording being thrown away is over: it has
/// nothing to close, so it is gone, never «not confirmed».
pub(in crate::transition) fn drain_timed_out(capture: &mut CaptureState, cx: &mut Step) {
    let now_ms = cx.now_ms;
    let timed_out = capture.recording.as_ref().is_some_and(|recording| {
        recording.phase == CapturePhase::Finalizing
            && recording
                .finalizing_since_ms
                .is_some_and(|since| now_ms > since.saturating_add(crate::FINALIZING_TIMEOUT_MS))
    });
    if timed_out {
        drained(capture, cx);
    }
}

/// After every decision: a recording being thrown away that something else
/// cut or replaced (a privacy stop, a revocation, a close) is gone too; it
/// never resumes or closes as a meeting.
pub(in crate::transition) fn settle(capture: &mut CaptureState) {
    let Some(discarded) = capture.discarding.as_deref() else {
        return;
    };
    let draining = capture.recording.as_ref().is_some_and(|recording| {
        recording.id == discarded && recording.phase == CapturePhase::Finalizing
    });
    if draining {
        return;
    }
    if capture
        .recording
        .as_ref()
        .is_some_and(|recording| recording.id == discarded)
    {
        capture.recording = None;
    }
    capture.discarding = None;
}

#[cfg(test)]
#[path = "discard_tests.rs"]
mod tests;
