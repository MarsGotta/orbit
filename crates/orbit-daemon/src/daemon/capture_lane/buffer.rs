//! The runtime's view of the capture buffer: the voice turn it owns, and the
//! recording's tracks, which live in the lane the capture task shares.
//!
//! It answers with the same calls the single buffer did. Who owns the
//! recording (its id, its epoch, which sources feed it) changes only through
//! the runtime's own calls here, never through the task, so the runtime keeps
//! its own copy of that and reads it without the lock.
use super::CaptureLane;
use crate::capture::{self, IngestedAudio, Segment, VoiceBuffer};
use anyhow::Result;
use orbit_store::ChunkMetadata;
use serde_json::Value;

/// The recording's owner as the lane's tracks last said it.
#[derive(Default)]
struct Owner {
    recording_id: Option<String>,
    epoch: Option<String>,
    microphone: bool,
    remote: bool,
}

#[derive(Default)]
pub(in crate::daemon) struct SplitBuffer {
    pub(in crate::daemon) voice: Option<VoiceBuffer>,
    owner: Owner,
    lane: CaptureLane,
}

impl SplitBuffer {
    pub(in crate::daemon) fn new(lane: CaptureLane) -> Self {
        Self {
            voice: None,
            owner: Owner::default(),
            lane,
        }
    }

    /// Reads the owner back after a call that may have changed it.
    fn refresh_owner(&mut self) {
        let core = self.lane.lock();
        self.owner = Owner {
            recording_id: core.tracks.recording_id().map(str::to_owned),
            epoch: core.tracks.recording_epoch().map(str::to_owned),
            microphone: core.tracks.recording_owner("mic_local").is_some(),
            remote: core.tracks.recording_owner("remote_app").is_some(),
        };
    }

    /// Includes the incomplete tail that has not become a sealed segment yet.
    pub(in crate::daemon) fn has_pending_audio(&self) -> bool {
        self.voice.is_some() || self.has_unsealed_recording_audio()
    }

    /// Recording PCM received and not sealed yet (a voice turn aside).
    pub(in crate::daemon) fn has_unsealed_recording_audio(&self) -> bool {
        self.lane.lock().tracks.has_unsealed()
    }

    pub(in crate::daemon) fn recording_owner(&self, source: &str) -> Option<&str> {
        let feeds = match source {
            "mic_local" => self.owner.microphone,
            "remote_app" => self.owner.remote,
            _ => false,
        };
        feeds
            .then_some(self.owner.recording_id.as_deref())
            .flatten()
    }

    pub(in crate::daemon) fn recording_epoch(&self) -> Option<&str> {
        self.owner.recording_id.as_ref()?;
        self.owner.epoch.as_deref()
    }

    /// Legacy callers have unknown epoch; no timestamp or identity is invented.
    #[cfg(test)]
    pub(in crate::daemon) fn begin_recording(&mut self, id: String) -> Result<()> {
        let mut core = self.lane.lock();
        let mut buffer = crate::capture::CaptureBuffer {
            tracks: std::mem::take(&mut core.tracks),
            voice: None,
        };
        let begun = buffer.begin_recording(id);
        core.tracks = buffer.tracks;
        drop(core);
        self.refresh_owner();
        begun
    }

    pub(in crate::daemon) fn begin_recording_with_epoch(
        &mut self,
        id: String,
        epoch: String,
    ) -> Result<()> {
        let begun = self
            .lane
            .lock()
            .tracks
            .begin_recording_with_epoch(id, epoch);
        self.refresh_owner();
        begun
    }

    pub(in crate::daemon) fn restore_position(&mut self, metadata: &ChunkMetadata) -> Result<()> {
        self.lane.lock().tracks.restore_position(metadata)
    }

    /// A new voice turn: the task stops serving frames on its own, since the
    /// turn takes them too.
    pub(in crate::daemon) fn begin_voice(&mut self) -> Result<()> {
        self.lane.disarm();
        capture::begin_voice(&mut self.voice)
    }

    #[cfg(test)]
    pub(in crate::daemon) fn ingest(&mut self, frame: &Value, now_ms: i64) -> Result<Vec<Segment>> {
        self.ingest_observed(frame, now_ms)
            .map(|ingested| ingested.segments)
    }

    pub(in crate::daemon) fn ingest_observed(
        &mut self,
        frame: &Value,
        now_ms: i64,
    ) -> Result<IngestedAudio> {
        let mut core = self.lane.lock();
        capture::ingest(&mut core.tracks, &mut self.voice, frame, now_ms, true)
    }

    pub(in crate::daemon) fn ingest_voice_frame(
        &mut self,
        frame: &Value,
        now_ms: i64,
    ) -> Result<()> {
        let mut core = self.lane.lock();
        capture::ingest(&mut core.tracks, &mut self.voice, frame, now_ms, false).map(|_| ())
    }

    pub(in crate::daemon) fn track_rate(&self, source: &str) -> Option<u32> {
        self.lane.lock().tracks.track_rate(source)
    }

    pub(in crate::daemon) fn restart_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        let mut core = self.lane.lock();
        capture::restart_source(&mut core.tracks, &mut self.voice, source)
    }

    pub(in crate::daemon) fn pause_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        let paused = self.lane.lock().tracks.pause_source(source);
        self.refresh_owner();
        paused
    }

    pub(in crate::daemon) fn flush(&mut self, end: bool) -> Vec<Segment> {
        let segments = self.lane.lock().tracks.flush(end);
        self.refresh_owner();
        segments
    }

    /// As a buffer that never received anything, for a test that restarts.
    #[cfg(test)]
    pub(in crate::daemon) fn reset(&mut self) {
        self.voice = None;
        self.lane.lock().tracks = crate::capture::RecordingTracks::default();
        self.refresh_owner();
    }
}
