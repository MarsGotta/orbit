//! The writer thread's side: one durable attempt at each operation, and
//! the notices its failures leave for the daemon.
use super::*;

pub(super) struct Writer {
    pub(super) store: Store,
    pub(super) events: tokio::sync::mpsc::Sender<WorkResult>,
    pub(super) completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    pub(super) seal_retry_at: Instant,
    pub(super) last_snapshot: Option<Instant>,
    pub(super) held_startup_stop: Option<Box<State>>,
    pub(super) startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    pub(super) diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    /// `StoreProblem` notices the full work channel refused, oldest
    /// first. They are delivered as soon as the daemon's loop makes room;
    /// none of them is the `Sealed` wake-up, whose receipt already lives in
    /// the completion log.
    pub(super) pending_problems: VecDeque<String>,
    /// Notices dropped after `MAX_PENDING_PROBLEMS`: the newest delivered
    /// notice carries this count, so even the bounded loss is said — and it
    /// is said in the message the daemon's single notice slot keeps, not in
    /// one the rest of the burst would overwrite.
    pub(super) overflowed_problems: u64,
}

impl Writer {
    pub(super) fn report_receipt_loss(&mut self, operation: &str) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record("store_receipt_receiver_gone", &[("operation", operation)]);
        }
    }
    /// Record the wake-up notice for a completion already logged above.
    /// Neither failure may hold the ordered writer waiting for the daemon's
    /// loop (that once deadlocked the writer and the loop on each other), and
    /// neither loses the receipt: it stays in
    /// `self.completions`. On a full channel the notice is dropped, and a
    /// dropped notice is not applied by another send — the daemon's
    /// maintenance collector reads the completion log on its 1 s tick, or at
    /// most `SEAL_RETRY_DELAY` (5 s) later while a failed seal has the retry
    /// backed off. A closed channel means that loop no longer exists.
    /// A `StoreProblem` may not take this drop path — its only
    /// record can be this notice — so problems go through
    /// [`Self::report_problem`], which waits for room.
    pub(super) fn wake_daemon(&self, event: WorkResult) {
        match self.events.try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// A store problem is never lost to a full work channel: it
    /// stays pending here and is delivered as soon as there is room. A
    /// closed channel still drops it — that daemon loop no longer exists to
    /// receive anything, and the receipts of the operations it asked for
    /// have already been answered to their own waiters.
    pub(super) fn report_problem(&mut self, message: String) {
        if self.pending_problems.len() >= MAX_PENDING_PROBLEMS {
            self.pending_problems.pop_front();
            self.overflowed_problems = self.overflowed_problems.saturating_add(1);
        }
        self.pending_problems.push_back(message);
        self.deliver_pending_problems();
    }

    /// Deliver the pending problems the channel can take now, oldest first.
    /// Bounded non-blocking attempts: this can never hold the ordered writer
    /// on the daemon's loop.
    pub(super) fn deliver_pending_problems(&mut self) {
        while let Some(oldest) = self.pending_problems.front().cloned() {
            // The daemon keeps a single notice slot, so the
            // newer messages of a burst overwrite the older ones in it. The
            // count of dropped notices must reach whoever reads that slot,
            // so it rides the newest pending message — the one the slot
            // ends up holding — and is only retired once that one is
            // delivered. On an older message it would be overwritten by the
            // rest of the burst before anyone read it.
            let newest = self.pending_problems.len() == 1;
            let message = if newest && self.overflowed_problems > 0 {
                format!(
                    "{oldest} Además, {} aviso(s) de almacenamiento anteriores no cupieron \
                     en la cola de {MAX_PENDING_PROBLEMS} y se descartaron.",
                    self.overflowed_problems
                )
            } else {
                oldest
            };
            match self
                .events
                .try_send(WorkResult::Store(crate::daemon::StoreWork::StoreProblem {
                    message,
                })) {
                Ok(()) => {
                    self.pending_problems.pop_front();
                    if newest {
                        self.overflowed_problems = 0;
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => break,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    self.pending_problems.clear();
                    self.overflowed_problems = 0;
                    break;
                }
            }
        }
    }

    /// When a pending problem notice is next retried, if any are pending.
    /// The only wake source for an otherwise idle writer holding a notice.
    pub(super) fn problem_flush_due(&self) -> Option<Instant> {
        (!self.pending_problems.is_empty()).then_some(Instant::now() + PENDING_PROBLEM_RETRY)
    }

    /// One durable attempt at a seal. `owed` counts every seal still in the
    /// actor's hands, this one included, for the notice a failure leaves.
    pub(super) fn seal(
        &mut self,
        segment: &crate::capture::Segment,
        owed: usize,
        queue_wait_us: Option<u64>,
    ) -> bool {
        let result = native::wav(
            &segment.pcm,
            segment.metadata.sample_rate,
            segment.metadata.channels,
        )
        .and_then(|wav| Ok(self.store.seal_chunk(&segment.metadata, &wav)?));
        match result {
            Ok(_) => {
                push_completion(
                    &self.completions,
                    StoreCompletion::Sealed {
                        id: segment.metadata.id.clone(),
                        recording: segment.metadata.recording_id.clone(),
                        sequence: segment.metadata.sequence,
                        queue_wait_us,
                    },
                );
                self.wake_daemon(WorkResult::Store(crate::daemon::StoreWork::Sealed {
                    recording: segment.metadata.recording_id.clone(),
                    sequence: segment.metadata.sequence,
                }));
                true
            }
            Err(error) => {
                let message = format!(
                    "Audio pendiente en memoria: {owed} fragmentos. No cierres Orbit; se reintentará guardar localmente. {error}"
                );
                push_completion(
                    &self.completions,
                    StoreCompletion::SealFailed {
                        message: message.clone(),
                        cause: crate::daemon::capture_lane::seal_cause(&error),
                        owed,
                    },
                );
                self.report_problem(message);
                false
            }
        }
    }

    /// Retries parked seals oldest first and stops at the first that fails
    /// again, which backs the whole retry lane off, as it always did.
    pub(super) fn retry_parked(&mut self, lanes: &mut Lanes) {
        while let Some(segment) = lanes
            .parked
            .front()
            .map(|parked| Arc::clone(&parked.segment))
        {
            if !self.seal(&segment, lanes.owed_seals(), None) {
                self.seal_retry_at = Instant::now() + SEAL_RETRY_DELAY;
                return;
            }
            lanes.parked.pop_front();
        }
    }

    /// When the parked seals are next retried, if any are parked.
    pub(super) fn retry_due(&self, lanes: &Lanes) -> Option<Instant> {
        (!lanes.parked.is_empty()).then_some(self.seal_retry_at)
    }

    /// When the coalesced snapshot may be written, if one is pending.
    ///
    /// The loop both writes and sleeps on this one answer, so a
    /// deadline that has passed always has a write behind it. Before, the
    /// write also waited for the parked seals and the sleep did not: with a
    /// snapshot due and a seal parked, the deadline was always in the past and
    /// the loop turned without sleeping, measured at 5.4 to 6.6 million turns
    /// and 3.7 to 4.4 s of CPU over one `SEAL_RETRY_DELAY` of 5 s, for two
    /// operations and two seal attempts, at a load average of 91 to 106 on 32
    /// cores; after, 24 turns and 0.01 to 0.02 s. Audio owns the strictest
    /// durability budget, so a seal or a `Run` already admitted still goes
    /// first: a coalesced presentation snapshot must never sit between
    /// receipts that can be written now. A parked seal cannot be written now;
    /// it waits for `retry_due`, and when both fall due in one turn the retry
    /// runs first.
    pub(super) fn snapshot_due(&self, lanes: &Lanes) -> Option<Instant> {
        if lanes.snapshot.is_none() || !lanes.seals.is_empty() || !lanes.runs.is_empty() {
            return None;
        }
        Some(
            self.last_snapshot
                .map_or_else(Instant::now, |written| written + SNAPSHOT_INTERVAL),
        )
    }

    pub(super) fn write_snapshot(&mut self, lanes: &mut Lanes) {
        let Some(pending) = lanes.snapshot.take() else {
            return;
        };
        let effective = self.held_startup_stop.as_ref().filter(|_| {
            self.startup_guard_active.load(Ordering::Acquire)
                && (!pending.state.privacy_stopped()
                    || pending.state.privacy_stop_by()
                        != Some(orbit_core::privacy_stop::StopRequester::User))
        });
        let state = effective.map_or(pending.state.as_ref(), Box::as_ref);
        let result = self
            .store
            .save_snapshot(&state.durable)
            .map_err(|error| refusal(&error, lanes));
        for receipt in pending.receipts {
            if receipt.send(result.clone()).is_err() {
                self.report_receipt_loss("snapshot");
            }
        }
        self.last_snapshot = Some(Instant::now());
        if let Err(message) = result {
            // A refused snapshot's only notice to the daemon is this
            // one — its waiters have their receipts, but a deferred snapshot
            // has none — so it reports through the pending lane instead of a
            // droppable wake-up.
            self.report_problem(message);
            // Executed and failed: the state is kept for the next coalesced
            // write, and holds no operation already admitted.
            lanes.snapshot = Some(PendingSnapshot {
                since: lanes.admitted,
                state: pending.state,
                receipts: Vec::new(),
            });
        }
    }

    pub(super) fn write_critical(&mut self, lanes: &mut Lanes, critical: CriticalSnapshot) {
        let superseded = lanes.snapshot.take();
        let effective = self.held_startup_stop.as_ref().filter(|_| {
            !critical.startup_stop
                && self.startup_guard_active.load(Ordering::Acquire)
                && (!critical.state.privacy_stopped()
                    || critical.state.privacy_stop_by()
                        != Some(orbit_core::privacy_stop::StopRequester::User))
        });
        let state = effective.map_or(critical.state.as_ref(), Box::as_ref);
        let result = match critical.job {
            Some(job) => self.store.save_snapshot_with_job(&state.durable, &job),
            None => self.store.save_snapshot(&state.durable),
        }
        .map_err(|error| refusal(&error, lanes));
        if critical.startup_stop && result.is_ok() {
            self.held_startup_stop = Some(critical.state.clone());
        }
        if let Some(superseded) = superseded {
            for receipt in superseded.receipts {
                if receipt.send(result.clone()).is_err() {
                    self.report_receipt_loss("snapshot_superseded");
                }
            }
        }
        if critical.receipt.send(result).is_err() {
            self.report_receipt_loss("critical");
        }
        self.last_snapshot = Some(Instant::now());
    }
}

pub(super) fn push_completion(
    completions: &Mutex<VecDeque<StoreCompletion>>,
    completion: StoreCompletion,
) {
    if let Ok(mut completions) = completions.lock() {
        completions.push_back(completion);
    }
}
