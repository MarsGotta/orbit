//! Supervision per actor. A panic in one actor is caught where that actor
//! runs, logged as `actor_panicked` with the actor's name and its restart
//! count, and that actor alone is relaunched from its durable state; the
//! recording and every other actor go on. Past its budget the panic goes up:
//! the daemon's loop ends with a failure, which is the whole-daemon relaunch
//! the app already makes (`apps/desktop/src-tauri/src/daemon_thread.rs`).
//!
//! The panic's message never reaches the log: it can carry content (a
//! formatted value, a path, a title). The log carries the actor and counts.
//! The process's panic hook is not changed here: it still prints and, if the
//! person allowed error reports, reports the panic where it happened.
use crate::diagnostics::DiagnosticLog;
use futures_util::FutureExt;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

/// How many relaunches one actor gets within [`RESTART_WINDOW`] before its
/// next panic ends the daemon instead.
///
/// What it protects: the meeting being recorded, from a panic that repeats.
/// A panic that came from one bad moment (a race, one malformed input) is
/// gone after a relaunch, and the first relaunch already shows it; a second
/// and a third leave room for one that recurs a few times without a pattern.
/// A panic that comes back a fourth time within the window comes from state
/// the actor rebuilds on every relaunch, so another one would only repeat it:
/// a whole-daemon relaunch rebuilds everything from the store, and the
/// recording resumes from its snapshot as after any crash.
///
/// What it is not: a limit on relaunches over a day. A panic outside the
/// window is counted again from one.
pub const RESTART_BUDGET: usize = 3;

/// The window [`RESTART_BUDGET`] counts in. A minute is the app's own floor
/// for a whole-daemon relaunch (`DAEMON_RELAUNCH_MIN_UPTIME`): with the same
/// window, an actor that panics in a loop ends the daemon at most once a
/// minute, the rate the app already accepts for a daemon that falls. The
/// slowest actor to repeat a panic is the store writer, which retries a
/// failed seal every five seconds (`SEAL_RETRY_DELAY`): four of those fit in
/// twenty seconds, well inside it.
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// Every supervised actor of the daemon, by the name its log line carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Actor {
    /// The capture task: frames, seals and the chunks' upkeep.
    CaptureTask,
    /// One chunk's transfer to the hub, spawned by the capture task.
    ChunkUpload,
    /// The store's ordered writer thread.
    StoreWriter,
    /// The helper's supervision thread, with the helper it launched.
    HelperSupervisor,
    /// The hub's websocket connection.
    HubConnection,
    /// The loop's one transfer worker: a recording's closure, the cleanup of
    /// confirmed audio and the wait for an upload's settlement.
    TransferWorker,
    /// The voice note's transcription worker.
    VoiceTurn,
    /// A reading aloud.
    Playback,
    /// The device's profile for the notes.
    NoteProfile,
    /// One lane of the notes' outbox.
    NoteLane,
    /// A deletion of a recording on this Mac.
    NoteDeletion,
    /// The lookup of the Mac account's name for the notes.
    NoteOwner,
    /// The Mac control engine's work.
    ComputerControl,
}

impl Actor {
    pub fn name(self) -> &'static str {
        match self {
            Self::CaptureTask => "capture_task",
            Self::ChunkUpload => "chunk_upload",
            Self::StoreWriter => "store_writer",
            Self::HelperSupervisor => "helper_supervisor",
            Self::HubConnection => "hub_connection",
            Self::TransferWorker => "transfer_worker",
            Self::VoiceTurn => "voice_turn",
            Self::Playback => "playback",
            Self::NoteProfile => "note_profile",
            Self::NoteLane => "note_lane",
            Self::NoteDeletion => "note_deletion",
            Self::NoteOwner => "note_owner",
            Self::ComputerControl => "computer_control",
        }
    }
}

/// What a caught panic leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The actor is relaunched; `restarts` counts its relaunches so far,
    /// this one included.
    Relaunch { restarts: u64 },
    /// The budget is spent: the actor stays down and the daemon ends.
    GiveUp,
}

