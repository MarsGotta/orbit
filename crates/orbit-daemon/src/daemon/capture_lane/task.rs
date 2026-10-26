//! The capture task: every frame the helper sends comes here first, on a
//! thread and a runtime of its own, so nothing the daemon's loop waits on
//! (an order, the calendar, the hub, the notes, the store's snapshot) can
//! keep a recording from being sequenced, sealed and queued for upload.
//!
//! It never waits on the loop, on the writer or on the helper: a seal is
//! admitted without waiting and its receipt is read when it is there. It
//! waits only for its next frame, its own clock, or room in the queue of
//! what it hands the loop, which is the same backpressure the loop's own
//! channel put on the helper before.
use super::{CaptureLane, ForwardedItem, fast, uploads::Uploader};
use crate::daemon::store_actor::CapturePort;
use crate::native::{AudioItem, RoutedAudio};
use crate::supervision::{Actor, Supervision, Verdict};
use futures_util::FutureExt;
use std::{
    panic::AssertUnwindSafe,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

/// The thread the capture task runs on, by this name in a crash report and
/// in `ps -T`.
pub(in crate::daemon) const CAPTURE_THREAD_NAME: &str = "orbit-capture";

/// How many helper events the task may hand the loop before it waits for the
/// loop to take them. What it protects: the daemon's memory while the loop is
/// held and the task cannot serve frames itself (a voice turn, a call, a
/// drain). It is the loop's own channel bound (`NATIVE_EVENTS_CAPACITY`, ten
/// seconds of two tracks), so a held loop costs the helper exactly what it
/// cost before: the helper keeps the backlog, in order, and nothing is lost.
pub(in crate::daemon) const FORWARD_CAPACITY: usize = crate::native::NATIVE_EVENTS_CAPACITY;

/// The task's own clock: receipts read, a failed seal retried and uploads
/// scheduled once a second, as the loop's tick did, but on this thread.
pub(in crate::daemon) const CAPTURE_TICK: Duration = Duration::from_secs(1);

/// A task turn longer than this is logged as `capture_turn_slow`: the loop's
/// own threshold (`turn_watch::SLOW_TURN_THRESHOLD`). The task does nothing
/// that waits, so a slow turn is a defect to find, never a normal case.
const SLOW_TURN: Duration = crate::daemon::turn_watch::SLOW_TURN_THRESHOLD;

/// The `reason` of the gap the task declares for the frame it was serving
/// when it panicked: a code, as the helper's own reasons are.
pub(in crate::daemon) const PANIC_GAP_REASON: &str = "capture_task_panicked";

/// What the task needs from the daemon.
pub(in crate::daemon) struct TaskParts {
    pub(in crate::daemon) lane: CaptureLane,
    pub(in crate::daemon) audio: mpsc::Receiver<RoutedAudio>,
    pub(in crate::daemon) port: CapturePort,
    pub(in crate::daemon) store: PathBuf,
    pub(in crate::daemon) diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    /// The task's own reader, for the chunks it uploads.
    pub(in crate::daemon) reader: orbit_store::ReadOnlyStore,
    pub(in crate::daemon) gate: crate::daemon::upload_outcome::SharedGate,
    pub(in crate::daemon) root: PathBuf,
    /// [`CAPTURE_TICK`]; a test that proves something happens without the
    /// clock sets it longer than the test.
    pub(in crate::daemon) tick: Duration,
    /// Where its panics, and its transfers', are counted.
    pub(in crate::daemon) supervision: Supervision,
}

/// The running task, as the daemon holds it.
pub(in crate::daemon) struct CaptureTask {
    stop: watch::Sender<bool>,
    lane: CaptureLane,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CaptureTask {
    pub(in crate::daemon) fn spawn(parts: TaskParts) -> std::io::Result<Self> {
        let (stop, stopped) = watch::channel(false);
        let lane = parts.lane.clone();
        let thread = std::thread::Builder::new()
            .name(CAPTURE_THREAD_NAME.into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => supervise(&runtime, Held::new(parts), &stopped),
                    Err(_) => eprintln!("capture_task_runtime_failed"),
                }
            })?;
        Ok(Self {
            stop,
            lane,
            thread: Some(thread),
        })
    }

    /// Ends the task once the helper is gone. It reads and drops anything
    /// still queued, so the helper's last frames cannot hold its closure.
    pub(in crate::daemon) fn stop(&mut self) {
        // A task waiting for room in the queue for the loop stops waiting.
        self.lane.lock().discard = true;
        self.lane.wake_task();
        self.stop.send_replace(true);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("capture_task_panicked");
        }
    }
}

