//! A meeting is not lost to a cut it did not ask for.
//!
//! Only the person («Terminar», the privacy stop), Orbit closing and the Mac
//! going to sleep end a capture. Any other cut (the helper lost or silent, a
//! stalled daemon, a failed frame, a daemon restart after a crash) leaves the
//! same recording interrupted, and once its audio has settled and the capture
//! is back it resumes on its own: a new segment of the same recording, the
//! missing time said with its duration. A cut recording that cannot resume is
//! closed at the end of what was recorded, so the hub writes its note, and the
//! closure is said with its cause. Nothing waits for anyone to press
//! «Terminar».
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::Refusal;
use std::time::Duration;
use tokio::time::Instant;

/// How long a resumable cut waits for the capture to come back (the helper
/// relaunched and its permissions observed) before the recording is closed.
/// The helper supervisor relaunches five times with 31 s of backoff in all,
/// plus each launch; two minutes covers that with room. Past it the
/// supervisor has given up and the capture is not coming back by itself. This
/// is not a limit on the meeting, only on a capture that is gone.
const CAPTURE_RETURN_WAIT: Duration = Duration::from_secs(120);

/// A resumed capture that runs this long is settled: a later cut is a new
/// one and starts its own count of automatic resumes. One minute is many
/// one-second chunks, so a capture that keeps failing right away (a refused
/// device, a broken helper) is told apart from one that worked for a while.
const RESUME_SETTLED_AFTER: Duration = Duration::from_secs(60);

/// Backoff protects a failing device/helper from a busy restart loop. It
/// limits retry frequency, never the number of attempts or meeting duration.
const RETRY_SECONDS: [u64; 5] = [1, 2, 5, 10, 30];
/// A short route rebuild should not alarm the person. Five seconds covers
/// those brief cuts; a longer cut is announced even if sealing is still owed.
const CUT_NOTICE_AFTER: Duration = Duration::from_secs(5);

/// What the daemon knows about the cut that left a recording interrupted.
pub(super) struct CutRecovery {
    recording_id: String,
    /// The cut's code: the helper's own, or the cause's name.
    cause: String,
    /// The cause lets the recording go on (see
    /// `StopCause::resumes_cut_recording`).
    resumable: bool,
    /// The end of what was recorded before the cut, for a closure.
    cut_at_ms: i64,
    /// Since when it has waited for the capture to come back.
    waiting_since: Instant,
    /// Automatic resumes in a row that did not settle.
    resumes: u32,
    resumed_at: Option<Instant>,
    next_try_at: Instant,
    attempts: usize,
    notified: bool,
    resume_announced: bool,
}

impl CutRecovery {
    pub(super) fn new(
        recording_id: &str,
        cause: &str,
        resumable: bool,
        cut_at_ms: i64,
        at: Instant,
    ) -> Self {
        Self {
            recording_id: recording_id.to_owned(),
            cause: cause.to_owned(),
            resumable,
            cut_at_ms,
            waiting_since: at,
            resumes: 0,
            resumed_at: None,
            next_try_at: at + Duration::from_secs(RETRY_SECONDS[0]),
            attempts: 0,
            notified: false,
            resume_announced: false,
        }
    }
}

/// The cut a new daemon finds in the recording the last one saved. One that
/// was live (a crash, or the daemon relaunched: a clean quit leaves it
/// interrupted) goes on once the new helper is ready; one «Terminar» was
/// finishing is closed, and so is any when this daemon has no helper to
/// capture with (a development daemon): nothing would bring it back.
/// Anything else was not cut by a restart.
pub(super) fn recovery_after_restart(
    saved: Option<&orbit_core::Recording>,
    helper: bool,
) -> Option<CutRecovery> {
    let recording = saved.filter(|recording| recording.phase.interrupted_on_recovery())?;
    Some(CutRecovery::new(
        &recording.id,
        "restart",
        helper && recording.phase != CapturePhase::Finalizing,
        last_audio_at_ms(recording),
        Instant::now(),
    ))
}

