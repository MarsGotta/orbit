//! The recording's lane: its tracks, the seals they owe and the frames the
//! capture task serves while the daemon's loop is busy with anything else.
//!
//! One lock guards it, shared by the loop and the capture task, and neither
//! holds it across an await, a disk write or a wait on anyone: only the
//! memory of the recording changes under it. The loop decides, the task
//! follows: the task receives every frame, and while the loop has armed it
//! (a recording in its plain steady state) it sequences, seals and queues
//! the frame itself; any other frame, and every frame after one the task
//! could not serve, goes to the loop in the order it came.
use crate::capture::{ObservedPcmSignal, RecordingTracks, Segment};
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

mod buffer;
mod fast;
mod seal_cause;
mod seals;
pub(super) mod task;
#[cfg(test)]
pub(super) mod test_support;
mod uploads;

pub(super) use buffer::SplitBuffer;
pub(super) use seal_cause::{DestinationInvalid, ReserveExhausted, seal_cause};
pub(super) use seals::SealQueue;
pub(super) use uploads::{UploadReport, UploadView};

/// What the capture task hands the loop, in the order it came.
pub(super) enum ForwardedItem {
    /// A helper event the task did not serve: the loop serves it as before.
    Event(Value),
    /// A frame the recording refused (a hole, a changed format): the loop
    /// declares it and contains the capture as it does its own.
    FrameFailed { frame: Value, error: String },
    /// Segments the task cut but could not admit (the reserve is full): the
    /// loop admits them, with the gaps and the notice that come with that.
    Segments { segments: Vec<Segment> },
    /// A seal the task admitted could not be handed to the writer: the loop
    /// contains it as it contains its own.
    SealRefused { error: String },
}

pub(super) struct Forwarded {
    /// The bridge's position of the event this came from.
    pub(super) seq: u64,
    /// The last helper event the bridge sent the loop before this one: the
    /// loop serves this only once it has served that one.
    pub(super) after_control: u64,
    pub(super) item: ForwardedItem,
}

/// One durable seal receipt the loop has not counted yet.
pub(super) struct Receipt {
    pub(super) recording: String,
    pub(super) track: String,
    pub(super) started_at_ms: i64,
}

/// A seal the writer refused: its message for the log, its cause and how
/// many seals it still owes, for the notice.
pub(super) struct SealFailure {
    pub(super) message: String,
    pub(super) cause: &'static str,
    pub(super) owed: usize,
}

/// What the task observed for the loop to apply when it next runs.
#[derive(Default)]
pub(super) struct Outbox {
    pub(super) receipts: Vec<Receipt>,
    pub(super) signals: Vec<(ObservedPcmSignal, Instant)>,
    /// The writer refused a seal: what the loop says about it.
    pub(super) seal_failure: Option<SealFailure>,
    pub(super) uploads: Vec<UploadReport>,
}

/// Which streams feed the recording, as the loop last armed the lane.
#[derive(Default)]
pub(super) struct Gate {
    pub(super) armed: bool,
    pub(super) microphone: Option<String>,
    pub(super) remote: Option<String>,
}

#[derive(Default)]
pub(super) struct LaneCore {
    pub(super) tracks: RecordingTracks,
    /// Segments admitted to sealing and not yet receipted, oldest first.
    pub(super) seals: VecDeque<Arc<Segment>>,
    pub(super) gate: Gate,
    pub(super) forwarded: VecDeque<Forwarded>,
    /// The bridge position the task has handled up to, frames and markers.
    pub(super) consumed_through: u64,
    /// The position of the last event the bridge handed the loop, as its
    /// marker told the task.
    pub(super) last_control: u64,
    /// Frames still waiting in the task's channel, for a call's health.
    pub(super) backlog: usize,
    pub(super) outbox: Outbox,
    /// What the uploads may do, as the loop last allowed it.
    pub(super) upload: UploadView,
    /// The chunk transfer in flight, aborted when a decision retires it.
    pub(super) upload_abort: Option<tokio::task::AbortHandle>,
    /// The task is running; a barrier does not wait for one that is gone.
    pub(super) task_running: bool,
    /// The loop has ended: the task reads and drops whatever still comes, so
    /// the helper's last frames cannot hold its own closure.
    pub(super) discard: bool,
}

/// The lane, shared by the runtime and the capture task.
#[derive(Clone, Default)]
pub(super) struct CaptureLane {
    core: Arc<Mutex<LaneCore>>,
    /// Wakes whoever waits on the lane: the loop for forwarded items, the
    /// task for room in the forwarded queue.
    woke: Arc<tokio::sync::Notify>,
    task_woke: Arc<tokio::sync::Notify>,
    /// Wakes a loop waiting for the task to reach a helper event's place.
    progressed: Arc<tokio::sync::Notify>,
}

impl CaptureLane {
    /// The lane's state; a poisoned lock keeps its data, since the only
    /// writers are short sections that cannot leave it half-changed.
    pub(super) fn lock(&self) -> MutexGuard<'_, LaneCore> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Something for the loop landed in the lane.
    pub(super) fn wake_loop(&self) {
        self.woke.notify_one();
    }

    pub(super) async fn loop_woken(&self) {
        self.woke.notified().await;
    }

    /// The task reached a later place in the helper's order.
    pub(super) fn wake_progress(&self) {
        self.progressed.notify_one();
    }

    pub(super) async fn progressed(&self) {
        self.progressed.notified().await;
    }

    /// Whether the oldest item handed over may be served once the loop has
    /// served the helper events up to `served_control`.
    pub(super) fn forwarded_ready(&self, served_control: u64) -> bool {
        self.lock()
            .forwarded
            .front()
            .is_some_and(|front| front.after_control <= served_control)
    }

    /// Waits until an item handed over may be served: what the loop's arm
    /// for them waits on, so a wake with nothing to serve is no turn.
    pub(super) async fn forwarded_waiting(&self, served_control: u64) {
        while !self.forwarded_ready(served_control) {
            self.loop_woken().await;
        }
    }

    /// The loop took forwarded items or stopped the task's arming.
    pub(super) fn wake_task(&self) {
        self.task_woke.notify_one();
    }

    pub(super) async fn task_woken(&self) {
        self.task_woke.notified().await;
    }

    /// Stops the task's own handling until the loop arms the lane again.
    pub(super) fn disarm(&self) {
        self.lock().gate.armed = false;
    }
}