/// An actor whose budget ran out: what the daemon's failure says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exhausted {
    pub actor: Actor,
    /// Its panics within the window, the last one included.
    pub panics: usize,
}

/// One actor's panics within the window, oldest first. Pure: the time is
/// given, never read here.
#[derive(Default)]
struct Budget {
    recent: VecDeque<Instant>,
    restarts: u64,
}

impl Budget {
    fn admit(&mut self, now: Instant) -> (Verdict, usize) {
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RESTART_WINDOW)
        {
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        let panics = self.recent.len();
        if panics > RESTART_BUDGET {
            return (Verdict::GiveUp, panics);
        }
        self.restarts = self.restarts.saturating_add(1);
        (
            Verdict::Relaunch {
                restarts: self.restarts,
            },
            panics,
        )
    }
}

#[derive(Default)]
struct Inner {
    budgets: BTreeMap<Actor, Budget>,
    /// Actors that fell and that their owner relaunches when it next looks.
    down: BTreeSet<Actor>,
    /// Panics a test injects into the next run of an actor under
    /// [`Supervision::guard`]. Nothing in a build without tests sets them.
    #[cfg(test)]
    injected: BTreeMap<Actor, u32>,
    exhausted: Option<Exhausted>,
    diagnostics: Option<DiagnosticLog>,
}

/// The daemon's one supervision, shared by every actor and the loop.
#[derive(Clone, Default)]
pub struct Supervision {
    inner: Arc<Mutex<Inner>>,
}

impl Supervision {
    /// Supervision that logs to `diagnostics`; without one (a fixture) it
    /// still counts and decides.
    pub fn new(diagnostics: Option<DiagnosticLog>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                diagnostics,
                ..Inner::default()
            })),
        }
    }

    /// A poisoned lock keeps its counts: the only writers are the short
    /// sections below, which cannot leave them half-changed.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One panic of `actor`, caught at `now`: logged, counted, and whether it
    /// is relaunched or the daemon ends.
    pub fn panicked_at(&self, actor: Actor, now: Instant) -> Verdict {
        let mut inner = self.lock();
        let (verdict, panics) = inner.budgets.entry(actor).or_default().admit(now);
        let (restarts, relaunch) = match verdict {
            Verdict::Relaunch { restarts } => (restarts, "actor"),
            Verdict::GiveUp => {
                inner.exhausted.get_or_insert(Exhausted { actor, panics });
                (
                    inner
                        .budgets
                        .get(&actor)
                        .map_or(0, |budget| budget.restarts),
                    "daemon",
                )
            }
        };
        if let Some(diagnostics) = &inner.diagnostics {
            diagnostics.record(
                "actor_panicked",
                &[
                    ("actor", actor.name()),
                    ("restarts", &restarts.to_string()),
                    ("panics_in_window", &panics.to_string()),
                    ("budget", &RESTART_BUDGET.to_string()),
                    ("window_s", &RESTART_WINDOW.as_secs().to_string()),
                    ("relaunch", relaunch),
                ],
            );
        }
        verdict
    }

    pub fn panicked(&self, actor: Actor) -> Verdict {
        self.panicked_at(actor, Instant::now())
    }

    /// `actor` fell within its budget and cannot relaunch itself: its owner
    /// does, on its next look ([`Self::take_down`]).
    pub fn mark_down(&self, actor: Actor) {
        self.lock().down.insert(actor);
    }

    /// Whether `actor` fell since the last look, once.
    pub fn take_down(&self, actor: Actor) -> bool {
        self.lock().down.remove(&actor)
    }

    /// The first actor whose budget ran out, if any: the loop ends on it.
    pub fn exhausted(&self) -> Option<Exhausted> {
        self.lock().exhausted
    }

    /// How many times `actor` was relaunched so far.
    #[cfg(test)]
    pub fn restarts(&self, actor: Actor) -> u64 {
        self.lock()
            .budgets
            .get(&actor)
            .map_or(0, |budget| budget.restarts)
    }
}