impl Runtime {
    /// A cut of `cause` (with `code`) is about to interrupt what is live:
    /// remember whether the recording goes on. A cut while «Terminar» was
    /// finishing it never resumes: the person asked for the end.
    pub(super) fn note_cut(&mut self, cause: orbit_core::privacy_stop::StopCause, code: &str) {
        let at = self.capture.recovery_now();
        let recording = self.state.durable.capture.recording.as_ref();
        let Some(recording) = recording.filter(|recording| {
            matches!(
                recording.phase,
                CapturePhase::Recording | CapturePhase::Starting | CapturePhase::Finalizing
            )
        }) else {
            return;
        };
        let resumable =
            recording.phase != CapturePhase::Finalizing && cause.resumes_cut_recording(code);
        let earlier = self
            .capture
            .recovery
            .as_ref()
            .filter(|earlier| earlier.recording_id == recording.id)
            .filter(|earlier| {
                earlier
                    .resumed_at
                    .is_some_and(|resumed| at.duration_since(resumed) < RESUME_SETTLED_AFTER)
            })
            .map(|earlier| (earlier.resumes, earlier.attempts));
        let mut recovery = CutRecovery::new(&recording.id, code, resumable, now(), at);
        if let Some((resumes, attempts)) = earlier {
            recovery.resumes = resumes;
            recovery.attempts = attempts;
            recovery.next_try_at = at + retry_wait(attempts);
        }
        self.capture.recovery = Some(recovery);
    }

    /// The capture's upkeep, once a second: resume a cut recording whose
    /// capture is back, or close one that cannot go on.
    pub(super) async fn recover_cut_recording(&mut self) -> Result<()> {
        let at = self.capture.recovery_now();
        let Some(recording) = self.state.durable.capture.recording.clone() else {
            self.capture.recovery = None;
            return Ok(());
        };
        if recording.phase != CapturePhase::Interrupted || recording.ended_at_ms.is_some() {
            // A resumed capture that ran long enough is settled; a closed
            // or replaced recording has nothing left to recover.
            let settled = self.capture.recovery.as_ref().is_some_and(|recovery| {
                recovery.recording_id != recording.id
                    || recording.ended_at_ms.is_some()
                    || recovery
                        .resumed_at
                        .is_some_and(|resumed| at.duration_since(resumed) >= RESUME_SETTLED_AFTER)
            });
            if settled {
                self.capture.recovery = None;
            }
            return Ok(());
        }
        self.announce_waiting_cut();
        if self.closing
            || self.audio.audio_is_draining()
            || self.stopped_audio_owed()
            || !self.capture.pending_seals.is_empty()
            || self.capture.buffer.has_unsealed_recording_audio()
            || self.audio.mic_stream.is_some()
            || self.audio.remote_stream.is_some()
            || !self.audio.pending_starts.is_empty()
        {
            // The cut is still settling its audio; nothing is decided
            // until what was recorded is sealed.
            return Ok(());
        }
        if recording.sealed_chunks == 0 && !self.cut_owns(&recording.id) {
            // Nothing recorded and no cut that goes on: the empty closure,
            // with its typed cause, owns it.
            return Ok(());
        }
        let mut recovery = match self.capture.recovery.take() {
            Some(recovery) if recovery.recording_id == recording.id => recovery,
            // A cut nobody attributed here (a «Terminar» whose drain never
            // came, a refused restart, the ceiling, a clean quit before this
            // start): it does not resume.
            _ => CutRecovery::new(
                &recording.id,
                &self.capture.unattributed_cut_cause(),
                false,
                last_audio_at_ms(&recording),
                self.capture.recovery_now(),
            ),
        };
        if self.cut_will_resume_with(&recovery) {
            let expired = at.duration_since(recovery.waiting_since) >= CAPTURE_RETURN_WAIT;
            if !expired && at < recovery.next_try_at {
                self.capture.recovery = Some(recovery);
                return Ok(());
            }
            if !expired {
                let result = self.try_resume(&recovery).await;
                recovery.attempts = recovery.attempts.saturating_add(1);
                recovery.next_try_at = self.capture.recovery_now() + retry_wait(recovery.attempts);
                match result {
                    Resume::Started => {
                        recovery.resumes = recovery.resumes.saturating_add(1);
                        recovery.resumed_at = Some(at);
                        self.capture.recovery = Some(recovery);
                        return Ok(());
                    }
                    Resume::Wait => {
                        self.capture.recovery = Some(recovery);
                        return Ok(());
                    }
                    Resume::Refused => {}
                }
            }
        }
        if recording.sealed_chunks == 0 {
            return self.close_empty_cut_recording(recovery).await;
        }
        self.close_cut_recording(recovery).await
    }

