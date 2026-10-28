//! The helper's supervision: its launches and relaunches, the tap that hands
//! its events over, and its closure in order.
use super::*;
use crate::supervision::{Actor, Supervision, Verdict};
use futures_util::FutureExt;

/// Ten seconds of two-track input fits with margin: the tap produces
/// ≈47 ev/s × 2 pistas and the bridge needs room to keep emitting while a
/// slow consumer drains. When it is full the bridge waits and stops reading
/// the helper's pipe: the backlog then waits in the helper, which keeps up to
/// five minutes of audio in order. This bounds memory here, never audio.
pub const NATIVE_EVENTS_CAPACITY: usize = 1000;

/// How long the daemon gives its helper to leave by itself once it has been
/// sent `shutdown` and its input has been closed, before killing it.
///
/// What it protects: the helper's own exit — releasing the global shortcuts,
/// persisting its privacy latch and writing its last diagnostic line — which a
/// `SIGKILL` would skip, and which a helper orphaned with the shortcuts still
/// registered made impossible for the next one.
///
/// Why this size: by the time it is asked, the helper has already drained its
/// audio (that happens inside the daemon's own [`crate::daemon::SHUTDOWN_BUDGET`]),
/// so what is left is an AppKit termination and one small file write. The
/// helper's own leave-anyway grace, `NativeQuitPolicy.orphanExitGrace` in
/// Swift, is two seconds; this gives it one more before the kill. Not measured
/// on the Mac yet: it is among what a reviewer verifies there.
///
/// What it is not: a wait for audio. A helper that is still capturing when this
/// starts has already missed the closure's budget and is killed on time.
pub const HELPER_EXIT_GRACE: Duration = Duration::from_secs(3);

/// Room over [`HELPER_EXIT_GRACE`] for the supervisor task to kill and reap a
/// helper that did not leave, before the daemon stops waiting for the task and
/// drops it, which kills the child without reaping it. A `SIGKILL` and a
/// `waitpid` take milliseconds; this bounds a hang, it is not a measured wait.
const HELPER_KILL_ROOM: Duration = Duration::from_secs(1);

/// Bounded backoff sequence (1, 2, 4, 8, 16 s) — every value sits under
/// a 30 s ceiling; a single recovery consumes one slot.
const RELAUNCH_BACKOFFS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

/// Five consecutive failures (each with the same generation of attempts
/// after the last `ready`) exhaust the supervisor's patience.
const MAX_RELAUNCH_ATTEMPTS: u32 = 5;

/// Internal channel between the bridge and the events tap. Sized so the
/// bridge can keep forwarding while the daemon is briefly busy.
const RELAUNCH_BRIDGE_TAP_CAPACITY: usize = 1024;

/// How the helper's supervision ended, for the daemon's log: codes and flags,
/// never content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperEnd {
    /// It left by itself within its grace. `asked` says whether `shutdown`
    /// reached it before its input closed.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        asked: bool,
    },
    /// It had not left when its grace ran out and was killed.
    Killed { asked: bool },
    /// No helper was running when the stop came: between relaunches.
    NotRunning,
    /// The supervisor had already given up relaunching, or could not start it.
    GaveUp,
    /// The supervisor task did not finish within its bound and was dropped,
    /// which kills a child without reaping it.
    Lost,
}

impl HelperEnd {
    /// The fields of the daemon's `helper_stopped` line.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let flag = |value: bool| value.to_string();
        match *self {
            Self::Exited {
                code,
                signal,
                asked,
            } => {
                let mut fields = vec![("how", "exited".to_owned()), ("asked", flag(asked))];
                if let Some(code) = code {
                    fields.push(("code", code.to_string()));
                }
                if let Some(signal) = signal {
                    fields.push(("signal", signal.to_string()));
                }
                fields
            }
            Self::Killed { asked } => vec![
                ("how", "killed".to_owned()),
                ("asked", flag(asked)),
                ("grace_ms", HELPER_EXIT_GRACE.as_millis().to_string()),
            ],
            Self::NotRunning => vec![("how", "not_running".to_owned())],
            Self::GaveUp => vec![("how", "gave_up".to_owned())],
            Self::Lost => vec![("how", "lost".to_owned())],
        }
    }
}

