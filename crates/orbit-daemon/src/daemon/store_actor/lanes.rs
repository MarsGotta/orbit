//! The writer's ordered lanes and the one rule that picks what runs next.
use super::*;

/// Position of an operation in the order the daemon handed it over.
pub(super) type Ticket = u64;

pub(super) struct Queued<T> {
    pub(super) ticket: Ticket,
    pub(super) operation: T,
}

pub(super) struct PendingSnapshot {
    /// The oldest admission this pending write still owes. Coalescing keeps
    /// only the newest state, and writing it pays every earlier admission.
    pub(super) since: Ticket,
    pub(super) state: Box<State>,
    pub(super) receipts: Vec<tokio::sync::oneshot::Sender<std::result::Result<(), String>>>,
}

pub(super) struct CriticalSnapshot {
    pub(super) state: Box<State>,
    pub(super) job: Option<Job>,
    pub(super) receipt: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    pub(super) startup_stop: bool,
}

/// A seal whose first attempt failed, awaiting its retry.
pub(super) struct Parked {
    pub(super) segment: Arc<crate::capture::Segment>,
    /// When that first attempt failed: since when its audio has been owed.
    pub(super) failed_at: Instant,
}

/// The store actor executes operations in the order it was handed
/// them. Each lane below keeps its own reason to exist, and one rule orders
/// them: a `Run`, a seal, a requested retry or a critical snapshot runs only
/// when every one of those admitted before it has been executed, and a `Run`
/// also waits for a coalesced snapshot admitted before it. `Lanes::next_step`
/// is that rule, and it is the only place that picks what runs next.
///
/// Why lanes at all. A seal that fails is not dropped: it is parked and
/// retried after `SEAL_RETRY_DELAY` or when the daemon asks (`RetrySeals`).
/// Its first attempt keeps its place in the order; its retries do not, because
/// a `Run` is awaited without a deadline and a later one waiting on a broken
/// disk would stop the whole daemon. So a parked seal has been executed, its
/// failure is already reported (`SealFailed`) before any later operation runs,
/// and a later operation proceeds. The retry lane backs off as it always did:
/// a failed retry waits `SEAL_RETRY_DELAY` again, and a new seal that fails on
/// its first attempt joins the lane without postponing the retry already due.
///
/// A coalesced snapshot is the one deliberate exception, and only towards
/// seals: audio goes ahead of it, and it may wait for the interval, but a
/// later `Run` forces it out first (`Step::Flush`). A critical snapshot
/// supersedes a pending one by writing the newer state, as before.
///
/// A parked seal does not hold a coalesced snapshot either, for the
/// same reason it does not hold a `Run`: `snapshot().await` has no deadline
/// either, and the privacy stop and the closure both wait on one. Held behind
/// a seal the disk kept refusing, the snapshot was never attempted, so its
/// waiter got no answer for as long as the disk stayed broken, and the daemon
/// with it. Audio still goes first whenever it can run: a seal not yet
/// attempted keeps the snapshot waiting, and a retry that falls due in the
/// same turn runs before it (see `Writer::snapshot_due`).
///
/// Before this rule each loop turn ran a queued `Run` before the queued seal, so
/// a `Run` admitted after a `Seal` usually ran first. That is what cancelled
/// voice turns hit: the seal creates the upload job the next `Run` cancels.
#[derive(Default)]
pub(super) struct Lanes {
    pub(super) admitted: Ticket,
    pub(super) runs: VecDeque<Queued<StoreRun>>,
    /// Seals never attempted, in admission order.
    pub(super) seals: VecDeque<Queued<(Arc<crate::capture::Segment>, Instant)>>,
    /// Seals attempted and failed, awaiting a retry. Already executed once.
    pub(super) parked: VecDeque<Parked>,
    /// The oldest `RetrySeals` not yet served.
    pub(super) retry: Option<Ticket>,
    pub(super) snapshot: Option<PendingSnapshot>,
    pub(super) critical: VecDeque<Queued<CriticalSnapshot>>,
}

#[derive(Clone, Copy)]
pub(super) enum Step {
    Seal,
    Retry,
    Flush,
    Run,
    Critical,
    Idle,
}

