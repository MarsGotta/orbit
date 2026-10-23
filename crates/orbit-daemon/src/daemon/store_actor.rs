//! Dedicated owner for the daemon's latency-sensitive durable writes.
//!
//! The Tokio actor only admits bounded operations. The writer thread owns
//! SQLite mutations and filesystem synchronization. Durable completions
//! return through the daemon's ordinary work-event channel.
use super::{WorkResult, native};
use crate::supervision::Supervision;
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

mod capture_port;
mod lanes;
mod receipts;
mod writer;
mod writer_loop;
pub(super) use capture_port::CapturePort;
pub(super) use capture_port::StoreCompletion;
use lanes::{CriticalSnapshot, Lanes, Parked, PendingSnapshot, Step, refusal};
pub(super) use receipts::{StartupStopWriter, StoreReceipt};
use receipts::{admission_error, receive, uncertain};
use writer::Writer;
#[cfg(test)]
pub(in crate::daemon) use writer_loop::{WRITER_PANIC_CAUSE, faults};

pub(super) fn open_owner(directory: &std::path::Path, key: [u8; 32]) -> Result<Store> {
    Ok(Store::open(directory, key)?)
}

pub(super) fn open_reader(directory: &std::path::Path, key: [u8; 32]) -> Result<ReadOnlyStore> {
    Ok(Store::open_reader(directory, key)?)
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
    /// The seal admission the capture lane shares with this actor.
    seals: CapturePort,
    thread: Option<thread::JoinHandle<()>>,
    startup_guard_active: Arc<std::sync::atomic::AtomicBool>,
    /// Where the loop's waits on this writer are reported, by call site.
    waits: super::turn_watch::TurnWatch,
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
        Self::spawn_supervised(store, events, Supervision::default())
    }

    /// The same, its panics counted in `supervision`.
    #[cfg(test)]
    pub(super) fn spawn_supervised(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        supervision: Supervision,
    ) -> Result<Self> {
        Self::spawn_with_reporter(store, events, Arc::default(), None, supervision)
    }

    pub(super) fn spawn_with_diagnostics(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        diagnostics: crate::diagnostics::DiagnosticLog,
        supervision: Supervision,
    ) -> Result<Self> {
        Self::spawn_with_reporter(
            store,
            events,
            Arc::default(),
            Some(diagnostics),
            supervision,
        )
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
        Self::spawn_with_reporter(store, events, turns, None, Supervision::default())
    }

    fn spawn_with_reporter(
        store: Store,
        events: tokio::sync::mpsc::Sender<WorkResult>,
        turns: Arc<AtomicU64>,
        diagnostics: Option<crate::diagnostics::DiagnosticLog>,
        supervision: Supervision,
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
                writer_loop::run(writer_loop::WriterParts {
                    store,
                    receiver,
                    events,
                    completions: thread_completions,
                    turns,
                    startup_guard_active: thread_guard,
                    diagnostics,
                    supervision,
                })
            })?;
        Ok(Self {
            seals: CapturePort::new(sender.clone(), completions),
            sender: Some(sender),
            startup_stop_sender,
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
        self.seals.begin_run(operation)
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
        self.seals.close();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("The store thread panicked while closing");
        }
    }
}

#[cfg(test)]
mod supervision_tests;

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