impl Supervision {
    /// Runs one run of `actor`: its output, or, if it panicked, the verdict
    /// on that panic, which is counted and logged here.
    pub async fn guard<F: Future>(&self, actor: Actor, future: F) -> Result<F::Output, Verdict> {
        #[cfg(test)]
        let future = {
            let injected = self.take_injected(actor);
            async move {
                if injected {
                    injected_panic(actor);
                }
                future.await
            }
        };
        AssertUnwindSafe(future)
            .catch_unwind()
            .await
            .map_err(|_| self.panicked(actor))
    }

    /// The next `count` runs of `actor` under [`Self::guard`] panic.
    #[cfg(test)]
    pub fn inject(&self, actor: Actor, count: u32) {
        self.lock().injected.insert(actor, count);
    }

    #[cfg(test)]
    fn take_injected(&self, actor: Actor) -> bool {
        let mut inner = self.lock();
        let Some(left) = inner.injected.get_mut(&actor).filter(|left| **left > 0) else {
            return false;
        };
        *left -= 1;
        true
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "a test injects this panic on purpose")]
fn injected_panic(actor: Actor) {
    panic!("injected panic of {}", actor.name());
}

/// A task that its owner relaunches: a panic is counted, the actor is
/// marked down for its owner (within the budget), and the task ends with a
/// failure that names it, never with the panic's message.
pub async fn relaunchable<T>(
    actor: Actor,
    supervision: Supervision,
    future: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    match supervision.guard(actor, future).await {
        Ok(result) => return result,
        Err(Verdict::Relaunch { .. }) => supervision.mark_down(actor),
        Err(Verdict::GiveUp) => {}
    }
    Err(anyhow::anyhow!(
        "El actor {} falló y se relanza",
        actor.name()
    ))
}

impl Exhausted {
    /// The daemon's failure, in the person's language: it reaches the app's
    /// log as the reason of the relaunch.
    pub fn describe(&self) -> String {
        format!(
            "El actor {} falló {} veces en {} s; el servicio se reinicia entero",
            self.actor.name(),
            self.panics,
            RESTART_WINDOW.as_secs()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_relaunches_in_a_window_and_the_fourth_panic_gives_up() {
        let supervision = Supervision::default();
        let start = Instant::now();
        for second in 0..3 {
            assert!(matches!(
                supervision.panicked_at(Actor::CaptureTask, start + Duration::from_secs(second)),
                Verdict::Relaunch { .. }
            ));
        }
        assert_eq!(supervision.exhausted(), None);
        assert_eq!(
            supervision.panicked_at(Actor::CaptureTask, start + Duration::from_secs(3)),
            Verdict::GiveUp
        );
        assert_eq!(
            supervision.exhausted(),
            Some(Exhausted {
                actor: Actor::CaptureTask,
                panics: 4
            })
        );
    }

    #[test]
    fn panics_older_than_the_window_are_forgotten_and_actors_count_apart() {
        let supervision = Supervision::default();
        let start = Instant::now();
        for minute in 0..10 {
            let at = start + RESTART_WINDOW * minute;
            assert!(matches!(
                supervision.panicked_at(Actor::StoreWriter, at),
                Verdict::Relaunch { .. }
            ));
            assert!(matches!(
                supervision.panicked_at(Actor::HubConnection, at),
                Verdict::Relaunch { .. }
            ));
        }
        assert_eq!(supervision.restarts(Actor::StoreWriter), 10);
        assert_eq!(supervision.exhausted(), None);
    }

    #[test]
    fn the_log_carries_the_actor_and_its_counts_and_nothing_else() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let supervision = Supervision::new(Some(DiagnosticLog::in_root(directory.path())));
        supervision.panicked(Actor::HelperSupervisor);
        let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
        let line: serde_json::Value = serde_json::from_str(log.trim())?;
        assert_eq!(line["event"], "actor_panicked");
        assert_eq!(line["fields"]["actor"], "helper_supervisor");
        assert_eq!(line["fields"]["restarts"], "1");
        assert_eq!(line["fields"]["relaunch"], "actor");
        let fields = line["fields"].as_object().map_or(0, serde_json::Map::len);
        assert_eq!(fields, 6, "{line}");
        Ok(())
    }
}
