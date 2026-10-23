//! Dedicated owner for the daemon's latency-sensitive durable writes.
//!
//! The Tokio actor only admits bounded operations. The writer thread owns
//! SQLite mutations and filesystem synchronization. Durable completions
//! return through the daemon's ordinary work-event channel.
use super::{WorkResult, native};
use anyhow::Result;
use orbit_core::State;
use orbit_store::{Job, ReadOnlyStore, Store};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// The existing 8192 slots bound memory for admitted writer operations while
/// a disk call stalls. A measurement with two tracks recording gave 94 daemon
/// turns/s; even if every turn admitted one write, this holds about 87 s
/// (8192 / 94) of turns before refusal. A load test of 1000 writes plus 80
/// seals completed with no admission refusal under load average 54.55. The
/// bound is not a durability deadline, a limit on saved recordings, or a
/// capacity shared with readers.
const STORE_OP_CAPACITY: usize = 8192;

pub(super) fn open_owner(directory: &std::path::Path, key: [u8; 32]) -> Result<Store> {
    Ok(Store::open(directory, key)?)
}

pub(super) fn open_reader(directory: &std::path::Path, key: [u8; 32]) -> Result<ReadOnlyStore> {
    Ok(Store::open_reader(directory, key)?)
}

fn queue_full() -> anyhow::Error {
    anyhow::anyhow!(
        "La cola de almacenamiento de {STORE_OP_CAPACITY} operaciones está llena; la escritura no se admitió"
    )
}

