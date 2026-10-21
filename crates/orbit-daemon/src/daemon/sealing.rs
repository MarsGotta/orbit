//! PCM ownership ends at a durable store receipt, never at an attempted write.
//! This emergency queue is volatile: it survives a recoverable I/O failure, not
//! process death or power loss. Capture is stopped on failure, never expanded.
use super::{
    Runtime,
    store_actor::{SEAL_RETRY_DELAY, StoreCompletion},
};
use anyhow::{Result, bail};
use orbit_core::notice::NoticeCode;
use std::{
    collections::VecDeque,
    ops::Index,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_PENDING_SEGMENTS: usize = 4096;
// `seal` never waits on the writer: the receipt returns through
// `WorkResult::Sealed` and the select loop stays free. The collector only
// takes a receipt that is already there. REF-03: tests run this same value;
// the test-only 200 ms they had hid a store actor that ran a later `Run`
// before an earlier `Seal`.
pub(super) const OPPORTUNISTIC_RECEIPT_WAIT: Duration = Duration::ZERO;

#[derive(Default)]
pub(super) struct PendingSeals(VecDeque<Arc<crate::capture::Segment>>);

impl PendingSeals {
    pub(super) fn push_back(&mut self, segment: crate::capture::Segment) {
        self.0.push_back(Arc::new(segment));
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &crate::capture::Segment> {
        self.0.iter().map(AsRef::as_ref)
    }

    pub(super) fn shared(&self) -> impl Iterator<Item = &Arc<crate::capture::Segment>> {
        self.0.iter()
    }

    pub(super) fn remove(&mut self, position: usize) -> Option<Arc<crate::capture::Segment>> {
        self.0.remove(position)
    }
}

impl Index<usize> for PendingSeals {
    type Output = crate::capture::Segment;

    fn index(&self, index: usize) -> &Self::Output {
        self.0[index].as_ref()
    }
}

impl Runtime {
    pub(super) fn seal(&mut self, segments: Vec<crate::capture::Segment>) -> Result<()> {
        let mut bytes: usize = self
            .capture
            .pending_seals
            .iter()
            .map(|segment| segment.pcm.len())
            .sum();
        let mut overflow = false;
        // Admit the entire multipista flush before attempting its first write.
        // An error on that write must not drop the remaining tracks.
        for segment in segments {
            if segment.pcm.len() > MAX_PENDING_BYTES.saturating_sub(bytes)
                || self.capture.pending_seals.len() == MAX_PENDING_SEGMENTS
            {
                overflow = true;
                self.note_call_gap(&segment.metadata.recording_id);
                if let Some(recording) = self.state.durable.recording.as_mut()
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
                continue;
            }
            bytes += segment.pcm.len();
            self.capture.pending_seals.push_back(segment);
        }
        if overflow {
            self.capture.reserve_exhausted = true;
            self.say(NoticeCode::AudioReserveExhausted);
            self.work.durable_snapshot = super::DurableSnapshot::from(&self.state);
            let _ = self.store_actor.defer_snapshot(&self.state);
            bail!(
                "Se agotó la reserva de audio; los fragmentos anteriores siguen pendientes y la pérdida adicional está declarada"
            );
        }
        self.validate_seal_destinations()?;
        self.dispatch_pending_seals()?;
        self.collect_ready_seals(OPPORTUNISTIC_RECEIPT_WAIT, false)
    }

    pub(super) fn retry_seals(&mut self) -> Result<()> {
        self.store_actor.retry_seals()?;
        self.validate_seal_destinations()?;
        self.dispatch_pending_seals()?;
        self.collect_ready_seals(OPPORTUNISTIC_RECEIPT_WAIT, true)
    }

    fn dispatch_pending_seals(&mut self) -> Result<()> {
        for segment in self.capture.pending_seals.shared() {
            self.store_actor.seal(Arc::clone(segment))?;
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
        loop {
            let mut failure = None;
            let mut applied = false;
            for completion in self.store_actor.take_completions() {
                match completion {
                    StoreCompletion::Sealed {
                        id,
                        recording,
                        sequence,
                        queue_wait_us: _,
                    } => applied |= self.apply_sealed_receipt(&id, &recording, sequence),
                    StoreCompletion::SealFailed { message } => failure = Some(message),
                }
            }
            if applied {
                self.defer_sealed_snapshot()?;
            }
            // The reserve-loss cause belongs to outstanding audio from the
            // capture that overflowed. Once it has ended and every retained
            // fragment is sealed, a later exit must not report that old 64 MiB
            // exhaustion as its own cause.
            if self.capture.reserve_exhausted
                && self.capture.pending_seals.is_empty()
                && !self.capture.buffer.has_pending_audio()
                && !self.recording_active()
                && self.capture.call_runtime.is_none()
            {
                self.capture.reserve_exhausted = false;
            }
            if let Some(message) = failure {
                self.capture.seal_retry_at = Some(Instant::now() + SEAL_RETRY_DELAY);
                self.say_failure(NoticeCode::RecordingSealFailed, &message);
                if report_failure {
                    bail!(message);
                }
                return Ok(());
            }
            if self.capture.pending_seals.is_empty() || Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }

    fn validate_seal_destinations(&self) -> Result<()> {
        let chunks = self.paths.store.join("chunks");
        if !std::fs::symlink_metadata(&chunks)?.file_type().is_dir() {
            bail!("La ruta de fragmentos no es un directorio privado");
        }
        for segment in self.capture.pending_seals.iter() {
            let destination = chunks.join(format!("{}.orbit", segment.metadata.id));
            match std::fs::symlink_metadata(destination) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    bail!("El destino del fragmento no es un archivo regular")
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub(super) fn apply_sealed_receipt(
        &mut self,
        id: &str,
        recording: &str,
        sequence: u64,
    ) -> bool {
        let position = self.capture.pending_seals.iter().position(|segment| {
            segment.metadata.id == id
                && segment.metadata.recording_id == recording
                && segment.metadata.sequence == sequence
        });
        if let Some(position) = position
            && let Some(segment) = self.capture.pending_seals.remove(position)
        {
            self.store_actor.acknowledge_seal(&segment.metadata.id);
            if let Some(active) = self
                .state
                .durable
                .recording
                .as_mut()
                .filter(|active| active.id == recording)
            {
                active.sealed_chunks = active.sealed_chunks.saturating_add(1);
                if let Some(track) = active
                    .tracks
                    .iter_mut()
                    .find(|track| track.name == segment.metadata.track)
                {
                    track.sealed_chunks = track.sealed_chunks.saturating_add(1);
                    track.last_chunk_at_ms =
                        track.last_chunk_at_ms.max(segment.metadata.started_at_ms);
                }
            }
            self.work.finalize_ready.remove(recording);
            return true;
        }
        false
    }

    /// Admit the coalesced projection snapshot without waiting for its disk
    /// write. The seal itself is already durable; this snapshot only preserves
    /// its counters across restart. Recording the admitted value keeps the
    /// following Tick from synchronously writing the same projection again.
    pub(super) fn defer_sealed_snapshot(&mut self) -> Result<()> {
        self.store_actor.defer_snapshot(&self.state)?;
        self.work.durable_snapshot = super::DurableSnapshot::from(&self.state);
        Ok(())
    }

    /// The verticals' timers: notes, the meeting question, calendars,
    /// sources, control and call detection. A failure here is a data fault of
    /// that vertical, never the capture's.
    pub(super) fn maintain_verticals(
        &mut self,
    ) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            self.maintain_recording_notes();
            Box::pin(self.maintain_meeting_question()).await?;
            self.maintain_calendar();
            Box::pin(self.check_source_timeout()).await?;
            Box::pin(self.maintain_computer()).await?;
            Box::pin(self.maintain_call_detection()).await
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
            // REF-41 r2 · a failed stopped-audio flush is still the
            // capture's own failure and still reaches the containment
            // table, but only after this turn's whole upkeep has run — the
            // same rule as A2's seal retry below. Before, the early return
            // here skipped the deferred drain, the core `Tick` (SYS-20's
            // finalize deadline), the call checks and the queued finalize,
            // so a disk that kept refusing the local flush also froze them,
            // and a notice delivered late could re-show «No cierres Orbit»
            // for audio the Tick had already moved on from.
            let flush_stopped = if self.stopped_audio_owed()
                && !self.audio_is_draining()
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
            };
            // REF-41 A2 · a failed sealing retry is still the capture's own
            // failure and still reaches the containment table, but only
            // after this turn's whole upkeep has run. Before, the `?` here
            // returned before the deferred drain, the core `Tick` (SYS-20's
            // finalize deadline), the call checks and the queued finalize,
            // so a disk that kept refusing seals also froze them.
            let seal_retry = if !self.capture.pending_seals.is_empty()
                && self
                    .capture
                    .seal_retry_at
                    .is_none_or(|deadline| Instant::now() >= deadline)
            {
                self.retry_seals().err()
            } else {
                None
            };
            // SYS-20: before the tick, never after. `Event::Tick` is what
            // interrupts a recording that has been finalizing too long, so a
            // finish that is ready to happen has to happen first — otherwise
            // the deadline reaches a recording the daemon already had
            // everything it needed to close.
            Box::pin(self.resume_deferred_drain()).await?;
            // Permission/call expiry sends capture commands, so Tick belongs
            // to capture maintenance. It runs after a ready deferred finish,
            // preserving SYS-20's established ordering.
            let before_tick = self
                .state
                .durable
                .recording
                .as_ref()
                .map(|recording| (recording.id.clone(), recording.phase.clone()));
            let tick_result = Box::pin(self.apply(orbit_core::Event::Tick)).await;
            let tick_cause = before_tick.and_then(|(id, phase)| {
                let recording = self.state.durable.recording.as_ref()?;
                if recording.id != id
                    || recording.phase != orbit_core::CapturePhase::Interrupted
                    || recording.ended_at_ms.is_some()
                {
                    return None;
                }
                match phase {
                    orbit_core::CapturePhase::Starting | orbit_core::CapturePhase::Recording => {
                        Some(orbit_core::RecordingInterruptionCause::RecordingExpired)
                    }
                    orbit_core::CapturePhase::Finalizing => {
                        Some(orbit_core::RecordingInterruptionCause::CaptureFinalizeTimedOut)
                    }
                    orbit_core::CapturePhase::Idle
                    | orbit_core::CapturePhase::Paused
                    | orbit_core::CapturePhase::Interrupted => None,
                }
            });
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
                Box::pin(
                    self.close_empty_interrupted(orbit_core::RecordingInterruption {
                        cause,
                        detail: None,
                        audio_available: false,
                    }),
                )
                .await?;
            }
            Box::pin(self.check_call_health()).await?;
            Box::pin(self.finish_call_drain()).await?;
            // A closed recording's manifest is capture durability, not upload
            // scheduling. Queue it here so a failed closure is classified as
            // capture maintenance; `schedule_upload` may observe it again
            // idempotently when admitting network work.
            let tail = Box::pin(self.queue_finalize()).await;
            // A failed flush parks its retry behind `SEAL_RETRY_DELAY`, so
            // it excludes the seal-retry branch below on the same turn;
            // both are folded anyway so neither can silently win.
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
        })
    }

    /// The pending-job count and upload queue. Their failures are the state's
    /// or the network's, said and never a cut; the core Tick lives with the
    /// capture commands it can emit.
    pub(super) fn maintain_state(&mut self) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            self.state.durable.pending_jobs = self.reader.pending_job_count()?;
            Box::pin(self.schedule_upload()).await
        })
    }
}

#[cfg(test)]
#[path = "sealing_tests.rs"]
mod tests;
