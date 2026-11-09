//! The recording as it is kept: its phase, its progress and why it stopped.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePhase {
    #[default]
    Idle,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Interrupted,
}

impl CapturePhase {
    /// A restart must interrupt only a capture that was still active in the saved state.
    pub fn interrupted_on_recovery(&self) -> bool {
        matches!(self, Self::Starting | Self::Recording | Self::Finalizing)
    }
}

impl fmt::Display for CapturePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idle => "cerrada",
            Self::Starting => "iniciando",
            Self::Recording => "grabando",
            Self::Paused => "en pausa",
            Self::Finalizing => "finalizando",
            Self::Interrupted => "interrumpida",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub title: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub ended_at_ms: Option<i64>,
    /// Retired wire field: new recordings use i64::MAX; never consulted.
    pub expires_at_ms: i64,
    pub application_pid: Option<i32>,
    #[serde(default)]
    pub audio: crate::protocol::MeetingAudio,
    pub phase: CapturePhase,
    pub sealed_chunks: u64,
    pub gaps: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<TrackProgress>,
    /// Wall-clock instant when the recording entered `Finalizing`. The core
    /// uses this to bound the wait for the Mac's drain acknowledgement; the
    /// field is serialized with `default` so a snapshot written by an older
    /// version that never tracked the wait keeps loading.
    #[serde(default)]
    pub finalizing_since_ms: Option<i64>,
    /// Retired snapshot field retained for old readers. Never consulted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ceiling_notice_given: bool,
    /// Where the person muted the microphone («Silenciar»), from the
    /// recording's start; the last is open while it is muted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted_spans: Vec<crate::microphone_mute::MutedSpan>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackProgress {
    pub name: String,
    pub sealed_chunks: u64,
    pub gaps: u64,
    pub last_chunk_at_ms: i64,
}

/// Non-sensitive capture facts. Track is a machine identifier such as mic_local.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkMetadata {
    pub id: String,
    pub recording_id: String,
    pub track: String,
    /// Explicit capture scope shared by this recording's tracks. None denotes
    /// legacy/unknown provenance, never an epoch inferred from a timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_epoch_id: Option<String>,
    pub sequence: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Native capture clock, shared between tracks; never synthesized from wall time.
    pub monotonic_ns: u64,
    /// First observed sample offset in this recording track. Resume preserves
    /// the counter even when a new explicit capture epoch starts.
    pub first_sample: u64,
    /// Estimated UTC Unix milliseconds when the first frame reached the receiver.
    pub started_at_ms: i64,
}

/// A local receipt is returned only after the blob and journal are synchronized.
/// It does not claim remote persistence or power-loss certification on hardware.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkReceipt {
    pub metadata: ChunkMetadata,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingInterruption {
    pub cause: RecordingInterruptionCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub audio_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingInterruptionCause {
    PrivacyStop,
    CauseNotSaved,
    MicrophonePermissionRequired,
    ScreenPermissionRequired,
    CaptureStartRefused,
    /// Historical saved cause; new recordings do not expire by duration.
    RecordingExpired,
    CaptureFinalizeTimedOut,
    HelperCaptureStop,
    HelperIntegrityFailure,
    HelperLost,
    CaptureEventFailed,
    CaptureMaintenanceFailed,
    AudioStopUnconfirmed,
    CaptureStopUndelivered,
    Shutdown,
    ListenerStopped,
    Restart,
    Unreadable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    pub application_pid: i32,
    pub application_name: String,
    pub bundle_id: String,
}