fn admission_error<T>(error: mpsc::TrySendError<T>) -> anyhow::Error {
    match error {
        mpsc::TrySendError::Full(_) => queue_full(),
        mpsc::TrySendError::Disconnected(_) => {
            anyhow::anyhow!("El actor de almacenamiento se desconectó; la escritura no se admitió")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod admission_tests {
    use super::*;

    #[test]
    fn a_full_queue_and_a_stopped_writer_have_distinct_causes() {
        let full = admission_error(mpsc::TrySendError::Full(()));
        assert!(full.to_string().contains("8192 operaciones"));
        assert!(full.to_string().contains("llena"));
        let stopped = admission_error(mpsc::TrySendError::Disconnected(()));
        assert!(stopped.to_string().contains("desconectó"));
        assert!(!stopped.to_string().contains("llena"));
    }
}
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(250);
/// One cadence shared by both sides of a failed seal: this
/// actor's parked-seal retry, and the daemon's `sealing` gate that decides
/// when maintenance re-dispatches failed audio and retries a stopped flush.
/// Both describe the same promise to the person — «se reintentan cada 5 s»
/// is printed from this value (see `owed_audio`) — so they are one constant:
/// a second copy could drift and the announced cadence would lie. What it
/// protects is the rate at which a disk that keeps refusing is re-polled
/// while its audio stays owed in memory. The 5 s is the cadence both sides
/// have shared since the sealing lanes existed (tests run the production
/// value too); it is not derived from a disk-recovery measurement,
/// and it is not a durability deadline: nothing waits on it synchronously,
/// and changing it changes the announced sentence and both gates together.
pub(super) const SEAL_RETRY_DELAY: Duration = Duration::from_secs(5);
/// How often the writer retries a `StoreProblem` notice the full
/// work channel could not take. It matches `SNAPSHOT_INTERVAL`, the fastest
/// producer of problems (a refused snapshot retries every 250 ms), so a
/// pending notice is retried at least as often as new ones appear. At worst
/// this wakes an otherwise idle writer four times a second while notices
/// are pending — nothing like the millions of turns of the busy loop
/// described at `snapshot_due`. It
/// is not a durability deadline: the notice is already recorded; this only
/// hands it over.
const PENDING_PROBLEM_RETRY: Duration = Duration::from_millis(250);
/// Bound of `StoreProblem` notices waiting for room in the work
/// channel. What it protects: the writer's memory while the daemon's loop
/// is stalled — 128 bounded taxonomy messages stay well under 64 KiB. At
/// the worst sustained failure rate (one refused snapshot per 250 ms plus
/// one refused seal retry per 5 s, 4.2/s) that size holds ~30 s of stall
/// (128 / 4.2 ≈ 30.5). That window is this bound's consequence, not its
/// goal: the ten-second closing budget bounds the exit path, not this
/// queue, and a daemon loop draining slower than its writer fails can
/// outlive it — the triple margin keeps a stall of that order from
/// reaching the drop path at all. What it is not: it is not a rate limit
/// on reports and not a channel capacity — past it, the count of the
/// notices that did not fit travels with the newest delivered one, so the
/// loss is still said with its number.
const MAX_PENDING_PROBLEMS: usize = 128;
const ACTOR_HANDOFF_GRACE: Duration = Duration::from_millis(25);
type StoreRun = Box<dyn FnOnce(&mut Store) -> bool + Send>;

pub(super) enum StoreOp {
    Run(StoreRun),
    Seal {
        segment: Arc<crate::capture::Segment>,
        admitted_at: Instant,
    },
    RetrySeals,
    Snapshot {
        state: Box<State>,
        receipt: Option<tokio::sync::oneshot::Sender<std::result::Result<(), String>>>,
    },
    SnapshotWithJob {
        state: Box<State>,
        job: Job,
        receipt: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
    CriticalSnapshot {
        state: Box<State>,
        receipt: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
    StartupStop {
        state: Box<State>,
        receipt: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
}

pub(super) struct StoreActor {
    sender: Option<mpsc::SyncSender<StoreOp>>,
    startup_stop_sender: Arc<Mutex<Option<mpsc::SyncSender<StoreOp>>>>,
    dispatched_seals: BTreeSet<String>,
    completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    thread: Option<thread::JoinHandle<()>>,
    startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    /// Where the loop's waits on this writer are reported, by call site.
    waits: super::turn_watch::TurnWatch,
}

#[derive(Clone)]
pub(super) struct StartupStopWriter {
    sender: Weak<Mutex<Option<mpsc::SyncSender<StoreOp>>>>,
}

impl StartupStopWriter {
    pub(super) async fn persist(&self, state: State) -> Result<()> {
        let (receipt, response) = tokio::sync::oneshot::channel();
        let sender = self
            .sender
            .upgrade()
            .ok_or(super::startup::StartupError::StopWriterStopped)?;
        {
            let sender = sender
                .lock()
                .map_err(|_| super::startup::StartupError::StopWriterLockPoisoned)?;
            let sender = sender
                .as_ref()
                .ok_or(super::startup::StartupError::StopWriterStopped)?;
            sender
                .try_send(StoreOp::StartupStop {
                    state: Box::new(state),
                    receipt,
                })
                .map_err(admission_error)?;
        }
        receive(response).await?.map_err(anyhow::Error::msg)
    }
}

pub(super) enum StoreCompletion {
    Sealed {
        id: String,
        recording: String,
        sequence: u64,
        #[allow(
            dead_code,
            reason = "the queue wait is consumed by the measurement tests"
        )]
        queue_wait_us: Option<u64>,
    },
    SealFailed {
        message: String,
    },
}

/// Receipt for an operation admitted synchronously to the ordered writer.
///
/// Admission and waiting are separate so the daemon can preserve admission
/// order in its current turn, then let a worker wait for disk without holding IPC.
pub(super) struct StoreReceipt<T> {
    response: tokio::sync::oneshot::Receiver<Result<T>>,
}

/// A failed operation says whether SQLite saw it change anything: when it
/// did, the outcome of those changes is uncertain and must not be retried
/// blindly.
fn uncertain<T>(result: Result<T>, changed: bool) -> Result<T> {
    result.map_err(|error| {
        let context = format!(
            "Operación local fallida: cambios SQLite observados {changed}; el resultado de esos cambios es incierto; causa: {error:#}"
        );
        error.context(context)
    })
}

impl<T> StoreReceipt<T> {
    pub(super) async fn wait(self) -> Result<T> {
        receive(self.response).await?
    }
}

impl StoreActor {
    /// Before recovery creates the actor thread, its module owns the one
    /// bootstrap connection and transfers it into that thread on spawn.
    pub(super) fn open_bootstrap(directory: &std::path::Path, key: [u8; 32]) -> Result<Store> {
        open_owner(directory, key)
    }

    pub(super) fn runtime_reader(
        &self,
        directory: &std::path::Path,
        key: [u8; 32],
    ) -> Result<ReadOnlyStore> {
        open_reader(directory, key)
    }

    #[cfg(test)]
    pub(super) fn spawn(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
    ) -> Result<Self> {
        Self::spawn_with_reporter(store, events, Arc::default(), None)
    }

    pub(super) fn spawn_with_diagnostics(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        diagnostics: crate::diagnostics::DiagnosticLog,
    ) -> Result<Self> {
        Self::spawn_with_reporter(store, events, Arc::default(), Some(diagnostics))
    }

    /// The same actor, with its loop turns counted into `turns`.
    ///
    /// The loop always counts; only whoever holds `turns` reads it.
    /// Whether the writer sleeps between real deadlines is a count of turns
    /// against the operations and write attempts that caused them, never a
    /// reading of a clock, so the proof needs the count itself.
    #[cfg(test)]
    pub(super) fn spawn_counting(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        turns: Arc<AtomicU64>,
    ) -> Result<Self> {
        Self::spawn_with_reporter(store, events, turns, None)
    }

    fn spawn_with_reporter(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        turns: Arc<AtomicU64>,
        diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(STORE_OP_CAPACITY);
        let completions = Arc::new(Mutex::new(VecDeque::new()));
        let thread_completions = Arc::clone(&completions);
        let startup_guard_active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let thread_guard = Arc::clone(&startup_guard_active);
        let startup_stop_sender = Arc::new(Mutex::new(Some(sender.clone())));
        let thread = thread::Builder::new()
            .name("orbit-store".into())
            .spawn(move || {
                run(
                    store,
                    receiver,
                    events,
                    thread_completions,
                    &turns,
                    thread_guard,
                    diagnostics,
                )
            })?;
        Ok(Self {
            sender: Some(sender),
            startup_stop_sender,
            dispatched_seals: BTreeSet::new(),
            completions,
            thread: Some(thread),
            startup_guard_active,
            waits: super::turn_watch::TurnWatch::default(),
        })
    }

    pub(super) fn startup_stop_writer(&self) -> Result<StartupStopWriter> {
        Ok(StartupStopWriter {
            sender: Arc::downgrade(&self.startup_stop_sender),
        })
    }

    pub(super) async fn finish_startup(&self) -> Result<()> {
        let guard = Arc::clone(&self.startup_guard_active);
        self.run(move |_store| {
            guard.store(false, Ordering::Release);
            Ok(())
        })
        .await
    }

    /// The closing budget expired while the writer may be inside a blocked
    /// system call. Process exit cannot join that thread; the final notice has
    /// already declared the writes still in memory. Ordinary drops still join.
    pub(super) fn detach_for_bounded_exit(&mut self) {
        self.thread.take();
    }

    pub(super) fn bounded_exit_declared(&self) -> bool {
        self.thread.is_none()
    }

    pub(super) fn seal(&mut self, segment: Arc<crate::capture::Segment>) -> Result<()> {
        let id = segment.metadata.id.clone();
        if self.dispatched_seals.contains(&id) {
            return Ok(());
        }
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("El actor de almacenamiento se detuvo"))?;
        sender
            .try_send(StoreOp::Seal {
                segment,
                admitted_at: Instant::now(),
            })
            .map_err(admission_error)?;
        self.dispatched_seals.insert(id);
        Ok(())
    }

    pub(super) fn acknowledge_seal(&mut self, id: &str) {
        self.dispatched_seals.remove(id);
    }

    pub(super) fn retry_seals(&self) -> Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("El actor de almacenamiento se detuvo"))?
            .try_send(StoreOp::RetrySeals)
            .map_err(admission_error)?;
        Ok(())
    }

    pub(super) fn take_completions(&self) -> Vec<StoreCompletion> {
        let Ok(mut completions) = self.completions.lock() else {
            return Vec::new();
        };
        completions.drain(..).collect()
    }

    fn admit_snapshot(&self, operation: StoreOp) -> Result<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("El actor de almacenamiento se detuvo"))?;
        sender.try_send(operation).map_err(admission_error)
    }

    pub(super) fn defer_snapshot(&self, state: &State) -> Result<()> {
        self.admit_snapshot(StoreOp::Snapshot {
            state: Box::new(state.clone()),
            receipt: None,
        })?;
        Ok(())
    }

    /// Report every wait on this writer to `watch`, the loop's turns.
    pub(super) fn report_waits_to(&mut self, watch: super::turn_watch::TurnWatch) {
        self.waits = watch;
    }

    /// Awaits `wait` and reports how long it took, named by `at`.
    async fn timed<T>(
        &self,
        at: &'static std::panic::Location<'static>,
        wait: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let started = Instant::now();
        let result = wait.await;
        self.waits.store_waited(at, started.elapsed());
        result
    }

    #[track_caller]
    pub(super) fn snapshot<'a>(
        &'a self,
        state: &'a State,
    ) -> impl std::future::Future<Output = Result<()>> + 'a {
        let at = std::panic::Location::caller();
        self.timed(at, async move {
            let (receipt, response) = tokio::sync::oneshot::channel();
            self.admit_snapshot(StoreOp::Snapshot {
                state: Box::new(state.clone()),
                receipt: Some(receipt),
            })?;
            receive(response).await?.map_err(anyhow::Error::msg)
        })
    }

    /// An ordered, non-coalesced snapshot with its own durable receipt. Exit
    /// declarations use this lane so the 250 ms coalescing interval cannot
    /// leave their loss notice unwritten when the process detaches the writer.
    #[track_caller]
    pub(super) fn critical_snapshot<'a>(
        &'a self,
        state: &'a State,
    ) -> impl std::future::Future<Output = Result<()>> + 'a {
        let at = std::panic::Location::caller();
        self.timed(at, async move {
            let (receipt, response) = tokio::sync::oneshot::channel();
            self.admit_snapshot(StoreOp::CriticalSnapshot {
                state: Box::new(state.clone()),
                receipt,
            })?;
            receive(response).await?.map_err(anyhow::Error::msg)
        })
    }

    #[track_caller]
    pub(super) fn snapshot_with_job<'a>(
        &'a self,
        state: &'a State,
        job: Job,
    ) -> impl std::future::Future<Output = Result<()>> + 'a {
        let at = std::panic::Location::caller();
        self.timed(at, async move {
            let (receipt, response) = tokio::sync::oneshot::channel();
            self.admit_snapshot(StoreOp::SnapshotWithJob {
                state: Box::new(state.clone()),
                job,
                receipt,
            })?;
            receive(response).await?.map_err(anyhow::Error::msg)
        })
    }

    /// Run one typed store operation on the dedicated writer connection.
    ///
    /// The type erasure lives only in the queued closure: its oneshot keeps the
    /// caller's result type, including domain admissions and durable receipts.
    ///
    /// The wait is reported to the loop's turn under the caller's call site.
    #[track_caller]
    pub(super) fn run<T, F>(
        &self,
        operation: F,
    ) -> impl std::future::Future<Output = Result<T>> + '_
    where
        T: Send + 'static,
        F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
    {
        let at = std::panic::Location::caller();
        self.timed(at, async move { self.begin_run(operation)?.wait().await })
    }

    /// Admit one typed operation now and return its future durable receipt.
    /// Unlike [`Self::run`], this never waits for the writer.
    pub(super) fn begin_run<T, F>(&self, operation: F) -> Result<StoreReceipt<T>>
    where
        T: Send + 'static,
        F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
    {
        let (receipt, response) = tokio::sync::oneshot::channel();
        let operation = Box::new(move |store: &mut Store| {
            let before = store.mutation_count();
            let result = operation(store);
            let changed = store.mutation_count() != before;
            receipt.send(uncertain(result, changed)).is_err()
        });
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("El actor de almacenamiento se detuvo"))?;
        sender
            .try_send(StoreOp::Run(operation))
            .map_err(admission_error)?;
        Ok(StoreReceipt { response })
    }
}

