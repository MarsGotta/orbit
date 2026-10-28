//! The seals the recording owes: admitted in order, handed to the writer
//! without waiting, and counted once their durable receipt is back.
//!
//! PCM ownership ends at a durable store receipt, never at an attempted
//! write. This queue is volatile: it survives a recoverable I/O failure, not
//! process death or power loss. Capture keeps going within the same bound.
use super::{CaptureLane, DestinationInvalid, LaneCore, Receipt};
use crate::capture::Segment;
use crate::daemon::store_actor::{CapturePort, StoreCompletion};
use anyhow::Result;
use std::{path::Path, sync::Arc};

/// Unsealed audio the lane may hold while the writer is slow or refusing:
/// what protects the daemon's memory from a disk that stopped. 64 MiB is over
/// half an hour of one 16 kHz track; past it the loss is declared as gaps
/// without growing the queue or cutting the capture.
pub(in crate::daemon) const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
/// The same protection counted in segments, for many short tails.
pub(in crate::daemon) const MAX_PENDING_SEGMENTS: usize = 4096;

/// The runtime's view of the seals the lane owes.
#[derive(Default)]
pub(in crate::daemon) struct SealQueue {
    lane: CaptureLane,
}

impl SealQueue {
    pub(in crate::daemon) fn new(lane: CaptureLane) -> Self {
        Self { lane }
    }

    pub(in crate::daemon) fn is_empty(&self) -> bool {
        self.lane.lock().seals.is_empty()
    }

    pub(in crate::daemon) fn len(&self) -> usize {
        self.lane.lock().seals.len()
    }

    /// The owed segments as they are now, oldest first.
    pub(in crate::daemon) fn iter(&self) -> std::vec::IntoIter<Arc<Segment>> {
        self.lane
            .lock()
            .seals
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// The segment owed at `index`, oldest first.
    #[cfg(test)]
    pub(in crate::daemon) fn get(&self, index: usize) -> Option<Arc<Segment>> {
        self.lane.lock().seals.get(index).cloned()
    }

    /// Whether a seal of `recording` is still owed.
    pub(in crate::daemon) fn any_for(&self, recording: &str) -> bool {
        self.lane
            .lock()
            .seals
            .iter()
            .any(|segment| segment.metadata.recording_id == recording)
    }

    /// Owes a segment the way a test that never cut it does.
    #[cfg(test)]
    pub(in crate::daemon) fn push_back(&self, segment: Segment) {
        self.lane.lock().seals.push_back(Arc::new(segment));
    }

    /// Forgets an owed seal, as a test that loses one does.
    #[cfg(test)]
    pub(in crate::daemon) fn remove(&self, index: usize) -> Option<Arc<Segment>> {
        self.lane.lock().seals.remove(index)
    }
}

impl LaneCore {
    fn pending_bytes(&self) -> usize {
        self.seals.iter().map(|segment| segment.pcm.len()).sum()
    }

    /// Whether admitting all of `segments` would pass the reserve.
    pub(super) fn would_overflow(&self, segments: &[Segment]) -> bool {
        let mut bytes = self.pending_bytes();
        for (count, segment) in (self.seals.len()..).zip(segments) {
            if segment.pcm.len() > MAX_PENDING_BYTES.saturating_sub(bytes)
                || count == MAX_PENDING_SEGMENTS
            {
                return true;
            }
            bytes += segment.pcm.len();
        }
        false
    }

    /// Admits the whole multitrack flush before its first write, so an error
    /// on that write cannot drop the other tracks. Returns what did not fit.
    pub(in crate::daemon) fn admit(&mut self, segments: Vec<Segment>) -> Vec<Segment> {
        let mut bytes = self.pending_bytes();
        let mut overflow = Vec::new();
        for segment in segments {
            if segment.pcm.len() > MAX_PENDING_BYTES.saturating_sub(bytes)
                || self.seals.len() == MAX_PENDING_SEGMENTS
            {
                overflow.push(segment);
                continue;
            }
            bytes += segment.pcm.len();
            self.seals.push_back(Arc::new(segment));
        }
        overflow
    }

    /// Refuses a chunk directory or destination that is not a private file.
    pub(in crate::daemon) fn validate_destinations(&self, store: &Path) -> Result<()> {
        let chunks = store.join("chunks");
        if !std::fs::symlink_metadata(&chunks)?.file_type().is_dir() {
            return Err(DestinationInvalid("The chunk path is not a private directory").into());
        }
        for segment in &self.seals {
            let destination = chunks.join(format!("{}.orbit", segment.metadata.id));
            match std::fs::symlink_metadata(destination) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    return Err(
                        DestinationInvalid("The chunk destination is not a regular file").into(),
                    );
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Hands every owed seal to the writer; one already handed is not again.
    pub(in crate::daemon) fn dispatch(&self, port: &CapturePort) -> Result<()> {
        for segment in &self.seals {
            port.seal(Arc::clone(segment))?;
        }
        Ok(())
    }

    /// Takes the writer's receipts into the outbox, for the loop to count.
    /// Returns whether a receipt or a failure arrived.
    pub(in crate::daemon) fn collect(&mut self, port: &CapturePort) -> bool {
        let mut arrived = false;
        for completion in port.take_completions() {
            arrived = true;
            match completion {
                StoreCompletion::Sealed {
                    id,
                    recording,
                    sequence,
                    queue_wait_us: _,
                } => self.receipt(port, &id, &recording, sequence),
                StoreCompletion::SealFailed {
                    message,
                    cause,
                    owed,
                } => {
                    self.outbox.seal_failure = Some(super::SealFailure {
                        message,
                        cause,
                        owed,
                    });
                    self.gate.armed = false;
                }
            }
        }
        arrived
    }

    fn receipt(&mut self, port: &CapturePort, id: &str, recording: &str, sequence: u64) {
        let position = self.seals.iter().position(|segment| {
            segment.metadata.id == id
                && segment.metadata.recording_id == recording
                && segment.metadata.sequence == sequence
        });
        let Some(segment) = position.and_then(|position| self.seals.remove(position)) else {
            return;
        };
        port.acknowledge(&segment.metadata.id);
        self.outbox.receipts.push(Receipt {
            recording: segment.metadata.recording_id.clone(),
            track: segment.metadata.track.clone(),
            started_at_ms: segment.metadata.started_at_ms,
        });
    }
}
