//! The computer-control view the daemon publishes, and the authority an
//! engine call is checked against.
use serde::{Deserialize, Serialize};

/// Ephemeral presentation only. Observations do not survive a daemon restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerControlView {
    pub request_id: String,
    pub stage: ComputerStage,
    pub connected: bool,
    pub connection_epoch: Option<String>,
    pub detail: String,
    pub observation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerStage {
    Connecting,
    Reading,
    Ready,
    Unknown,
    Executing,
    PersistencePending,
    Completed,
    Failed,
    Cancelled,
}

impl ComputerStage {
    pub fn busy(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Reading | Self::Executing | Self::PersistencePending
        )
    }
}

#[derive(Debug, Clone)]
pub struct ActionAuthority {
    pub device_id: String,
    pub generation: u64,
    pub now_ms: i64,
    /// Hub availability is transport state, not local input authority.
    pub online: bool,
    pub privacy_stopped: bool,
}