impl Drop for CaptureTask {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

/// Says the task is gone when it returns, however it returns.
struct Running(CaptureLane);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.lock().task_running = false;
        self.0.wake_loop();
    }
}

struct Task {
    lane: CaptureLane,
    port: CapturePort,
    store: PathBuf,
    diagnostics: Option<crate::diagnostics::DiagnosticLog>,
}

/// The helper frame the task was serving: its place and its track, for the
/// gap a panic leaves.
struct InHand {
    seq: u64,
    source: String,
    stream: String,
}

impl InHand {
    fn of(routed: &RoutedAudio) -> Option<Self> {
        let AudioItem::Frame(frame) = &routed.item else {
            return None;
        };
        Some(Self {
            seq: routed.seq,
            source: frame.get("source")?.as_str()?.to_owned(),
            stream: frame.get("stream_id")?.as_str()?.to_owned(),
        })
    }
}

/// What outlives one run of the task: its channel, in the helper's order,
/// its chunk transfers and the frame in hand. A relaunch starts from these
/// and from the lane, which the loop shares and keeps.
struct Held {
    task: Task,
    audio: mpsc::Receiver<RoutedAudio>,
    uploader: Uploader,
    transfers: mpsc::Receiver<()>,
    tick: Duration,
    supervision: Supervision,
    in_hand: Option<InHand>,
}

impl Held {
    fn new(parts: TaskParts) -> Self {
        let (done, transfers) = mpsc::channel(1);
        let uploader = Uploader::new(
            parts.reader,
            parts.port.clone(),
            parts.gate,
            parts.root,
            done,
            parts.supervision.clone(),
        );
        Self {
            task: Task {
                lane: parts.lane,
                port: parts.port,
                store: parts.store,
                diagnostics: parts.diagnostics,
            },
            audio: parts.audio,
            uploader,
            transfers,
            tick: parts.tick,
            supervision: parts.supervision,
            in_hand: None,
        }
    }

    /// Close admission, then preserve every accepted item before the receiver
    /// goes away. A later bridge send falls back to the loop, whose barrier
    /// waits for this handover so it cannot overtake the accepted backlog.
    async fn hand_over_backlog(&mut self) {
        let lane = self.task.lane.clone();
        lane.lock().handing_over = true;
        self.audio.close();
        while let Some(routed) = self.audio.recv().await {
            // Wait for room as usual. An explicit stop releases backpressure:
            // only the bounded input tail can then join the forwarded queue.
            loop {
                let room = {
                    let core = lane.lock();
                    core.discard || core.forwarded.len() < FORWARD_CAPACITY
                };
                if room {
                    break;
                }
                lane.wake_loop();
                lane.task_woken().await;
            }
            {
                let mut core = lane.lock();
                match routed.item {
                    AudioItem::Frame(frame) => {
                        fast::forward(&mut core, routed.seq, ForwardedItem::Event(frame));
                    }
                    AudioItem::Marker { .. } => core.last_control = routed.seq,
                }
                core.consumed_through = routed.seq;
                core.backlog = self.audio.len();
            }
            lane.wake_loop();
            lane.wake_progress();
        }
        lane.lock().handing_over = false;
        lane.wake_loop();
        lane.wake_progress();
    }

    /// After a panic, before the relaunch: the loop serves every frame until
    /// it arms the lane again, and the frame in hand, which may or may not
    /// have reached the recording, is declared a hole of its track. The
    /// hole is what keeps the recording: the loop seals that track's tail
    /// and accepts its next frame, instead of refusing a sequence that no
    /// longer follows. At most that one frame is lost, and said.
    fn after_panic(&mut self) {
        let lane = &self.task.lane;
        {
            let mut core = lane.lock();
            core.gate.armed = false;
            if let Some(in_hand) = self.in_hand.take() {
                let gap = serde_json::json!({
                    "event": "gap", "v": 1, "source": in_hand.source,
                    "stream_id": in_hand.stream, "reason": PANIC_GAP_REASON,
                    "frames": 0, "duration_unknown": true,
                });
                fast::forward(&mut core, in_hand.seq, ForwardedItem::Event(gap));
                core.consumed_through = core.consumed_through.max(in_hand.seq);
            }
        }
        lane.wake_loop();
        lane.wake_progress();
    }
}