async fn receive<T>(mut response: tokio::sync::oneshot::Receiver<T>) -> Result<T> {
    // Tokio's paused clock cannot observe work performed by this std thread.
    // Give the writer a short cooperative handoff before awaiting normally so
    // virtual-time deadline tests do not skip over an already-running fsync.
    let handoff_until = Instant::now() + ACTOR_HANDOFF_GRACE;
    loop {
        match response.try_recv() {
            Ok(value) => return Ok(value),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                return Err(anyhow::anyhow!(
                    "El actor de almacenamiento perdió el recibo"
                ));
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                if Instant::now() < handoff_until =>
            {
                tokio::task::yield_now().await;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                return response
                    .await
                    .map_err(|_| anyhow::anyhow!("El actor de almacenamiento perdió el recibo"));
            }
        }
    }
}

impl Drop for StoreActor {
    fn drop(&mut self) {
        match self.startup_stop_sender.lock() {
            Ok(mut sender) => {
                sender.take();
            }
            Err(poisoned) => {
                poisoned.into_inner().take();
            }
        }
        self.sender.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("The store thread panicked while closing");
        }
    }
}

/// Position of an operation in the order the daemon handed it over.
type Ticket = u64;

struct Queued<T> {
    ticket: Ticket,
    operation: T,
}