    fn announce_waiting_cut(&mut self) {
        let at = self.capture.recovery_now();
        let Some(recovery) = self.capture.recovery.as_mut() else {
            return;
        };
        if recovery.resumable
            && !recovery.notified
            && at.duration_since(recovery.waiting_since) >= CUT_NOTICE_AFTER
        {
            recovery.notified = true;
            self.say(NoticeCode::RecordingCutWaiting);
        }
    }

    /// Whether a cut that may go on owns this recording: an empty closure
    /// waits for it, since the meeting resumes or is closed here, with the
    /// count of its tries. A privacy stop or Orbit closing never waits.
    pub(super) fn cut_owns(&self, recording_id: &str) -> bool {
        !self.state.privacy_stopped()
            && !self.closing
            && self
                .capture
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.recording_id == recording_id && recovery.resumable)
    }

    /// A recording that never sealed a chunk and cannot go on (it kept
    /// failing at its start, or its capture did not come back) is closed
    /// empty with its typed cause, and the closure says why and after how
    /// many tries.
    async fn close_empty_cut_recording(&mut self, recovery: CutRecovery) -> Result<()> {
        self.log(
            "cut_recording_closed_empty",
            &[
                ("cause", &recovery.cause),
                ("resumes", &recovery.resumes.to_string()),
            ],
        );
        let interruption =
            self.capture
                .interruption
                .clone()
                .unwrap_or(orbit_core::RecordingInterruption {
                    cause: orbit_core::RecordingInterruptionCause::CauseNotSaved,
                    detail: None,
                    audio_available: false,
                });
        Box::pin(self.close_empty_interrupted(interruption)).await?;
        let closed = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .is_some_and(|recording| recording.ended_at_ms.is_some());
        if closed {
            self.say(
                Notice::new(NoticeCode::RecordingClosedAfterCut)
                    .with_token(NoticeKey::Cause, &recovery.cause)
                    .with(NoticeKey::Fragments, 0_u64)
                    .with(NoticeKey::Attempt, recovery.resumes),
            );
        } else {
            // Something still holds it (a late seal, a drain): decided again
            // on the next turn.
            self.capture.recovery = Some(recovery);
        }
        Ok(())
    }

    fn cut_will_resume_with(&self, recovery: &CutRecovery) -> bool {
        recovery.resumable
            && !self.state.privacy_stopped()
            && self.state.durable.voice.call.is_none()
    }

    async fn try_resume(&mut self, recovery: &CutRecovery) -> Resume {
        if !self.state.capabilities().native_helper() {
            return Resume::Wait;
        }
        match self.apply(Event::ResumeMeeting).await {
            Ok(()) => Resume::Started,
            Err(error) => {
                let refusal = error.downcast_ref::<Refusal>();
                let wait = matches!(
                    refusal,
                    Some(
                        Refusal::Busy { .. }
                            | Refusal::HelperNotRunning
                            | Refusal::PermissionUnknown { .. }
                    )
                );
                self.log(
                    "cut_recording_resume_refused",
                    &[
                        ("cause", &recovery.cause),
                        ("wait", if wait { "true" } else { "false" }),
                        ("refusal", &error.to_string()),
                    ],
                );
                if wait { Resume::Wait } else { Resume::Refused }
            }
        }
    }

    /// A request to resume is not a working capture. Say it only after the
    /// helper acknowledged every recording source and the core is recording.
    pub(super) fn confirm_cut_resumed(&mut self) {
        let at = self.capture.recovery_now();
        let Some(recovery) = self.capture.recovery.as_mut() else {
            return;
        };
        if recovery.resumed_at.is_none() || recovery.resume_announced {
            return;
        }
        recovery.resume_announced = true;
        let missing_ms = u64::try_from(at.duration_since(recovery.waiting_since).as_millis())
            .unwrap_or(u64::MAX);
        let cause = recovery.cause.clone();
        let resumes = recovery.resumes;
        self.log(
            "cut_recording_resumed",
            &[
                ("cause", &cause),
                ("missing_ms", &missing_ms.to_string()),
                ("resumes", &resumes.to_string()),
            ],
        );
        self.say(
            Notice::new(NoticeCode::RecordingResumedAfterCut)
                .with_token(NoticeKey::Cause, &cause)
                .with(NoticeKey::ElapsedMs, missing_ms),
        );
    }

    async fn close_cut_recording(&mut self, recovery: CutRecovery) -> Result<()> {
        self.log(
            "cut_recording_closed",
            &[
                ("cause", &recovery.cause),
                (
                    "resumable",
                    if recovery.resumable { "true" } else { "false" },
                ),
                ("resumes", &recovery.resumes.to_string()),
            ],
        );
        self.apply(Event::CutRecordingClosed {
            ended_at_ms: recovery.cut_at_ms,
            cause: recovery.cause,
        })
        .await
    }

    /// At start: a recording an earlier run left cut (interrupted, or still
    /// closing when it stopped) that will not go on is closed now, at the end
    /// of the audio it kept, so the hub writes its note. One this restart cut
    /// and that may resume is left to `recover_cut_recording`, which resumes
    /// it once the helper is back or closes it after the wait.
    pub(super) async fn close_cut_recording_at_start(&mut self) -> Result<()> {
        let recording = self.state.durable.capture.recording.clone();
        let Some(recording) = recording.filter(|recording| {
            recording.phase == CapturePhase::Interrupted
                && recording.ended_at_ms.is_none()
                && recording.sealed_chunks > 0
        }) else {
            return Ok(());
        };
        if self.cut_owns(&recording.id) {
            return Ok(());
        }
        let mut recovery = match self.capture.recovery.take() {
            Some(recovery) if recovery.recording_id == recording.id => recovery,
            _ => CutRecovery::new(
                &recording.id,
                &self.capture.unattributed_cut_cause(),
                false,
                last_audio_at_ms(&recording),
                self.capture.recovery_now(),
            ),
        };
        // The counts were rebuilt from the sealed chunks after this cut was
        // read from the snapshot: the end is the later of the two.
        recovery.cut_at_ms = recovery.cut_at_ms.max(last_audio_at_ms(&recording));
        self.close_cut_recording(recovery).await
    }
}

