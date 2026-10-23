//! «Terminar sin guardar»: a recording ends without becoming a meeting.
//!
//! While it records or is paused, the person can throw it away. Its capture
//! stops as a «Terminar» stops it, so the helper's last frames still drain,
//! but nothing of it is closed, uploaded or written up: the daemon drops
//! what drains and deletes what it had kept (`daemon/discard.rs`). Once the
//! drain ends, or cannot end, the recording leaves the state and the person
//! is told «Reunión descartada».
use super::Effect;
use crate::model::*;
use crate::notice::NoticeCode;
use crate::refusal::{Expected, Parameter, Refusal};

/// The recording `recording_id`, recording or paused, is thrown away.
/// Under a privacy stop nothing is left to drain: it is gone at once.
pub(super) fn decide_discard(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    recording_id: &str,
    now_ms: i64,
) -> Result<(), Refusal> {
    let recording = next
        .durable
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
    if state.durable.privacy_stopped {
        next.durable.recording = None;
        next.set_notice(NoticeCode::RecordingDiscarded);
        return Ok(());
    }
    recording.phase = CapturePhase::Finalizing;
    recording.finalizing_since_ms = Some(now_ms);
    next.durable.discarding = Some(recording_id.to_owned());
    effects.push(Effect::StopCapture);
    Ok(())
}

/// The capture of the recording being thrown away is over, drained or
/// given up on: the recording leaves the state, and that is said. False
/// when nothing is being thrown away, so the caller goes on as before.
pub(super) fn drained(next: &mut State) -> bool {
    let Some(discarded) = next.durable.discarding.take() else {
        return false;
    };
    if next
        .durable
        .recording
        .as_ref()
        .is_some_and(|recording| recording.id == discarded)
    {
        next.durable.recording = None;
    }
    next.set_notice(NoticeCode::RecordingDiscarded);
    true
}

/// The wait for the drain of a recording being thrown away is over: it has
/// nothing to close, so it is gone, never «not confirmed».
pub(super) fn drain_timed_out(next: &mut State, now_ms: i64) {
    let timed_out = next.durable.recording.as_ref().is_some_and(|recording| {
        recording.phase == CapturePhase::Finalizing
            && recording
                .finalizing_since_ms
                .is_some_and(|since| now_ms > since.saturating_add(crate::FINALIZING_TIMEOUT_MS))
    });
    if timed_out {
        drained(next);
    }
}

/// After every decision: a recording being thrown away that something else
/// cut or replaced (a privacy stop, a revocation, a close) is gone too; it
/// never resumes or closes as a meeting.
pub(super) fn settle(next: &mut State) {
    let Some(discarded) = next.durable.discarding.as_deref() else {
        return;
    };
    let draining = next.durable.recording.as_ref().is_some_and(|recording| {
        recording.id == discarded && recording.phase == CapturePhase::Finalizing
    });
    if draining {
        return;
    }
    if next
        .durable
        .recording
        .as_ref()
        .is_some_and(|recording| recording.id == discarded)
    {
        next.durable.recording = None;
    }
    next.durable.discarding = None;
}

#[cfg(test)]
#[path = "discard_tests.rs"]
mod tests;
