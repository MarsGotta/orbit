//! PCM ownership ends at a durable store receipt, never at an attempted write.
//! The seals the recording owes live in the capture lane (`capture_lane`),
//! shared with the capture task; this is the loop's side of them.
use super::{Runtime, store_actor::SEAL_RETRY_DELAY};
use anyhow::{Result, bail};
use orbit_core::notice::NoticeCode;
use std::time::{Duration, Instant};

// `seal` never waits on the writer: the receipt returns through
// `WorkResult::Sealed` and the select loop stays free. The collector only
// takes a receipt that is already there. Tests run this same value:
// the test-only 200 ms they had hid a store actor that ran a later `Run`
// before an earlier `Seal`.
pub(super) const OPPORTUNISTIC_RECEIPT_WAIT: Duration = Duration::ZERO;

impl Runtime {
    pub(super) fn seal(&mut self, mut segments: Vec<crate::capture::Segment>) -> Result<()> {
        // «Terminar sin guardar»: what drains of it is never kept.
        if let Some(discarded) = self.state.durable.discarding() {
            segments.retain(|segment| segment.metadata.recording_id != discarded);
        }
        let sealed = self.seal_segments(segments);
        // The cause goes with the notice its caller says (`say_failure`).
        if let Err(error) = &sealed {
            self.capture.seal_cause = Some(super::capture_lane::seal_cause(error));
        }
        sealed
    }

    fn seal_segments(&mut self, segments: Vec<crate::capture::Segment>) -> Result<()> {
        let overflow = self.capture.lane.lock().admit(segments);
        if !overflow.is_empty() {
            self.declare_reserve_overflow(&overflow);
            return Err(super::capture_lane::ReserveExhausted.into());
        }
        self.dispatch_pending_seals()?;
        self.collect_ready_seals(OPPORTUNISTIC_RECEIPT_WAIT, false)
    }

    /// Segments that did not fit in the reserve are lost: each one is a gap
    /// of its recording and its track, and the notice says so.
    fn declare_reserve_overflow(&mut self, overflow: &[crate::capture::Segment]) {
        for segment in overflow {
            self.audio.note_call_gap(&segment.metadata.recording_id);
            if let Some(recording) = self.state.durable.capture.recording.as_mut()
                && recording.id == segment.metadata.recording_id
            {
                recording.gaps = recording.gaps.saturating_add(1);
                if let Some(track) = recording
                    .tracks
                    .iter_mut()
                    .find(|track| track.name == segment.metadata.track)
                {
                    track.gaps = track.gaps.saturating_add(1);
                }
            }
        }
        self.capture.reserve_exhausted = true;
        self.say(NoticeCode::AudioReserveExhausted);
        self.persisted.durable = super::DurableSnapshot::from(&self.state);
        let _ = self.store_actor.defer_snapshot(&self.state);
    }

    pub(super) fn retry_seals(&mut self) -> Result<()> {
        self.store_actor.retry_seals()?;
        self.dispatch_pending_seals()?;
        self.collect_ready_seals(OPPORTUNISTIC_RECEIPT_WAIT, true)
    }

    fn dispatch_pending_seals(&mut self) -> Result<()> {
        let port = self.store_actor.capture_port();
        {
            let core = self.capture.lane.lock();
            core.validate_destinations(&self.paths.store)?;
            core.dispatch(&port)?;
        }
        self.capture.seal_retry_at = None;
        Ok(())
    }

