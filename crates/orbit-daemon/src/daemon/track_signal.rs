//! Ephemeral receive-side observations, independent of capture/store authority.
//! The pure monitor never decides whether to retain PCM, add a gap or stop.
use super::Runtime;
use crate::capture::ObservedPcmSignal;
use orbit_core::audio_signal::{RecordingSignalView, TrackSignalMonitor};
use std::time::Instant;

pub(super) struct MeetingSignal {
    recording_id: String,
    capture_epoch_id: String,
    started_at: Instant,
    microphone_stream: String,
    remote_stream: Option<String>,
    microphone: TrackSignalMonitor,
    remote: Option<TrackSignalMonitor>,
}

impl MeetingSignal {
    fn new(
        recording_id: String,
        capture_epoch_id: String,
        microphone_stream: String,
        remote_stream: Option<String>,
        started_at: Instant,
    ) -> Self {
        let remote = remote_stream.as_ref().map(|_| TrackSignalMonitor::new(0));
        Self {
            recording_id,
            capture_epoch_id,
            started_at,
            microphone_stream,
            remote_stream,
            microphone: TrackSignalMonitor::new(0),
            remote,
        }
    }

    fn elapsed_ms(&self, at: Instant) -> Option<u64> {
        u64::try_from(at.checked_duration_since(self.started_at)?.as_millis()).ok()
    }

    fn observe(&mut self, frame: ObservedPcmSignal, at: Instant) {
        let Some(elapsed_ms) = self.elapsed_ms(at) else {
            return;
        };
        let monitor = match frame.source {
            "mic_local" if frame.stream_id == self.microphone_stream => Some(&mut self.microphone),
            "remote_app" if self.remote_stream.as_deref() == Some(&frame.stream_id) => {
                self.remote.as_mut()
            }
            _ => None,
        };
        if let Some(monitor) = monitor {
            // Invalid observational facts cannot change capture or discard PCM.
            let _ = monitor.observe(frame.level, frame.sample_rate, elapsed_ms);
        }
    }

    fn view(&self, at: Instant) -> Option<RecordingSignalView> {
        let elapsed_ms = self.elapsed_ms(at)?;
        Some(RecordingSignalView {
            recording_id: self.recording_id.clone(),
            capture_epoch_id: self.capture_epoch_id.clone(),
            mic_local: self.microphone.view(elapsed_ms),
            remote_app: self.remote.as_ref().map(|monitor| monitor.view(elapsed_ms)),
        })
    }
}

impl Runtime {
    pub(super) fn begin_recording_signal(&mut self, recording_id: String, epoch: String) {
        self.capture.audio.meeting.signal = self.capture.mic_stream.as_ref().map(|microphone| {
            MeetingSignal::new(
                recording_id,
                epoch,
                microphone.clone(),
                self.capture.remote_stream.clone(),
                Instant::now(),
            )
        });
    }

    pub(super) fn observe_recording_signal(
        &mut self,
        observation: Option<ObservedPcmSignal>,
        received_at: Instant,
    ) {
        if self.state.privacy_stopped() || !self.recording_active() {
            return;
        }
        let Some(observation) = observation else {
            return;
        };
        let Some(monitor) = self.capture.audio.meeting.signal.as_mut() else {
            return;
        };
        if self.capture.buffer.recording_owner(observation.source) == Some(&monitor.recording_id)
            && self.capture.buffer.recording_epoch() == Some(&monitor.capture_epoch_id)
            && self
                .state
                .durable
                .capture
                .recording
                .as_ref()
                .map(|recording| recording.id.as_str())
                == Some(&monitor.recording_id)
        {
            monitor.observe(observation, received_at);
        }
    }

    pub(super) fn recording_signal_view(&self) -> Option<RecordingSignalView> {
        self.recording_signal_view_at(Instant::now())
    }

    fn recording_signal_view_at(&self, at: Instant) -> Option<RecordingSignalView> {
        if self.state.privacy_stopped() || !self.recording_active() {
            return None;
        }
        if let Some(monitor) = &self.capture.audio.meeting.signal
            && self
                .state
                .durable.capture.recording
                .as_ref()
                .map(|recording| recording.id.as_str())
                == Some(&monitor.recording_id)
            // Sealing used to be synchronous, so a Status projection
            // could not expose all observed frames while its durable chunk
            // receipts still lagged behind. Preserve that coherence without
            // making Status wait on the writer: the live monitor stays in RAM
            // and becomes visible as soon as the admitted seals are receipted.
            && !self.capture.pending_seals.any_for(&monitor.recording_id)
            && self.capture.buffer.recording_epoch() == Some(&monitor.capture_epoch_id)
            && self.capture.buffer.recording_owner("mic_local") == Some(&monitor.recording_id)
            && (monitor.remote_stream.is_none()
                || self.capture.buffer.recording_owner("remote_app") == Some(&monitor.recording_id))
            && self.capture.mic_stream.as_deref() == Some(&monitor.microphone_stream)
            && self.capture.remote_stream == monitor.remote_stream
        {
            return monitor.view(at);
        }
        None
    }
}

#[cfg(test)]
mod tests;