struct PendingSnapshot {
    /// The oldest admission this pending write still owes. Coalescing keeps
    /// only the newest state, and writing it pays every earlier admission.
    since: Ticket,
    state: Box<State>,
    receipts: Vec<tokio::sync::oneshot::Sender<std::result::Result<(), String>>>,
}

struct CriticalSnapshot {
    state: Box<State>,
    job: Option<Job>,
    receipt: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    startup_stop: bool,
}

/// A seal whose first attempt failed, awaiting its retry.
struct Parked {
    segment: Arc<crate::capture::Segment>,
    /// When that first attempt failed: since when its audio has been owed.
    failed_at: Instant,
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
struct Lanes {
    admitted: Ticket,
    runs: VecDeque<Queued<StoreRun>>,
    /// Seals never attempted, in admission order.
    seals: VecDeque<Queued<(Arc<crate::capture::Segment>, Instant)>>,
    /// Seals attempted and failed, awaiting a retry. Already executed once.
    parked: VecDeque<Parked>,
    /// The oldest `RetrySeals` not yet served.
    retry: Option<Ticket>,
    snapshot: Option<PendingSnapshot>,
    critical: VecDeque<Queued<CriticalSnapshot>>,
}

#[derive(Clone, Copy)]
enum Step {
    Seal,
    Retry,
    Flush,
    Run,
    Critical,
    Idle,
}

impl Lanes {
    fn admit(&mut self, operation: StoreOp) {
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
    fn next_step(&self) -> Step {
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
    fn ready(&self) -> bool {
        !self.seals.is_empty()
            || !self.runs.is_empty()
            || !self.critical.is_empty()
            || self.retry.is_some()
    }

    /// Seals the actor still owns, counted as the notice reports them.
    fn owed_seals(&self) -> usize {
        self.seals.len().saturating_add(self.parked.len())
    }

    /// What a refusal adds while seals are parked: how many fragments
    /// the actor still owes, and since when, counted from the moment the
    /// oldest parked one first failed. Nothing is added while none has failed:
    /// a seal still queued is not a problem yet.
    fn owed_audio(&self, now: Instant) -> Option<String> {
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
fn refusal(error: &impl std::fmt::Display, lanes: &Lanes) -> String {
    let cause = format!("No se pudo guardar el estado local: {error}");
    match lanes.owed_audio(Instant::now()) {
        Some(owed) => format!("{cause}. {owed}."),
        None => cause,
    }
}

struct Writer {
    store: Store,
    events: tokio::sync::mpsc::Sender<WorkResult>,
    completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    seal_retry_at: Instant,
    last_snapshot: Option<Instant>,
    held_startup_stop: Option<Box<State>>,
    startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    /// `StoreProblem` notices the full work channel refused, oldest
    /// first. They are delivered as soon as the daemon's loop makes room;
    /// none of them is the `Sealed` wake-up, whose receipt already lives in
    /// the completion log.
    pending_problems: VecDeque<String>,
    /// Notices dropped after `MAX_PENDING_PROBLEMS`: the newest delivered
    /// notice carries this count, so even the bounded loss is said — and it
    /// is said in the message the daemon's single notice slot keeps, not in
    /// one the rest of the burst would overwrite.
    overflowed_problems: u64,
}

impl Writer {
    fn report_receipt_loss(&mut self, operation: &str) {
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
    fn wake_daemon(&self, event: WorkResult) {
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
    fn report_problem(&mut self, message: String) {
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
    fn deliver_pending_problems(&mut self) {
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
    fn problem_flush_due(&self) -> Option<Instant> {
        (!self.pending_problems.is_empty()).then_some(Instant::now() + PENDING_PROBLEM_RETRY)
    }

    /// One durable attempt at a seal. `owed` counts every seal still in the
    /// actor's hands, this one included, for the notice a failure leaves.
    fn seal(
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
                    },
                );
                self.report_problem(message);
                false
            }
        }
    }

    /// Retries parked seals oldest first and stops at the first that fails
    /// again, which backs the whole retry lane off, as it always did.
    fn retry_parked(&mut self, lanes: &mut Lanes) {
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
    fn retry_due(&self, lanes: &Lanes) -> Option<Instant> {
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
    fn snapshot_due(&self, lanes: &Lanes) -> Option<Instant> {
        if lanes.snapshot.is_none() || !lanes.seals.is_empty() || !lanes.runs.is_empty() {
            return None;
        }
        Some(
            self.last_snapshot
                .map_or_else(Instant::now, |written| written + SNAPSHOT_INTERVAL),
        )
    }

    fn write_snapshot(&mut self, lanes: &mut Lanes) {
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

    fn write_critical(&mut self, lanes: &mut Lanes, critical: CriticalSnapshot) {
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

fn run(
    store: Store,
    receiver: mpsc::Receiver<StoreOp>,
    events: tokio::sync::mpsc::Sender<WorkResult>,
    completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    turns: &AtomicU64,
    startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    diagnostics: Option<crate::diagnostics::DiagnosticLog>,
) {
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
        match step {
            Step::Seal => {
                if let Some(queued) = lanes.seals.pop_front() {
                    let owed = lanes.owed_seals().saturating_add(1);
                    let queue_wait_us = Instant::now()
                        .duration_since(queued.operation.1)
                        .as_micros()
                        .min(u128::from(u64::MAX)) as u64;
                    let sealed = writer.seal(&queued.operation.0, owed, Some(queue_wait_us));
                    if !sealed {
                        let failed_at = Instant::now();
                        if lanes.parked.is_empty() {
                            writer.seal_retry_at = failed_at + SEAL_RETRY_DELAY;
                        }
                        lanes.parked.push_back(Parked {
                            segment: queued.operation.0,
                            failed_at,
                        });
                    }
                }
            }
            Step::Retry => {
                lanes.retry = None;
                writer.retry_parked(&mut lanes);
            }
            Step::Flush => writer.write_snapshot(&mut lanes),
            Step::Run => {
                if let Some(queued) = lanes.runs.pop_front() {
                    let receipt_lost = (queued.operation)(&mut writer.store);
                    if receipt_lost {
                        writer.report_receipt_loss("run");
                    }
                }
            }
            Step::Critical => {
                if let Some(queued) = lanes.critical.pop_front() {
                    writer.write_critical(&mut lanes, queued.operation);
                }
            }
            Step::Idle => {}
        }

        // Audio first: a retry that has fallen due runs before a snapshot due
        // in the same turn.
        if writer
            .retry_due(&lanes)
            .is_some_and(|due| Instant::now() >= due)
        {
            writer.retry_parked(&mut lanes);
        }
        if writer
            .snapshot_due(&lanes)
            .is_some_and(|due| Instant::now() >= due)
        {
            writer.write_snapshot(&mut lanes);
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

fn push_completion(completions: &Mutex<VecDeque<StoreCompletion>>, completion: StoreCompletion) {
    if let Ok(mut completions) = completions.lock() {
        completions.push_back(completion);
    }
}

#[cfg(test)]
mod pending_problem_tests {
    use super::*;
    use crate::daemon::StoreWork;

    /// The daemon keeps a single notice slot, so when a burst
    /// of pending problems is finally delivered, each newer message
    /// overwrites the previous one in that slot. The count of dropped
    /// notices must therefore travel with the NEWEST delivered message —
    /// the one the slot ends up holding. An earlier version folded it into the
    /// oldest, which the rest of the burst overwrote before anyone could read it.
    #[tokio::test]
    async fn the_overflow_count_travels_in_the_newest_delivered_notice() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path(), [7u8; 32])?;
        let (events, mut receiver) = tokio::sync::mpsc::channel(1);
        events
            .send(WorkResult::Problem {
                ticket: crate::daemon::ticketed::WorkTicket::test(0),
                message: "occupant".into(),
            })
            .await
            .map_err(|_| anyhow::anyhow!("the occupying event could not be queued"))?;
        let mut writer = Writer {
            store,
            events,
            completions: Arc::new(Mutex::new(VecDeque::new())),
            seal_retry_at: Instant::now(),
            last_snapshot: None,
            held_startup_stop: None,
            startup_guard_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            diagnostics: None,
            pending_problems: VecDeque::new(),
            overflowed_problems: 0,
        };
        // Two notices past the ceiling while the channel is full: the two
        // oldest are dropped and counted.
        for index in 0..(MAX_PENDING_PROBLEMS as u64 + 2) {
            writer.report_problem(format!("synthetic notice {index}"));
        }
        let dropped = writer.overflowed_problems;
        assert_eq!(dropped, 2, "the fixture must push past the ceiling");
        // Free the slot and let the writer drain, one slot at a time, the
        // way its loop retries each turn.
        assert!(matches!(
            receiver.recv().await,
            Some(WorkResult::Problem { .. })
        ));
        let mut delivered = Vec::new();
        while !writer.pending_problems.is_empty() {
            writer.deliver_pending_problems();
            while let Ok(event) = receiver.try_recv() {
                if let WorkResult::Store(StoreWork::StoreProblem { message }) = event {
                    delivered.push(message);
                }
            }
        }
        assert_eq!(
            delivered.len(),
            MAX_PENDING_PROBLEMS,
            "every notice the bound kept must be delivered"
        );
        let last = delivered
            .last()
            .map(|message| message.as_str())
            .unwrap_or_default();
        assert!(
            last.contains(&format!("Además, {dropped} aviso(s)")),
            "the dropped count must arrive in the newest notice, the one the \
             daemon's single slot keeps: {last}"
        );
        Ok(())
    }
}