    pub(super) fn collect_ready_seals(
        &mut self,
        wait: Duration,
        report_failure: bool,
    ) -> Result<()> {
        let deadline = Instant::now() + wait;
        let port = self.store_actor.capture_port();
        loop {
            self.capture.lane.lock().collect(&port);
            let failure = self.absorb_seal_receipts()?;
            // The reserve-loss cause belongs to outstanding audio from the
            // capture that overflowed. Once it has ended and every retained
            // fragment is sealed, a later exit must not report that old 64 MiB
            // exhaustion as its own cause.
            if self.capture.reserve_exhausted
                && self.capture.pending_seals.is_empty()
                && !self.capture.buffer.has_pending_audio()
                && !self.recording_active()
                && self.audio.call.is_none()
            {
                self.capture.reserve_exhausted = false;
            }
            if let Some(failure) = failure {
                self.capture.seal_retry_at = Some(Instant::now() + SEAL_RETRY_DELAY);
                let notice = orbit_core::notice::Notice::new(NoticeCode::RecordingSealFailed)
                    .with_token(orbit_core::notice::NoticeKey::Cause, failure.cause)
                    .with(
                        orbit_core::notice::NoticeKey::Fragments,
                        failure.owed as u64,
                    );
                self.say_failure(notice, &failure.message);
                if report_failure {
                    bail!(failure.message);
                }
                return Ok(());
            }
            if self.capture.pending_seals.is_empty() || Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }

    /// Counts the receipts the lane took, whoever took them, on the recording
    /// they belong to, and returns the writer's latest refusal if any.
    fn absorb_seal_receipts(&mut self) -> Result<Option<super::capture_lane::SealFailure>> {
        let (receipts, failure) = {
            let mut core = self.capture.lane.lock();
            (
                std::mem::take(&mut core.outbox.receipts),
                core.outbox.seal_failure.take(),
            )
        };
        let applied = !receipts.is_empty();
        for receipt in receipts {
            self.count_sealed_receipt(&receipt);
        }
        if applied {
            self.defer_sealed_snapshot()?;
        }
        Ok(failure)
    }

    fn count_sealed_receipt(&mut self, receipt: &super::capture_lane::Receipt) {
        if let Some(active) = self
            .state
            .durable
            .capture
            .recording
            .as_mut()
            .filter(|active| active.id == receipt.recording)
        {
            active.sealed_chunks = active.sealed_chunks.saturating_add(1);
            if let Some(track) = active
                .tracks
                .iter_mut()
                .find(|track| track.name == receipt.track)
            {
                track.sealed_chunks = track.sealed_chunks.saturating_add(1);
                track.last_chunk_at_ms = track.last_chunk_at_ms.max(receipt.started_at_ms);
            }
        }
        self.transfers.finalize_ready.remove(&receipt.recording);
    }

    /// Admit the coalesced projection snapshot without waiting for its disk
    /// write. The seal itself is already durable; this snapshot only preserves
    /// its counters across restart. Recording the admitted value keeps the
    /// following Tick from synchronously writing the same projection again.
    pub(super) fn defer_sealed_snapshot(&mut self) -> Result<()> {
        self.store_actor.defer_snapshot(&self.state)?;
        self.persisted.durable = super::DurableSnapshot::from(&self.state);
        Ok(())
    }

    /// The verticals' timers: notes, the meeting question, calendars, an
    /// answer to the agent, sources, control and call detection. A failure here is a data fault of
    /// that vertical, never the capture's. Each is a named step of the tick's
    /// turn, so a slow one is named in `daemon_turn_slow`.
    pub(super) fn maintain_verticals(
        &mut self,
    ) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            let watch = self.turn_watch.clone();
            watch.time_sync("maintain_verticals.recording_notes", || {
                self.maintain_recording_notes()
            });
            watch
                .time(
                    "maintain_verticals.meeting_question",
                    Box::pin(self.maintain_meeting_question()),
                )
                .await?;
            watch.time_sync("maintain_verticals.calendar", || self.maintain_calendar());
            watch
                .time(
                    "maintain_verticals.interaction_response",
                    Box::pin(self.expire_interaction_response()),
                )
                .await?;
            watch
                .time(
                    "maintain_verticals.sources",
                    Box::pin(self.check_source_timeout()),
                )
                .await?;
            watch.time_sync("maintain_verticals.files", || self.check_files_timeout());
            watch
                .time(
                    "maintain_verticals.computer",
                    Box::pin(self.maintain_computer()),
                )
                .await?;
            watch
                .time(
                    "maintain_verticals.call_detection",
                    Box::pin(self.maintain_call_detection()),
                )
                .await
        })
    }

    /// The capture's own upkeep: the local flush of audio a stop left
    /// behind, sealing retries, a deferred drain, a call's health and its
    /// drain. A failure here puts the audio at risk and cuts what is live.
    pub(super) fn maintain_capture(
        &mut self,
    ) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            Box::pin(self.retry_full_stop_send()).await?;
            self.retry_retired_stream_stops().await;
            let flush_stopped = self.flush_owed_stopped_audio().await;
            let seal_retry = self.retry_due_seals();
            // Before the tick, never after. `Event::Tick` is what
            // interrupts a recording that has been finalizing too long, so a
            // finish that is ready to happen has to happen first — otherwise
            // the deadline reaches a recording the daemon already had
            // everything it needed to close.
            Box::pin(self.resume_deferred_drain()).await?;
            Box::pin(self.tick_capture()).await?;
            // A cut recording whose audio has settled resumes once the
            // capture is back, or is closed; never left for «Terminar».
            Box::pin(self.recover_cut_recording()).await?;
            Box::pin(self.check_call_health()).await?;
            Box::pin(self.finish_call_drain()).await?;
            // A closed recording's manifest is capture durability, not upload
            // scheduling. Queue it here so a failed closure is classified as
            // capture maintenance; `schedule_upload` may observe it again
            // idempotently when admitting network work.
            let tail = Box::pin(self.queue_finalize()).await;
            fold_capture_failures(flush_stopped, seal_retry, tail)
        })
    }

    /// A failed stopped-audio flush is still the capture's own failure and
    /// still reaches the containment table, but only after this turn's whole
    /// upkeep has run — the same rule as the seal retry. Before, the early
    /// return here skipped the deferred drain, the core `Tick` (the finalize
    /// deadline), the call checks and the queued finalize, so a disk that
    /// kept refusing the local flush also froze them, and a notice delivered
    /// late could re-show «No cierres Orbit» for audio the Tick had already
    /// moved on from.
    async fn flush_owed_stopped_audio(&mut self) -> Option<anyhow::Error> {
        if self.stopped_audio_owed()
            && !self.audio.audio_is_draining()
            && self.capture.buffer.voice.is_some()
            && self
                .capture
                .seal_retry_at
                .is_none_or(|deadline| Instant::now() >= deadline)
            && let Err(error) = self.flush_stopped_audio().await
        {
            self.capture.seal_retry_at = Some(Instant::now() + SEAL_RETRY_DELAY);
            Some(error)
        } else {
            None
        }
    }

    /// A failed sealing retry is still the capture's own failure and still
    /// reaches the containment table, but only after this turn's whole
    /// upkeep has run. Before, the `?` here returned before the deferred
    /// drain, the core `Tick` (the finalize deadline), the call checks and
    /// the queued finalize, so a disk that kept refusing seals also froze
    /// them.
    fn retry_due_seals(&mut self) -> Option<anyhow::Error> {
        if !self.capture.pending_seals.is_empty()
            && self
                .capture
                .seal_retry_at
                .is_none_or(|deadline| Instant::now() >= deadline)
        {
            self.retry_seals().err()
        } else {
            None
        }
    }

    /// Silence questions and drain acknowledgements belong to capture upkeep.
    /// Both finish through the normal drain; missing signal facts keep capture.
    async fn tick_capture(&mut self) -> Result<()> {
        let before_tick = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|recording| (recording.id.clone(), recording.phase.clone()));
        Box::pin(self.check_silence()).await?;
        let tick_result = Box::pin(self.apply(orbit_core::Event::Tick)).await;
        let tick_cause =
            before_tick.and_then(|(id, phase)| tick_interruption(&self.state, &id, phase));
        if let Some(cause) = tick_cause {
            self.capture
                .interruption
                .get_or_insert(orbit_core::RecordingInterruption {
                    cause,
                    detail: None,
                    audio_available: false,
                });
        }
        tick_result?;
        if let Some(cause) = tick_cause {
            self.capture.recovery = None;
            Box::pin(
                self.close_empty_interrupted(orbit_core::RecordingInterruption {
                    cause,
                    detail: None,
                    audio_available: false,
                }),
            )
            .await?;
        }
        Ok(())
    }

    /// The pending-job count and upload queue. Their failures are the state's
    /// or the network's, said and never a cut; the core Tick lives with the
    /// capture commands it can emit.
    pub(super) fn maintain_state(&mut self) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            self.state.durable.pending_jobs = self.reader.pending_job_count()?;
            Box::pin(self.schedule_upload()).await?;
            // A voice note whose seal failed is still only in memory, and
            // nothing else comes back for it: its seal is retried like a
            // recording's. A failure is said, never a cut of the meeting.
            if self.capture.buffer.voice.is_some()
                && !self.state.durable.voice.voice_active
                && self.audio.call.is_none()
                && !self.audio.audio_is_draining()
                && !self.stopped_audio_owed()
                && self
                    .capture
                    .seal_retry_at
                    .is_none_or(|deadline| Instant::now() >= deadline)
                && let Err(error) = Box::pin(self.dispatch_voice_transcription()).await
            {
                self.capture.seal_retry_at = Some(Instant::now() + SEAL_RETRY_DELAY);
                return Err(error);
            }
            Box::pin(self.send_waiting_voice_note()).await
        })
    }
}

