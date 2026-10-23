//! All exit causes share one local durability boundary. Losing IPC is not
//! permission to abandon PCM, and closing cannot be cancelled by a late rearm.
//! Closing cuts the capture without latching a stop: the next start comes
//! back working unless the user had pressed theirs.
//! Every door into the closure — the IPC `Shutdown`, SIGTERM and SIGINT —
//! latches the same intent with the same bounded budget, so a disk that will never accept a seal cannot hold the daemon
//! hostage forever, and whoever waits for the daemon knows the one number to
//! wait for.
use super::{Command, Runtime};
use anyhow::{Context, Result};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::NativeEventHeader;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// How long an orderly closure may take, from the moment it is latched to the
/// moment the daemon stops waiting and declares what is still in memory.
///
/// What it protects: the durable closure — the helper's drain of live audio,
/// the seals of the audio already received and every vertical's pending write
/// — from being cut short by whoever asked for it. The same number for every
/// door: the app quitting (`Shutdown` over IPC), launchd at logout (SIGTERM)
/// and a terminal (SIGINT).
///
/// Why this size: an ordinary closure takes well under a second (the app-core
/// suite measured 597 ms at its worst, under load, for the daemon's exit after
/// `Shutdown`), and ten seconds leave room for two retries of a failed seal at
/// the store's five-second retry delay.
///
/// What it is not: the whole exit. After it the daemon still gives its helper
/// [`crate::native::HELPER_EXIT_GRACE`] to leave, and everyone who waits for
/// the daemon — the app, the installer — waits at least the
/// sum. Those relations are kept by hand (the guard that checked them was
/// removed).
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);

/// Extra time for the final critical snapshot after the closure budget expires.
/// It protects the audio-loss declaration from the writer's 250 ms coalescing
/// lane and from a brief scheduler stall before the thread is detached.
/// Twenty critical receipts took 17–94 ms at load 13.46–15.10; the previous
/// two-write path took up to 1721 ms in 12 runs at load 18.86–37.30. This
/// 1128 ms bound is twelve times the critical path's observed worst; the
/// app's quit wait (`DAEMON_EXIT_WAIT`, apps/desktop/src-tauri) is checked at
/// compile time against it, the closure, the helper's grace and `EXIT_FLUSH`. It is not a guarantee that a hung fsync completed and is not
/// a new audio-drain budget.
pub const SNAPSHOT_RECEIPT_GRACE: Duration = Duration::from_millis(1128);

#[derive(Default)]
pub(super) struct Shutdown {
    pub requested: bool,
    pub listener_failed: bool,
    pub(super) first_press_at: Option<Instant>,
    pub(super) deadline: Option<Instant>,
    pub(super) signals: u8,
}

pub(super) enum CloseWait {
    Completed,
    BudgetExpired,
    SecondSignal(&'static str),
}

pub(super) enum BoundedExitCause {
    BudgetExpired,
    SecondSignal(&'static str),
}

impl Shutdown {
    /// The writer can remain inside fsync after the helper has been told to
    /// stop. Keep both the already established ten-second closure budget and
    /// a second signal live while the shutdown future awaits that writer.
    pub(super) async fn close_until_exit<F>(
        &mut self,
        runtime: &mut Runtime,
        next_signal: F,
    ) -> CloseWait
    where
        F: std::future::Future<Output = &'static str>,
    {
        self.latch();
        let Some(deadline) = self.deadline else {
            return CloseWait::BudgetExpired;
        };
        tokio::select! {
            biased;
            signal = next_signal => CloseWait::SecondSignal(signal),
            outcome = self.command(Command::Shutdown, runtime) =>
                if outcome.is_err() && Instant::now() >= deadline {
                    CloseWait::BudgetExpired
                } else {
                    CloseWait::Completed
                },
        }
    }
    /// Starts the bounded budget if no latch has been recorded yet. Called by
    /// every path that turns the exit intent on (a signal, the IPC shutdown
    /// command and a failed listener), so the budget and the
    /// latch share one source and the daemon cannot end up requested without a
    /// deadline.
    fn latch(&mut self) {
        if self.first_press_at.is_none() {
            let now = Instant::now();
            self.first_press_at = Some(now);
            self.deadline = Some(now + SHUTDOWN_BUDGET);
        }
        self.requested = true;
    }