/// The task's life: one run of [`serve`] after another, as long as a panic
/// is within its budget. Past it the task stays down and its channel closes,
/// so the bridge hands every frame to the loop (`NativeRoute::send`), and
/// the loop ends the daemon on the spent budget.
fn supervise(runtime: &tokio::runtime::Runtime, mut held: Held, stopped: &watch::Receiver<bool>) {
    loop {
        let run = AssertUnwindSafe(serve(&mut held, stopped.clone())).catch_unwind();
        if runtime.block_on(run).is_ok() {
            return;
        }
        held.after_panic();
        if held.supervision.panicked(Actor::CaptureTask) == Verdict::GiveUp {
            runtime.block_on(held.hand_over_backlog());
            return;
        }
    }
}

async fn serve(held: &mut Held, mut stopped: watch::Receiver<bool>) {
    let Held {
        task,
        audio,
        uploader,
        transfers,
        tick,
        in_hand,
        ..
    } = held;
    task.lane.lock().task_running = true;
    let _running = Running(task.lane.clone());
    if *stopped.borrow() {
        task.lane.lock().discard = true;
        return;
    }
    let mut tick = tokio::time::interval(*tick);
    let mut listening = true;
    loop {
        tokio::select! {
            routed = audio.recv(), if listening => match routed {
                Some(routed) => {
                    let started = Instant::now();
                    *in_hand = InHand::of(&routed);
                    if task.receive(routed, audio.len()).await {
                        uploader.schedule(&task.lane);
                    }
                    *in_hand = None;
                    task.timed("frame", started);
                }
                // No helper, or it is gone: the chunks still go up.
                None => listening = false,
            },
            Some(()) = transfers.recv() => uploader.schedule(&task.lane),
            _ = tick.tick() => {
                let started = Instant::now();
                task.maintain();
                uploader.schedule(&task.lane);
                task.timed("tick", started);
            }
            _ = stopped.changed() => if *stopped.borrow() {
                task.lane.lock().discard = true;
                return;
            },
        }
    }
}

impl Task {
    /// One item of the helper's order. Returns whether a seal receipt
    /// arrived, so a new chunk may go up at once.
    async fn receive(&self, routed: RoutedAudio, backlog: usize) -> bool {
        match routed.item {
            AudioItem::Marker { disarms } => {
                {
                    let mut core = self.lane.lock();
                    core.consumed_through = routed.seq;
                    core.last_control = routed.seq;
                    core.backlog = backlog;
                    if disarms {
                        core.gate.armed = false;
                    }
                }
                // A loop serving that event may be waiting for this place.
                self.lane.wake_progress();
                false
            }
            AudioItem::Frame(frame) => {
                #[cfg(test)]
                self.lane.panic_if_injected();
                self.room().await;
                let (handed_over, receipted) = {
                    let mut core = self.lane.lock();
                    core.backlog = backlog;
                    let before = core.forwarded.len();
                    let receipted = !core.discard
                        && fast::serve_frame(&mut core, &self.port, &self.store, routed.seq, frame);
                    core.consumed_through = routed.seq;
                    (core.forwarded.len() > before, receipted)
                };
                // A frame served here asks nothing of the loop: it is woken
                // only for what it must serve.
                if handed_over {
                    self.lane.wake_loop();
                }
                receipted
            }
        }
    }

    /// Waits while the queue for the loop is full and the frame would join
    /// it. An armed lane serves its frames itself and never waits here.
    async fn room(&self) {
        loop {
            {
                let core = self.lane.lock();
                if core.discard || core.gate.armed || core.forwarded.len() < FORWARD_CAPACITY {
                    return;
                }
            }
            self.lane.wake_loop();
            tokio::select! {
                () = self.lane.task_woken() => {}
                // A missed wake costs one recheck, never a stuck task.
                () = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }

    /// The task's own upkeep: receipts the writer logged meanwhile, and a
    /// failed seal retried once the writer's backoff has passed.
    fn maintain(&self) {
        let arrived = {
            let mut core = self.lane.lock();
            fast::upkeep(&mut core, &self.port)
        };
        if arrived {
            self.lane.wake_loop();
        }
    }

    fn timed(&self, what: &'static str, started: Instant) {
        let took = started.elapsed();
        if took <= SLOW_TURN {
            return;
        }
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record(
                "capture_turn_slow",
                &[("source", what), ("took_ms", &took.as_millis().to_string())],
            );
        }
    }
}