/// Only an unconfirmed close can now be interrupted by Tick.
fn tick_interruption(
    state: &orbit_core::State,
    id: &str,
    phase: orbit_core::CapturePhase,
) -> Option<orbit_core::RecordingInterruptionCause> {
    let recording = state.durable.capture.recording.as_ref()?;
    if recording.id != id
        || recording.phase != orbit_core::CapturePhase::Interrupted
        || recording.ended_at_ms.is_some()
    {
        return None;
    }
    match phase {
        orbit_core::CapturePhase::Finalizing => {
            Some(orbit_core::RecordingInterruptionCause::CaptureFinalizeTimedOut)
        }
        orbit_core::CapturePhase::Idle
        | orbit_core::CapturePhase::Starting
        | orbit_core::CapturePhase::Recording
        | orbit_core::CapturePhase::Paused
        | orbit_core::CapturePhase::Interrupted => None,
    }
}

/// A failed flush parks its retry behind `SEAL_RETRY_DELAY`, so it excludes
/// the seal retry on the same turn; both are folded anyway so neither can
/// silently win, and the queued closure's failure is said with them.
fn fold_capture_failures(
    flush_stopped: Option<anyhow::Error>,
    seal_retry: Option<anyhow::Error>,
    tail: Result<()>,
) -> Result<()> {
    let capture_failure = match (flush_stopped, seal_retry) {
        (None, retry) => retry,
        (Some(error), None) => Some(error),
        (Some(flush), Some(retry)) => Some(flush.context(format!(
            "y el reintento de sellado también falló: {retry:#}"
        ))),
    };
    match (capture_failure, tail) {
        (None, outcome) => outcome,
        (Some(error), Ok(())) => Err(error),
        (Some(error), Err(tail)) => {
            Err(error.context(format!("y el cierre en cola también falló: {tail:#}")))
        }
    }
}

#[cfg(test)]
#[path = "sealing_tests.rs"]
mod tests;
