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
    /// Whether the person was told the recording is about to reach its
    /// ceiling: said once. Absent in a snapshot from an older version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ceiling_notice_given: bool,
    /// Where the person muted the microphone («Silenciar»), from the
    /// recording's start; the last is open while it is muted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted_spans: Vec<crate::microphone_mute::MutedSpan>,
}

/// How long any recording may run, whatever started it: by hand, the menu
/// bar, ⌃⌥M, an event in Hoy or the calendar, or a detected call.
///
/// Protects: a recording somebody forgot, or a call whose end the Mac never
/// saw, which would otherwise keep recording and uploading for ever. It is
/// not meant to cut meetings and nothing about a meeting sets it, not even
/// its calendar event: an event's end once stopped a meeting that ran over.
/// Why four hours: meetings of two hours must be kept whole, and four leaves
/// as much again for one that runs over. The app says it before a recording
/// starts («Graba hasta 4 h») from the snapshot, never from a copy. It does
/// not bound a voice call with the assistant (`voice::CALL_MINUTES`), the
/// audio kept on the Mac or what the hub accepts.
pub const RECORDING_CEILING_MINUTES: u16 = 240;

/// How long before [`RECORDING_CEILING_MINUTES`] the person is told, once,
/// that the recording will stop: room to finish it and start another.
pub const RECORDING_CEILING_NOTICE_MINUTES: i64 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackProgress {
    pub name: String,
    pub sealed_chunks: u64,
    pub gaps: u64,
    pub last_chunk_at_ms: i64,
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
