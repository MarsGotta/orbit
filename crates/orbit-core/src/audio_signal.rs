//! Bounded receiver-side signal observations, not a speech or mute detector.
//!
//! Callers supply validated mono PCM, format and receiver time. This module does
//! not retain samples, consult a clock, infer authorization, or produce effects.
use serde::{Deserialize, Serialize};

/// One second of mono S16 at the highest accepted rate.
pub const MAX_PCM_BYTES: usize = 384_000;
pub const MIN_SAMPLE_RATE: u32 = 8_000;
pub const MAX_SAMPLE_RATE: u32 = 192_000;
/// Inclusive RMS threshold in signed-16 amplitude units, not a dB calibration.
pub const LOW_SIGNAL_RMS_S16: u16 = 32;
pub const LOW_SIGNAL_THRESHOLD_MS: u64 = 10_000;
pub const SAMPLE_DELIVERY_TIMEOUT_MS: u64 = 3_000;

/// Validated integer statistics for one bounded PCM buffer. Private fields and
/// no deserialization prevent callers from constructing unchecked measurements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmSignal {
    frames: u64,
    sum_squared_amplitudes: u64,
}

impl PcmSignal {
    pub fn frames(self) -> u64 {
        self.frames
    }

    pub fn sum_squared_amplitudes(self) -> u64 {
        self.sum_squared_amplitudes
    }

    /// Compare mean-square energy exactly: rounding an integer square root
    /// would incorrectly classify some RMS values slightly greater than 32.
    pub fn is_low_signal(self) -> bool {
        let threshold = u64::from(LOW_SIGNAL_RMS_S16);
        self.sum_squared_amplitudes <= self.frames * threshold * threshold
    }
}

/// Measure mono signed-16 little-endian PCM without allocating or modifying it.
/// Empty, odd-length and oversized buffers have no measurement. The bound keeps
/// the sum of squared i16::MIN amplitudes well within u64.
pub fn measure_s16le(pcm: &[u8]) -> Option<PcmSignal> {
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) || pcm.len() > MAX_PCM_BYTES {
        return None;
    }
    let samples = pcm.as_chunks::<2>().0;
    let sum_squared_amplitudes = samples
        .iter()
        .map(|bytes| {
            let amplitude = i64::from(i16::from_le_bytes(*bytes));
            (amplitude * amplitude) as u64
        })
        .sum();
    Some(PcmSignal {
        frames: samples.len() as u64,
        sum_squared_amplitudes,
    })
}

/// Describes delivered samples only. LowSignal does not certify silence, speech
/// absence or muting; DeliveryDelayed does not identify a device or network fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalCondition {
    AwaitingSamples,
    Receiving,
    LowSignal,
    DeliveryDelayed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSignalView {
    pub condition: SignalCondition,
    /// Cumulative accepted frames, saturating rather than wrapping.
    pub received_frames: u64,
    /// None before the first samples or if the supplied clock precedes them.
    pub since_last_samples_ms: Option<u64>,
    /// Sample duration of the current low-energy run, not elapsed wall time.
    /// Zero when delivery is delayed or the supplied clock cannot be compared.
    pub low_signal_ms: u64,
    /// Delivered samples below the call's speech threshold. None means the
    /// producer did not supply this observation or delivery is not usable.
    /// Separate from low_signal_ms: room noise is not a dead microphone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_quiet_ms: Option<u64>,
}

/// At most two tracks. The adapter supplies and validates both identities;
/// absence of remote_app means that track was not requested, not failed delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSignalView {
    pub recording_id: String,
    pub capture_epoch_id: String,
    pub mic_local: TrackSignalView,
    pub remote_app: Option<TrackSignalView>,
}

/// Fixed-size history: counters and the last accepted format/time, never PCM.
/// A caller starts a new monitor when its stream or capture scope changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackSignalMonitor {
    started_at_ms: u64,
    last_samples_at_ms: Option<u64>,
    received_frames: u64,
    sample_rate: Option<u32>,
    low_frames: u64,
    speech_quiet_frames: u64,
}

impl TrackSignalMonitor {
    pub fn new(started_at_ms: u64) -> Self {
        Self {
            started_at_ms,
            last_samples_at_ms: None,
            received_frames: 0,
            sample_rate: None,
            low_frames: 0,
            speech_quiet_frames: 0,
        }
    }

    /// Accept at most one second of mono samples. Invalid format/duration or
    /// receiver time before the start/last accepted sample leaves self unchanged.
    /// Equal receive times are permitted for buffers delivered in the same tick.
    pub fn observe(&mut self, signal: PcmSignal, sample_rate: u32, received_at_ms: u64) -> bool {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate)
            || signal.frames > u64::from(sample_rate)
            || received_at_ms < self.started_at_ms
            || self
                .last_samples_at_ms
                .is_some_and(|previous| received_at_ms < previous)
        {
            return false;
        }
        let discontinuous = self.sample_rate != Some(sample_rate)
            || self
                .last_samples_at_ms
                .is_some_and(|previous| received_at_ms - previous >= SAMPLE_DELIVERY_TIMEOUT_MS);
        if discontinuous || !signal.is_low_signal() {
            self.low_frames = 0;
        }
        if signal.is_low_signal() {
            self.low_frames = self.low_frames.saturating_add(signal.frames);
        }
        let threshold = u64::from(crate::voice::VoicePolicy::default().rms_threshold);
        let voiced = signal.sum_squared_amplitudes >= signal.frames * threshold * threshold;
        if discontinuous || voiced {
            self.speech_quiet_frames = 0;
        }
        if !voiced {
            self.speech_quiet_frames = self.speech_quiet_frames.saturating_add(signal.frames);
        }
        self.received_frames = self.received_frames.saturating_add(signal.frames);
        self.last_samples_at_ms = Some(received_at_ms);
        self.sample_rate = Some(sample_rate);
        true
    }

    /// Read-only projection. Missing samples never accumulate low signal. A time
    /// before the start or latest accepted delivery yields Unknown; views do not
    /// advance a clock or mutate subsequent observations.
    pub fn view(&self, now_ms: u64) -> TrackSignalView {
        let mut view = TrackSignalView {
            condition: SignalCondition::Unknown,
            received_frames: self.received_frames,
            since_last_samples_ms: None,
            low_signal_ms: 0,
            speech_quiet_ms: None,
        };
        let reference = self.last_samples_at_ms.unwrap_or(self.started_at_ms);
        let Some(elapsed) = now_ms.checked_sub(reference) else {
            return view;
        };
        view.since_last_samples_ms = self.last_samples_at_ms.map(|_| elapsed);
        if elapsed >= SAMPLE_DELIVERY_TIMEOUT_MS {
            view.condition = SignalCondition::DeliveryDelayed;
        } else if self.last_samples_at_ms.is_none() {
            view.condition = SignalCondition::AwaitingSamples;
        } else {
            view.low_signal_ms = self.low_signal_ms();
            view.speech_quiet_ms = self
                .sample_rate
                .map(|rate| duration_ms(self.speech_quiet_frames, rate));
            view.condition = if view.low_signal_ms >= LOW_SIGNAL_THRESHOLD_MS {
                SignalCondition::LowSignal
            } else {
                SignalCondition::Receiving
            };
        }
        view
    }

    fn low_signal_ms(&self) -> u64 {
        let Some(rate) = self.sample_rate else {
            return 0;
        };
        duration_ms(self.low_frames, rate)
    }
}

fn duration_ms(frames: u64, rate: u32) -> u64 {
    let duration = u128::from(frames) * 1_000 / u128::from(rate);
    u64::try_from(duration).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
