use serde::{Deserialize, Serialize};

/// Durable processing state. Unknown effects must be reconciled, not replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Blocked,
    Cancelled,
    Unknown,
}

impl JobState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retry_wait" => Ok(Self::RetryWait),
            "succeeded" => Ok(Self::Succeeded),
            "blocked" => Ok(Self::Blocked),
            "cancelled" => Ok(Self::Cancelled),
            "unknown" => Ok(Self::Unknown),
            _ => Err(crate::Error::Invalid("stored job state")),
        }
    }
}

/// The immutable operation input is encrypted; state changes never replace it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// A short machine-readable operation name, never a title or prompt.
    pub kind: String,
    pub payload: serde_json::Value,
    pub state: JobState,
    pub created_at: i64,
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

/// Recoverable artifacts are reported without silently deleting evidence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub inspected: usize,
    pub recovered: usize,
    pub invalid: usize,
    /// Manifests whose previously confirmed audio file is absent.
    pub missing: usize,
    pub incomplete_temporary: usize,
    pub limit_reached: bool,
}

/// Runtime evidence from the actual linked SQLite library and open connection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreDiagnostics {
    pub sqlite_version: String,
    pub sqlite_source_id: String,
    pub compile_options: Vec<String>,
    pub schema_version: i64,
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub fullfsync: i64,
    pub pending_jobs: u64,
    pub chunk_count: u64,
    pub chunk_bytes: u64,
}
