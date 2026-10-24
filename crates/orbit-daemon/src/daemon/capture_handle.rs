//! The capture's handle: the recording's buffer, its pending seals and the
//! lane it shares with the capture task (`capture_lane`).
use super::{capture_lane, cut_recovery};
use crate::capture::Segment;
use anyhow::Result;
use orbit_store::ChunkMetadata;
use serde_json::Value;

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

impl CaptureHandle {
    /// Whether a voice turn is being captured.
    pub(super) fn voice_capturing(&self) -> bool {
        self.buffer.voice.is_some()
    }

    /// A voice turn starts; the capture task stops serving frames on its
    /// own, since the turn takes them too.
    pub(super) fn begin_voice(&mut self) -> Result<()> {
        self.buffer.begin_voice()
    }

    /// One frame of the voice turn, before it is sent.
    pub(super) fn ingest_voice_frame(&mut self, frame: &Value, now_ms: i64) -> Result<()> {
        self.buffer.ingest_voice_frame(frame, now_ms)
    }

    /// The voice turn's audio goes, unsent.
    pub(super) fn discard_voice(&mut self) {
        self.buffer.voice = None;
    }

    /// A recording starts in `epoch`: its tracks are now its owner's.
    pub(super) fn begin_recording_with_epoch(&mut self, id: String, epoch: String) -> Result<()> {
        self.buffer.begin_recording_with_epoch(id, epoch)
    }

    /// A resumed recording goes on from the last chunk stored of a track.
    pub(super) fn restore_position(&mut self, metadata: &ChunkMetadata) -> Result<()> {
        self.buffer.restore_position(metadata)
    }

    /// The recording `source` feeds, if it feeds one.
    pub(super) fn recording_owner(&self, source: &str) -> Option<&str> {
        self.buffer.recording_owner(source)
    }

    /// `source` stops feeding the recording: what it left is to be sealed.
    pub(super) fn pause_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        self.buffer.pause_source(source)
    }

    /// What every track holds, to be sealed; `end` closes the recording.
    pub(super) fn flush(&mut self, end: bool) -> Vec<Segment> {
        self.buffer.flush(end)
    }

    /// Whether a seal is still being written.
    pub(super) fn seals_pending(&self) -> bool {
        !self.pending_seals.is_empty()
    }

    /// Reserve exhaustion belongs to the recording whose unsealed PCM
    /// overflowed: once nothing of it is left to seal, the next capture
    /// starts with a fresh reserve.
    pub(super) fn renew_reserve_when_drained(&mut self) {
        if self.pending_seals.is_empty() && !self.buffer.has_pending_audio() {
            self.reserve_exhausted = false;
        }
    }

    /// What interrupted the recording, while it is interrupted.
    pub(super) fn interruption(&self) -> Option<&orbit_core::RecordingInterruption> {
        self.interruption.as_ref()
    }

    /// A recording starts: whatever interrupted the last one is behind it.
    pub(super) fn clear_interruption(&mut self) {
        self.interruption = None;
    }

    /// A deleted meeting's chunks are not uploaded any more, and the one
    /// going up now is stopped.
    pub(super) fn discard_uploads(&self, recording_id: &str) {
        let mut core = self.lane.lock();
        core.upload.discarded.insert(recording_id.to_owned());
        if let Some(transfer) = core.upload_abort.take() {
            transfer.abort();
        }
    }

    /// A deletion that failed: the meeting's chunks go up again.
    pub(super) fn keep_uploads(&self, recording_id: &str) {
        self.lane.lock().upload.discarded.remove(recording_id);
    }
}
