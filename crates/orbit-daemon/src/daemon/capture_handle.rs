//! The capture's handle: the recording's buffer, its pending seals and the
//! lane it shares with the capture task (`capture_lane`).
use super::{capture_lane, cut_recovery};

/// What the loop holds of the recording being captured.
pub(super) struct CaptureHandle {
    pub(super) buffer: capture_lane::SplitBuffer,
    pub(super) pending_seals: capture_lane::SealQueue,
    /// The recording's lane, shared with the capture task.
    pub(super) lane: capture_lane::CaptureLane,
    /// Why the last seal the loop admitted failed, for the notice its caller
    /// says (`say_failure`).
    pub(super) seal_cause: Option<&'static str>,
    /// The bridge position of the last helper event the loop served: what
    /// the task handed over after it may be served.
    pub(super) served_control: u64,
    pub(super) reserve_exhausted: bool,
    pub(super) seal_retry_at: Option<std::time::Instant>,
    pub(super) interruption: Option<orbit_core::RecordingInterruption>,
    /// The cut that left the recording interrupted, while it waits to
    /// resume or close (`cut_recovery`).
    pub(super) recovery: Option<cut_recovery::CutRecovery>,
}

impl CaptureHandle {
    pub(super) fn new(recovery: Option<cut_recovery::CutRecovery>) -> Self {
        let lane = capture_lane::CaptureLane::default();
        Self {
            buffer: capture_lane::SplitBuffer::new(lane.clone()),
            pending_seals: capture_lane::SealQueue::new(lane.clone()),
            lane,
            served_control: 0,
            seal_cause: None,
            reserve_exhausted: false,
            seal_retry_at: None,
            interruption: None,
            recovery,
        }
    }
}