fn retry_wait(attempt: usize) -> Duration {
    Duration::from_secs(RETRY_SECONDS[attempt.min(RETRY_SECONDS.len() - 1)])
}

enum Resume {
    Started,
    /// The capture is not back yet (the helper, its permissions, a drain).
    Wait,
    /// The recording cannot go on (expired, a permission denied, …).
    Refused,
}

/// The end of what was recorded: the last chunk a track sealed, or the start
/// when no track says.
fn last_audio_at_ms(recording: &orbit_core::Recording) -> i64 {
    recording
        .tracks
        .iter()
        .map(|track| track.last_chunk_at_ms)
        .max()
        .unwrap_or(recording.started_at_ms)
        .max(recording.started_at_ms)
}

impl crate::daemon::capture_handle::CaptureHandle {
    fn recovery_now(&self) -> Instant {
        #[cfg(test)]
        if let Some(at) = self.recovery_time {
            return at;
        }
        Instant::now()
    }

    /// Freeze/advance only the recovery clock; real store I/O keeps its clock.
    #[cfg(test)]
    pub(super) fn advance_recovery_clock(&mut self, by: Duration) {
        self.recovery_time = Some(self.recovery_now() + by);
    }

    /// As if the capture had stayed away for the whole wait.
    #[cfg(test)]
    pub(super) fn expire_capture_wait(&mut self) {
        if let Some(recovery) = self.recovery.as_mut() {
            recovery.waiting_since = Instant::now()
                .checked_sub(CAPTURE_RETURN_WAIT)
                .unwrap_or(recovery.waiting_since);
        }
    }

    /// The code a closure says when no cut was recorded for it: the typed
    /// interruption the daemon kept, or that the cause is unknown.
    fn unattributed_cut_cause(&self) -> String {
        self.interruption
            .as_ref()
            .and_then(|interruption| serde_json::to_value(interruption.cause).ok())
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "cause_not_saved".to_owned())
    }
}
