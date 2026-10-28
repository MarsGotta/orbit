//! The writer thread's loop: the next step of the lanes, the deadlines it
//! sleeps to, and the supervision of every step.
//!
//! A step that panics is abandoned and the writer goes on with the next one,
//! with its lanes and its connection as they were: what it owes is still in
//! the lanes, and SQLite is at its last commit, because the panic unwound
//! through the transaction's guard, which rolls it back. Nothing is answered
//! for the abandoned step except a failure:
//!
//! - a seal is parked and retried after `SEAL_RETRY_DELAY`, as a seal the
//!   disk refused, and its receipt says it failed. Sealing is idempotent by
//!   chunk id (`Store::seal_chunk`), so a retry of a seal that did reach the
//!   disk before the panic finds it there and writes nothing twice;
//! - a `Run`, a snapshot or a critical snapshot lose their receipt, which
//!   its waiter reads as the writer having lost it, never as written.
//!
//! So no chunk is acknowledged that was not written. Past the writer's
//! budget the thread ends: every later admission is refused, and the loop
//! ends the daemon on the spent budget.
use super::*;
use crate::supervision::{Actor, Supervision, Verdict};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The cause of a seal abandoned by a panic, as the notice carries it.
pub(in crate::daemon) const WRITER_PANIC_CAUSE: &str = "writer_panicked";

/// What the writer thread starts from.
pub(super) struct WriterParts {
    pub(super) store: Store,
    pub(super) receiver: mpsc::Receiver<StoreOp>,
    pub(super) events: tokio::sync::mpsc::Sender<WorkResult>,
    pub(super) completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    pub(super) turns: Arc<AtomicU64>,
    pub(super) startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    pub(super) diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    pub(super) supervision: Supervision,
}

/// Runs `step`, and says whether it panicked.
fn guarded<T>(step: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(step)).ok()
}

