//! The recording and its capture: a meeting starts, pauses, resumes,
//! finishes or is thrown away; its fragments and gaps are counted; a cut
//! interrupts it and the clock bounds it.
//!
//! Reads of other domains: a call holds the microphone (`voice_call`), the
//! microphone and screen guards (`helper`, which reads the privacy stop),
//! and the privacy stop, under which nothing is left to drain.
use super::Step;
use crate::failure::DegradedKind;
use crate::model::{CapturePhase, Recording};
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::refusal::{Expected, Parameter, Refusal};

mod closing;
mod discard;
mod meeting;

pub(super) use closing::tick_close;
pub(super) use discard::{discard, drain_timed_out, settle};
pub(super) use meeting::{
    cut_closed, empty_closed, finalized, finish, pause, resume, start, started,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureState {
    pub recording: Option<Recording>,
    /// «Terminar sin guardar»: the recording whose capture is draining to
    /// be thrown away, never closed, uploaded or written up
    /// (`capture/discard.rs`). Absent in every other state.
    pub(crate) discarding: Option<String>,
}

impl CaptureState {
    /// The recording being thrown away while its capture drains.
    pub fn discarding(&self) -> Option<&str> {
        self.discarding.as_deref()
    }

    /// The recording still open: not closed, and not idle.
    pub(super) fn open_recording(&self) -> Option<&Recording> {
        self.recording.as_ref().filter(|recording| {
            recording.ended_at_ms.is_none() && recording.phase != CapturePhase::Idle
        })
    }

    /// Whether a capture is still running for the recording, as a restart
    /// would find it.
    pub(super) fn capturing(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|recording| recording.phase.interrupted_on_recovery())
    }

    /// A restart never restarts the capture: a recording being thrown away
    /// is gone, and one that was live is left interrupted, with a gap.
    pub(crate) fn recovered(&mut self) -> Option<Notice> {
        // A recording being thrown away is gone: it never resumes or closes.
        if let Some(discarded) = self.discarding.take()
            && self
                .recording
                .as_ref()
                .is_some_and(|recording| recording.id == discarded)
        {
            self.recording = None;
        }
        let recording = self
            .recording
            .as_mut()
            .filter(|recording| recording.phase.interrupted_on_recovery())?;
        recording.phase = CapturePhase::Interrupted;
        recording.gaps += 1;
        Some(
            Notice::new(NoticeCode::RecordingInterruptedByRestart)
                .with(NoticeKey::Fragments, recording.sealed_chunks),
        )
    }
}

/// The capture is interrupted where it was live: starting, recording or
/// closing. The fragments it keeps, when it was.
pub(super) fn interrupt(capture: &mut CaptureState) -> Option<u64> {
    let recording = capture.recording.as_mut().filter(|recording| {
        matches!(
            recording.phase,
            CapturePhase::Recording | CapturePhase::Starting | CapturePhase::Finalizing
        )
    })?;
    recording.phase = CapturePhase::Interrupted;
    Some(recording.sealed_chunks)
}

pub(super) fn chunk_sealed(capture: &mut CaptureState) {
    if let Some(recording) = capture.recording.as_mut() {
        recording.sealed_chunks += 1;
    }
}

pub(super) fn gap(capture: &mut CaptureState, cx: &mut Step) {
    if let Some(recording) = capture.recording.as_mut() {
        recording.gaps = recording.gaps.saturating_add(1);
    }
    cx.say(NoticeCode::AudioGap);
}

pub(super) fn degraded(capture: &mut CaptureState, kind: DegradedKind, cx: &mut Step) {
    if matches!(kind, DegradedKind::AudioGap) {
        gap(capture, cx);
    }
}

/// The open recording, or a refusal naming the state it needed.
fn recording_in(capture: &mut CaptureState) -> Result<&mut Recording, Refusal> {
    capture.recording.as_mut().ok_or_else(wrong_state)
}

fn wrong_state() -> Refusal {
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::CorrectCaptureState,
    }
}