impl Lanes {
    pub(super) fn admit(&mut self, operation: StoreOp) {
        let ticket = self.admitted;
        self.admitted = self.admitted.saturating_add(1);
        match operation {
            StoreOp::Run(operation) => self.runs.push_back(Queued { ticket, operation }),
            StoreOp::Seal {
                segment,
                admitted_at,
            } => self.seals.push_back(Queued {
                ticket,
                operation: (segment, admitted_at),
            }),
            StoreOp::RetrySeals => {
                self.retry.get_or_insert(ticket);
            }
            StoreOp::Snapshot { state, receipt } => {
                let (since, mut receipts) = self
                    .snapshot
                    .take()
                    .map(|pending| (pending.since, pending.receipts))
                    .unwrap_or((ticket, Vec::new()));
                if let Some(receipt) = receipt {
                    receipts.push(receipt);
                }
                self.snapshot = Some(PendingSnapshot {
                    since,
                    state,
                    receipts,
                });
            }
            StoreOp::SnapshotWithJob {
                state,
                job,
                receipt,
            } => self.critical.push_back(Queued {
                ticket,
                operation: CriticalSnapshot {
                    state,
                    job: Some(job),
                    receipt,
                    startup_stop: false,
                },
            }),
            StoreOp::CriticalSnapshot { state, receipt } => self.critical.push_back(Queued {
                ticket,
                operation: CriticalSnapshot {
                    state,
                    job: None,
                    receipt,
                    startup_stop: false,
                },
            }),
            StoreOp::StartupStop { state, receipt } => self.critical.push_back(Queued {
                ticket,
                operation: CriticalSnapshot {
                    state,
                    job: None,
                    receipt,
                    startup_stop: true,
                },
            }),
        }
    }

    /// The oldest operation still owed across the ordered lanes. A `Run` whose
    /// turn has come first flushes a snapshot admitted before it.
    pub(super) fn next_step(&self) -> Step {
        let seal = self.seals.front().map(|queued| queued.ticket);
        let run = self.runs.front().map(|queued| queued.ticket);
        let critical = self.critical.front().map(|queued| queued.ticket);
        let Some(oldest) = [seal, self.retry, run, critical]
            .into_iter()
            .flatten()
            .min()
        else {
            return Step::Idle;
        };
        if Some(oldest) == seal {
            Step::Seal
        } else if Some(oldest) == self.retry {
            Step::Retry
        } else if Some(oldest) == run {
            if self
                .snapshot
                .as_ref()
                .is_some_and(|pending| pending.since < oldest)
            {
                Step::Flush
            } else {
                Step::Run
            }
        } else {
            Step::Critical
        }
    }

    /// Work that is ready now, so the actor must not sleep before doing it.
    pub(super) fn ready(&self) -> bool {
        !self.seals.is_empty()
            || !self.runs.is_empty()
            || !self.critical.is_empty()
            || self.retry.is_some()
    }

    /// Seals the actor still owns, counted as the notice reports them.
    pub(super) fn owed_seals(&self) -> usize {
        self.seals.len().saturating_add(self.parked.len())
    }

    /// What a refusal adds while seals are parked: how many fragments
    /// the actor still owes, and since when, counted from the moment the
    /// oldest parked one first failed. Nothing is added while none has failed:
    /// a seal still queued is not a problem yet.
    pub(super) fn owed_audio(&self, now: Instant) -> Option<String> {
        let oldest = self.parked.front()?;
        let owed_for = now.saturating_duration_since(oldest.failed_at).as_millis();
        Some(format!(
            "{} fragmento(s) de audio siguen sin guardar desde hace {owed_for} ms; se reintentan cada {} s",
            self.owed_seals(),
            SEAL_RETRY_DELAY.as_secs()
        ))
    }
}

/// The answer a snapshot's waiters get when the store refuses it:
/// the store's own cause and, while seals are parked, the audio owed with its
/// numbers. The same text goes to the daemon as a `StoreProblem`, so whoever
/// asked and whoever reads the notice see one cause.
pub(super) fn refusal(error: &impl std::fmt::Display, lanes: &Lanes) -> String {
    let cause = format!("No se pudo guardar el estado local: {error}");
    match lanes.owed_audio(Instant::now()) {
        Some(owed) => format!("{cause}. {owed}."),
        None => cause,
    }
}