pub(super) fn run(parts: WriterParts) {
    let WriterParts {
        store,
        receiver,
        events,
        completions,
        turns,
        startup_guard_active,
        diagnostics,
        supervision,
    } = parts;
    let mut lanes = Lanes::default();
    let mut writer = Writer {
        store,
        events,
        completions,
        seal_retry_at: Instant::now(),
        last_snapshot: None,
        held_startup_stop: None,
        startup_guard_active,
        diagnostics,
        pending_problems: VecDeque::new(),
        overflowed_problems: 0,
    };
    let mut disconnected = false;

    loop {
        turns.fetch_add(1, Ordering::Relaxed);
        // A problem notice that was refused for a full channel is
        // retried here first: the loop it must reach may have made room
        // while the writer was inside a system call.
        writer.deliver_pending_problems();
        while let Ok(operation) = receiver.try_recv() {
            lanes.admit(operation);
        }

        let step = lanes.next_step();
        let panicked = writer.step(&mut lanes, step);
        let panicked = panicked | writer.due(&mut lanes);
        if panicked && supervision.panicked(Actor::StoreWriter) == Verdict::GiveUp {
            break;
        }

        if disconnected {
            break;
        }
        // The writer sleeps until the next real deadline, computed
        // from the same answers that gate the writes above. With nothing owed
        // it waits for an operation or for the queue to close: there is no
        // deadline to wake for, so it wakes for none. A pending problem
        // notice is itself a real deadline: the writer is the only
        // one who can retry its delivery.
        let deadline = if lanes.ready() {
            Some(Instant::now())
        } else {
            [
                writer.retry_due(&lanes),
                writer.snapshot_due(&lanes),
                writer.problem_flush_due(),
            ]
            .into_iter()
            .flatten()
            .min()
        };
        let next = match deadline {
            Some(deadline) => {
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(operation) => lanes.admit(operation),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
        }
    }
}

impl Writer {
    /// One step the lanes chose. Returns whether it panicked.
    fn step(&mut self, lanes: &mut Lanes, step: Step) -> bool {
        match step {
            Step::Seal => {
                let Some(queued) = lanes.seals.pop_front() else {
                    return false;
                };
                let owed = lanes.owed_seals().saturating_add(1);
                let queue_wait_us = Instant::now()
                    .duration_since(queued.operation.1)
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64;
                let segment = queued.operation.0;
                let attempt = guarded(|| {
                    #[cfg(test)]
                    faults::seal(&segment.metadata.id);
                    self.seal(&segment, owed, Some(queue_wait_us))
                });
                if attempt.is_none() {
                    self.seal_abandoned(&segment, owed);
                }
                if attempt != Some(true) {
                    let failed_at = Instant::now();
                    if lanes.parked.is_empty() {
                        self.seal_retry_at = failed_at + SEAL_RETRY_DELAY;
                    }
                    lanes.parked.push_back(Parked { segment, failed_at });
                }
                attempt.is_none()
            }
            Step::Retry => {
                lanes.retry = None;
                self.retry_guarded(lanes)
            }
            Step::Flush => guarded(|| self.write_snapshot(lanes)).is_none(),
            Step::Run => {
                let Some(queued) = lanes.runs.pop_front() else {
                    return false;
                };
                match guarded(|| (queued.operation)(&mut self.store)) {
                    Some(true) => {
                        self.report_receipt_loss("run");
                        false
                    }
                    Some(false) => false,
                    None => true,
                }
            }
            Step::Critical => {
                let Some(queued) = lanes.critical.pop_front() else {
                    return false;
                };
                guarded(|| self.write_critical(lanes, queued.operation)).is_none()
            }
            Step::Idle => false,
        }
    }

    /// The retry and the snapshot that fell due meanwhile. Audio first: a
    /// retry that has fallen due runs before a snapshot due in the same
    /// turn. Returns whether either panicked.
    fn due(&mut self, lanes: &mut Lanes) -> bool {
        let mut panicked = false;
        if self
            .retry_due(lanes)
            .is_some_and(|due| Instant::now() >= due)
        {
            panicked |= self.retry_guarded(lanes);
        }
        if self
            .snapshot_due(lanes)
            .is_some_and(|due| Instant::now() >= due)
        {
            panicked |= guarded(|| self.write_snapshot(lanes)).is_none();
        }
        panicked
    }

    /// The parked seals retried; one whose retry panicked stays parked, at
    /// the head, and the lane backs off as after a refused retry.
    fn retry_guarded(&mut self, lanes: &mut Lanes) -> bool {
        if guarded(|| self.retry_parked(lanes)).is_some() {
            return false;
        }
        self.seal_retry_at = Instant::now() + SEAL_RETRY_DELAY;
        true
    }

    /// A seal abandoned by a panic is said as a refused one: its audio is
    /// owed, in memory, and retried.
    fn seal_abandoned(&mut self, segment: &crate::capture::Segment, owed: usize) {
        let message = format!(
            "Audio pending in memory: {owed} chunks. Keep Orbit open; local saving will be retried. The local writer failed to save chunk {} and continues with other work.",
            segment.metadata.sequence
        );
        writer::push_completion(
            &self.completions,
            StoreCompletion::SealFailed {
                message: message.clone(),
                cause: WRITER_PANIC_CAUSE,
                owed,
            },
        );
        self.report_problem(message);
    }
}

/// Seals a test asks to panic on their first attempt, by chunk id: ids are
/// unique to each test's recording, so tests running at once do not meet.
/// Nothing in a build without tests looks for them.
#[cfg(test)]
pub(in crate::daemon) mod faults {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, PoisonError};

    static SEALS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

    pub(in crate::daemon) fn panic_on_first_seal(id: &str) {
        SEALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned());
    }

    #[allow(clippy::panic, reason = "a test injects this panic on purpose")]
    pub(super) fn seal(id: &str) {
        if SEALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
        {
            panic!("injected seal panic");
        }
    }
}