/// The thread the helper's supervision runs on, by this name in a crash
/// report and in `ps -T`.
pub const SUPERVISOR_THREAD_NAME: &str = "orbit-helper-bridge";

/// The helper's supervision as the daemon holds it: how it ended and the one
/// signal that ends it in order.
///
/// It runs on a thread of its own, with its own runtime, not on the daemon's.
/// The heartbeat is what tells the helper the daemon is alive. On the
/// daemon's runtime, work that held all of its threads (a blocking call, a
/// slow disk flush) would hold the beat as well, and the helper would read a
/// busy daemon as a dead one; on 2026-10-22 the helper missed the beat for
/// over 6 s and cut a meeting. Pipe reads and writes, the beat and relaunches
/// now depend only on this thread.
pub struct Supervisor {
    end: oneshot::Receiver<HelperEnd>,
    stop: watch::Sender<bool>,
}

impl Supervisor {
    /// Launches the supervision on a thread of its own. Its events go through
    /// `route`: the loop's channel alone, or that and the capture task's.
    pub fn spawn(
        path: Option<PathBuf>,
        commands: mpsc::Receiver<Value>,
        route: impl Into<NativeRoute>,
        supervision: Supervision,
    ) -> Self {
        let route = route.into();
        let (stop, stopped) = watch::channel(false);
        let (ended, end) = oneshot::channel();
        let announce = route.clone();
        let launched = std::thread::Builder::new()
            .name(SUPERVISOR_THREAD_NAME.into())
            .spawn(move || {
                let end = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        let life = Life {
                            path,
                            commands,
                            route,
                            stopped,
                            supervision,
                        };
                        life.run(&runtime)
                    }
                    Err(_) => {
                        if route
                            .blocking_send(json!({"event":"helper_disconnected"}))
                            .is_err()
                        {
                            eprintln!("helper_disconnected_receiver_closed");
                        }
                        HelperEnd::GaveUp
                    }
                };
                // Nobody waits for it when the daemon did not ask to stop.
                let _ = ended.send(end);
            });
        if launched.is_err()
            && announce
                .try_send(json!({"event":"helper_disconnected"}))
                .is_err()
        {
            eprintln!("helper_disconnected_not_announced");
        }
        Self { end, stop }
    }

    /// Ends the supervision in order: the helper is sent `shutdown`, its input
    /// is closed and it gets [`HELPER_EXIT_GRACE`] to leave before it is killed.
    /// `events` is the daemon's end of the event channel, drained meanwhile so
    /// a full channel cannot hold the helper's last frames — or this stop.
    pub async fn stop(self, events: &mut mpsc::Receiver<Value>) -> HelperEnd {
        self.stop.send_replace(true);
        let mut end = self.end;
        let bound = tokio::time::sleep(HELPER_EXIT_GRACE + HELPER_KILL_ROOM);
        tokio::pin!(bound);
        let mut draining = true;
        loop {
            tokio::select! {
                ended = &mut end => return ended.unwrap_or(HelperEnd::Lost),
                event = events.recv(), if draining => draining = event.is_some(),
                // A thread cannot be aborted: the supervision ends on its own
                // within its bounded closure, and its child goes with the
                // process at the latest.
                () = &mut bound => return HelperEnd::Lost,
            }
        }
    }
}

/// What the supervision keeps across a panic of its own: the daemon's
/// commands, in order, the route its events take and the stop.
struct Life {
    path: Option<PathBuf>,
    commands: mpsc::Receiver<Value>,
    route: NativeRoute,
    stopped: watch::Receiver<bool>,
    supervision: Supervision,
}

impl Life {
    /// One run of [`supervise`] after another, while a panic is within the
    /// budget. The helper of the run that panicked goes with it: its child
    /// is killed on drop. The daemon hears that as a helper that fell, and
    /// latches the capture; the relaunched supervision launches a new
    /// helper, whose `ready` lets a recording that was live go on
    /// (`cut_recovery`), as after any fall of the helper.
    fn run(mut self, runtime: &tokio::runtime::Runtime) -> HelperEnd {
        loop {
            let run = supervise(
                self.path.clone(),
                &mut self.commands,
                self.route.clone(),
                self.stopped.clone(),
            );
            let Ok(end) = runtime.block_on(std::panic::AssertUnwindSafe(run).catch_unwind()) else {
                let verdict = self.supervision.panicked(Actor::HelperSupervisor);
                if self
                    .route
                    .blocking_send(json!({"event":"helper_disconnected"}))
                    .is_err()
                {
                    eprintln!("helper_disconnected_receiver_closed");
                }
                if verdict == Verdict::GiveUp || *self.stopped.borrow() {
                    return HelperEnd::GaveUp;
                }
                continue;
            };
            return end;
        }
    }
}

