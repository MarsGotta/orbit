use serde::{Deserialize, Serialize};

pub use orbit_core::model::{ChunkMetadata, ChunkReceipt};

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