    pub async fn command(&mut self, command: Command, runtime: &mut Runtime) -> anyhow::Result<()> {
        if !self.allows_command(&command) {
            anyhow::bail!("Orbit está cerrando y conservando audio; no admite nuevas operaciones");
        }
        // The intent survives a fallible flush. Recovery may finish the exit,
        // but may never silently change it into permission to capture again.
        if matches!(command, Command::Shutdown) {
            self.latch();
        }
        if matches!(command, Command::Shutdown)
            && let Some(deadline) = self.deadline
        {
            return tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                runtime.command(command),
            )
            .await
            .map_err(|_| anyhow::anyhow!(
                "Cierre sin confirmar tras el plazo de {} s: la operación no terminó y su etapa exacta quedó sin observar; pendientes: {}. Se declarará lo conservado en memoria",
                SHUTDOWN_BUDGET.as_secs(), runtime.pending_stop_counts()
            ))?;
        }
        runtime.command(command).await
    }

    pub async fn observe_listener(&mut self, finished: bool, runtime: &mut Runtime) {
        if finished && !self.listener_failed {
            self.listener_failed = true;
            self.latch();
            if let Some(deadline) = self.deadline {
                let _ = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(deadline),
                    runtime.close_for_exit(
                        orbit_core::privacy_stop::StopCause::ListenerStopped,
                        "El servidor IPC se detuvo; Orbit se cierra conservando el audio pendiente.",
                    ),
                )
                .await;
            }
        }
    }

    /// Counts termination signals; SIGINT and SIGTERM are the same request. The
    /// first one latches the exit intent with [`SHUTDOWN_BUDGET`], or keeps the
    /// budget another door already started: a SIGTERM that lands while an IPC
    /// `Shutdown` is closing (logout while the app quits) must not cut that
    /// closure short. A second one means "now": the daemon declares what is
    /// left and leaves.
    pub fn signal(&mut self) {
        self.signals = self.signals.saturating_add(1);
        if self.signals == 1 {
            self.latch();
        } else {
            self.requested = true;
            self.deadline = Some(Instant::now());
        }
    }

    /// Decides whether the one budget (or a second signal) has run out at `now`.
    pub fn exit_now(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    pub fn ready(&self, runtime: &Runtime) -> bool {
        // Audio drain is a separate gate: it tracks an ongoing native stop,
        // not memory waiting to be sealed, and the bounded budget must not
        // claim authority over what the helper still owns. The single pending
        // list drove the rest of this gate.
        self.requested && !runtime.audio.audio_is_draining() && runtime.has_any_pending().is_empty()
    }

    pub fn allows_command(&self, command: &Command) -> bool {
        !self.requested
            || matches!(
                command,
                Command::Status | Command::PrivacyStop | Command::Shutdown
            )
    }

    pub fn allows_native_event(&self, event: &Value) -> bool {
        !self.requested
            || NativeEventHeader::from_value(event).is_ok_and(|header| {
                matches!(
                    header.event,
                    "audio" | "gap" | "response" | "helper_disconnected"
                )
            })
    }
}

/// SIGINT and SIGTERM, installed before anything else starts. SIGTERM is what
/// launchd sends at logout and what `kill` sends; SIGINT is a terminal's
/// Ctrl-C. Both used to differ — only SIGINT was heard, so SIGTERM killed the
/// daemon without its closure — and both now open the same door.
pub(super) struct Signals {
    interrupt: Signal,
    terminate: Signal,
}

impl Signals {
    pub(super) fn install() -> Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())
                .context("No se pudo atender SIGINT para cerrar de forma ordenada")?,
            terminate: signal(SignalKind::terminate())
                .context("No se pudo atender SIGTERM para cerrar de forma ordenada")?,
        })
    }

    /// The next termination signal, by name. A stream that ended can never
    /// deliver again, so it waits for ever rather than for nothing.
    pub(super) async fn next(&mut self) -> &'static str {
        tokio::select! {
            Some(()) = self.interrupt.recv() => "SIGINT",
            Some(()) = self.terminate.recv() => "SIGTERM",
            else => std::future::pending::<&'static str>().await,
        }
    }
}

impl Runtime {
    /// The last line about the helper: whether it left by itself after
    /// `shutdown` or had to be killed, with codes only.
    pub(super) fn log_helper_end(&self, end: crate::native::HelperEnd) {
        let fields = end.fields();
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        self.log("helper_stopped", &fields);
    }