/// The supervision's whole life on its own thread: `run`, and the fall it
/// announces when the relaunches run out.
async fn supervise(
    path: Option<PathBuf>,
    commands: &mut mpsc::Receiver<Value>,
    route: NativeRoute,
    stopped: watch::Receiver<bool>,
) -> HelperEnd {
    let stopping = stopped.clone();
    match run_with(path, commands, route.clone(), stopped).await {
        Ok(end) => end,
        Err(_) => {
            // The daemon latches the capture on this; a supervision
            // that ends because it was asked to has nothing to report.
            if !*stopping.borrow()
                && route
                    .send(json!({"event":"helper_disconnected"}))
                    .await
                    .is_err()
            {
                eprintln!("helper_disconnected_receiver_closed");
            }
            HelperEnd::GaveUp
        }
    }
}

/// Supervises the helper until `stop` turns true, the command channel closes
/// or the relaunches run out. `stop` is how the daemon ends it in order.
pub async fn run(
    path: Option<PathBuf>,
    commands: mpsc::Receiver<Value>,
    route: impl Into<NativeRoute>,
    stop: watch::Receiver<bool>,
) -> Result<HelperEnd> {
    let mut commands = commands;
    run_with(path, &mut commands, route.into(), stop).await
}

async fn run_with(
    path: Option<PathBuf>,
    commands: &mut mpsc::Receiver<Value>,
    route: NativeRoute,
    mut stop: watch::Receiver<bool>,
) -> Result<HelperEnd> {
    let path = path.context("Native helper not configured")?;
    if !path.is_absolute() {
        anyhow::bail!("The helper requires an absolute path");
    }
    let mut consecutive_failures: u32 = 0;
    loop {
        if *stop.borrow() {
            return Ok(HelperEnd::NotRunning);
        }
        let (bridge_result, saw_ready) = launch(&path, commands, &route, &mut stop).await;
        let bridge_error = match bridge_result {
            Ok(end) => return Ok(end),
            Err(error) => error,
        };
        if saw_ready {
            // The previous cycle observed at least one ready event, so any
            // crash that followed it starts a fresh generation.
            consecutive_failures = 0;
        } else {
            consecutive_failures = consecutive_failures.saturating_add(1);
        }
        if saw_ready || consecutive_failures == 1 {
            announce_loss(&route, &bridge_error).await;
        }
        if consecutive_failures >= MAX_RELAUNCH_ATTEMPTS {
            return Err(anyhow::anyhow!(
                "The native helper crashed {} consecutive times without announcing ready again",
                MAX_RELAUNCH_ATTEMPTS
            ));
        }
        let backoff = RELAUNCH_BACKOFFS[consecutive_failures as usize];
        // A stop during the wait ends the supervision: nothing new is
        // launched into a daemon that is closing.
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            _ = stop.wait_for(|stopped| *stopped) => return Ok(HelperEnd::NotRunning),
        }
    }
}

/// One launch of the helper and its bridge, with the tap that forwards the
/// bridge's events through `route`. Says whether the helper said `ready`.
async fn launch(
    path: &std::path::Path,
    commands: &mut mpsc::Receiver<Value>,
    route: &NativeRoute,
    stop: &mut watch::Receiver<bool>,
) -> (Result<HelperEnd>, bool) {
    let saw_ready = Arc::new(AtomicBool::new(false));
    let (bridge_tx, mut bridge_rx): (mpsc::Sender<Value>, mpsc::Receiver<Value>) =
        mpsc::channel(RELAUNCH_BRIDGE_TAP_CAPACITY);
    let bridge_tx_for_bridge = bridge_tx.clone();
    let tap_route = route.clone();
    let tap_saw_ready = saw_ready.clone();
    let tap = tokio::spawn(async move {
        while let Some(value) = bridge_rx.recv().await {
            if NativeEventHeader::from_value(&value).is_ok_and(|header| header.event == "ready") {
                tap_saw_ready.store(true, Ordering::SeqCst);
            }
            if tap_route.send(value).await.is_err() {
                break;
            }
        }
    });
    let bridge = run_helper_bridge(path, commands, bridge_tx_for_bridge, stop);
    #[cfg(test)]
    let bridge = panic_when_asked(path, bridge);
    let bridge_result = bridge.await;
    // Closing the original sender (the bridge task already dropped its
    // clone) is what signals the tap to finish.
    drop(bridge_tx);
    let _ = tap.await;
    (bridge_result, saw_ready.load(Ordering::SeqCst))
}

/// The daemon needs to learn the helper is gone so it can latch the capture.
/// Every crash that closes a generation announces it: a helper that was
/// already ready (the real case of a crash after startup) and the first
/// crash of a generation that never got that far. Repeated crashes without a
/// ready in between do not re-announce; the supervisor gives up after five.
async fn announce_loss(route: &NativeRoute, error: &anyhow::Error) {
    let disconnected = match error
        .downcast_ref::<MalformedAudio>()
        .map(ToString::to_string)
    {
        Some(cause) => json!({"event":"helper_disconnected","cause":cause}),
        None => json!({"event":"helper_disconnected"}),
    };
    if route.send(disconnected).await.is_err() {
        eprintln!("helper_disconnected_receiver_closed");
    }
}

/// Ends a helper in order: `shutdown`, then its input closed, then
/// [`HELPER_EXIT_GRACE`] for it to leave while its output keeps being read, and
/// only then `SIGKILL`. The child is always reaped before this returns.
pub(super) async fn close_helper<R, W>(mut child: Child, reader: R, mut writer: W) -> HelperEnd
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let deadline = Instant::now() + HELPER_EXIT_GRACE;
    let asked = match command("shutdown", json!({})) {
        Ok(command) => tokio::time::timeout_at(
            deadline.min(Instant::now() + WRITE_TIMEOUT),
            write(&mut writer, command),
        )
        .await
        .is_ok_and(|written| written.is_ok()),
        Err(error) => {
            eprintln!("native_shutdown_command_invalid: {error}");
            false
        }
    };
    // End of input: the helper's other signal to leave, and the only one
    // a helper that missed `shutdown` can still get.
    drop(writer);
    // Its last frames — the shutdown receipt, a final state — are read and
    // dropped, so a full pipe cannot keep it from leaving.
    let mut reader = reader;
    let mut sink = tokio::io::sink();
    let drain = tokio::io::copy_buf(&mut reader, &mut sink);
    tokio::pin!(drain);
    let mut drained = false;
    let left = tokio::time::timeout_at(deadline, async {
        loop {
            tokio::select! {
                status = child.wait() => return status,
                _ = &mut drain, if !drained => drained = true,
            }
        }
    })
    .await;
    match left {
        Ok(Ok(status)) => {
            use std::os::unix::process::ExitStatusExt;
            HelperEnd::Exited {
                code: status.code(),
                signal: status.signal(),
                asked,
            }
        }
        _ => {
            let _ = child.kill().await;
            HelperEnd::Killed { asked }
        }
    }
}

/// A test asks the supervision to panic while its helper runs by creating
/// `<helper>.panic` beside the helper; it is removed as the panic fires.
/// Nothing in a build without tests looks for it.
#[cfg(test)]
#[allow(clippy::panic, reason = "a test injects this panic on purpose")]
async fn panic_when_asked<T>(path: &std::path::Path, bridge: impl Future<Output = T>) -> T {
    let trigger = path.with_extension("panic");
    tokio::pin!(bridge);
    loop {
        tokio::select! {
            out = &mut bridge => return out,
            () = tokio::time::sleep(Duration::from_millis(20)) => {
                if std::fs::remove_file(&trigger).is_ok() {
                    panic!("injected helper supervisor panic");
                }
            }
        }
    }
}

#[cfg(test)]
mod supervision_tests;