    /// One list of every vertical with work still held in memory. The names are
    /// stable so logs and snapshots stay diffable; everything else (counts,
    /// audio loss) layers on top of the same set.
    pub(super) fn has_any_pending(&self) -> Vec<&'static str> {
        let mut pending = Vec::new();
        if !self.capture.pending_seals.is_empty() {
            pending.push("seals");
        }
        if self.capture.buffer.has_pending_audio() {
            pending.push("audio");
        }
        if self.audio.call.is_some() {
            pending.push("call");
        }
        pending
    }

    pub(super) fn pending_stop_counts(&self) -> String {
        let counts = self
            .has_any_pending()
            .into_iter()
            .map(|name| format!("{name}: {}", self.capture.pending_count(name)))
            .collect::<Vec<_>>();
        if counts.is_empty() {
            "snapshot: 1".to_owned()
        } else {
            counts.join(", ")
        }
    }

    /// Closes the loop on a closure that ran out of time: logs what stayed in
    /// memory (counts only) and queues the final snapshot. A writer stuck
    /// inside fsync cannot be joined on exit, so the process detaches it only
    /// at this already declared, bounded boundary.
    pub(super) async fn declare_bounded_exit(&mut self) -> Result<()> {
        self.declare_bounded_exit_for(BoundedExitCause::BudgetExpired)
            .await
    }

    pub(super) async fn declare_bounded_exit_for(&mut self, cause: BoundedExitCause) -> Result<()> {
        self.declare_bounded_exit_with_grace(cause, SNAPSHOT_RECEIPT_GRACE)
            .await
    }

    async fn declare_bounded_exit_with_grace(
        &mut self,
        cause: BoundedExitCause,
        snapshot_receipt_grace: Duration,
    ) -> Result<()> {
        let pending = self.has_any_pending();
        let joined_pending = pending.join(",");
        let unsealed_audio =
            !self.capture.pending_seals.is_empty() || self.capture.buffer.has_pending_audio();
        let counts = pending
            .iter()
            .map(|name| format!("{name}: {}", self.capture.pending_count(name)))
            .collect::<Vec<_>>()
            .join(", ");
        let summary = if counts.is_empty() {
            "sin pendientes".to_string()
        } else {
            counts
        };
        self.log(
            "bounded_exit",
            &[
                (
                    "pending",
                    if pending.is_empty() {
                        "none"
                    } else {
                        // Bound to a name so the borrow lives long enough for
                        // the log call.
                        &joined_pending
                    },
                ),
                ("audio", if unsealed_audio { "unsealed" } else { "sealed" }),
            ],
        );
        let reason = match cause {
            BoundedExitCause::BudgetExpired => {
                format!("tras el plazo de {} s", SHUTDOWN_BUDGET.as_secs())
            }
            BoundedExitCause::SecondSignal(signal) => format!("por segunda señal {signal}"),
        };
        self.log(
            "bounded_exit_declared",
            &[("reason", &reason), ("summary", &summary)],
        );
        self.say(
            Notice::new(NoticeCode::ClosedWithPendingWork)
                .with(NoticeKey::AudioUnsaved, unsealed_audio)
                .with(NoticeKey::ReserveExhausted, self.capture.reserve_exhausted),
        );
        // This non-coalesced lane supersedes a waiting presentation snapshot.
        // Its receipt confirms the loss declaration before detach.
        match tokio::time::timeout(
            snapshot_receipt_grace,
            self.store_actor.critical_snapshot(&self.state),
        )
        .await
        {
            Ok(Ok(())) => self.log("final_snapshot_confirmed", &[]),
            Ok(Err(error)) => self.say_failure(
                Notice::new(NoticeCode::StopStepFailed)
                    .with_token(NoticeKey::Step, "final_snapshot"),
                &error,
            ),
            Err(_) => self.say_failure(
                Notice::new(NoticeCode::StopStepFailed)
                    .with_token(NoticeKey::Step, "final_snapshot"),
                &format!(
                    "sin confirmar tras {} ms",
                    snapshot_receipt_grace.as_millis()
                ),
            ),
        }
        self.store_actor.detach_for_bounded_exit();
        Ok(())
    }
}

impl crate::daemon::capture_handle::CaptureHandle {
    /// Counts how many pieces of work a vertical still holds, without naming
    /// the content. Every vertical except the seal queue reports at most one
    /// outstanding operation; seals report the segment backlog.
    fn pending_count(&self, name: &str) -> usize {
        match name {
            "seals" => self.pending_seals.len(),
            _ => 1,
        }
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod bounded_exit_tests;

#[cfg(test)]
#[path = "shutdown_close_tests.rs"]
mod tests;
