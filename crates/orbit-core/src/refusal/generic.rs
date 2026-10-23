//! The families every domain shares: a resource too large, an operation
//! busy or late, a parameter that is not what was expected. Each names its
//! domain through its `Resource`, `Operation` or `Parameter`, never through
//! a code of its own.
use super::Resource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InternalCause {
    NativeCommand,
    ControlHistory,
    ComputerStart,
    DaemonOperation { error: String },
    AppOperation { code: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StorageCause {
    Detail(String),
    PrivacyStop {
        error: String,
        step: crate::RecoveryStep,
        elapsed_ms: u64,
    },
}
impl From<String> for StorageCause {
    fn from(value: String) -> Self {
        Self::Detail(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadInterruption {
    Authority {
        change: crate::authority::AuthorityChange,
    },
    Cancelled,
    InventoryDispatch {
        sent_reads: u64,
        total_reads: u64,
    },
    GenerationChanged {
        started_generation: u64,
        current_generation: u64,
    },
    ConversationChanged,
    HelperRestarted {
        previous_generation: Option<u64>,
        current_generation: Option<u64>,
        previous_instance: Option<String>,
        current_instance: Option<String>,
    },
    CalendarChanged {
        changes: u64,
        max_restarts: u64,
    },
    CalendarHelperStopped,
    CalendarReadRestarted,
    CalendarGenerationChanged,
    CalendarReplyReplaced,
    SourceReplyReplaced,
    CaptureStopped,
    ComputerRevoked,
    CalendarExpired {
        deadline_ms: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonReplyIssue {
    Missing,
    Decode { error: String },
    CursorRepeated,
    CountMismatch { delivered: u64, total: u64 },
}

impl Resource {
    /// A response too large for the IPC line may already have done its work;
    /// any other resource too large is refused before anything happens.
    pub(super) fn too_large_retry(self) -> super::Retry {
        use super::Retry;
        match self {
            Self::IpcResponse => Retry::CheckOutcome,
            Self::Subscriptions
            | Self::IpcRequest
            | Self::HubResponse
            | Self::Message
            | Self::Recording
            | Self::Session
            | Self::Audio
            | Self::Calendar
            | Self::Source
            | Self::Capture
            | Self::ComputerAction
            | Self::Storage
            | Self::Folder
            | Self::File => Retry::AfterFix,
        }
    }
}

macro_rules! generic_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            EncodeFailed = 2 => "encode_failed" { resource: Resource }, retry Retry::Never;
            Internal = 6 => "internal" { cause: InternalCause }, retry Retry::Never;
            // The app and the daemon speak different IPC protocol versions.
            ProtocolVersion = 7 => "protocol_version" { received: u64, required: u64 },
                retry Retry::AfterFix;
            DaemonReplyInvalid = 9 => "daemon_reply_invalid" { field: String, issue: DaemonReplyIssue },
                retry Retry::CheckOutcome;
            ReadInterrupted = 4 => "read_interrupted" { cause: ReadInterruption },
                retry Retry::CheckOutcome;
            Busy = 26 => "busy" {
                operation: Operation,
                retry_after_ms: Option<u64>,
            }, retry match retry_after_ms {
                Some(ms) => Retry::AfterMs(*ms),
                None => Retry::AfterFix,
            };
            WorkInFlight = 28 => "work_in_flight" {
                in_flight: u64,
                max: u64,
            }, retry Retry::CheckOutcome;
            PhysicalWorkFinishing = 29 => "physical_work_finishing" {
                in_flight: u64,
                max: u64,
            }, retry Retry::CheckOutcome;
            TooLarge = 30 => "too_large" {
                resource: Resource,
                bytes: u64,
                max_bytes: u64,
            }, retry Resource::too_large_retry(*resource);
            TooMany = 31 => "too_many" {
                resource: Resource,
                count: u64,
                max: u64,
            }, retry Retry::AfterFix;
            InvalidParameter = 32 => "invalid_parameter" {
                parameter: Parameter,
                expected: Expected,
            }, retry Retry::AfterFix;
            RevisionChanged = 33 => "revision_changed" {
                expected_revision: u64,
                actual_revision: u64,
                expected_state_sha256: String,
                actual_state_sha256: String,
            }, retry Retry::AfterFix;
            Timeout = 34 => "timeout" {
                operation: Operation,
                waited_ms: u64,
                deadline_ms: u64,
            }, retry Retry::CheckOutcome;
            Expired = 35 => "expired" {
                resource: Resource,
                expired_at_ms: Option<i64>,
            }, retry Retry::AfterFix;
            NotFound = 36 => "not_found" {
                resource: Resource,
            }, retry Retry::AfterFix;
            OutcomeUnknown = 37 => "outcome_unknown" {
                operation_id: String,
            }, retry Retry::CheckOutcome;
            WorkExpired = 38 => "work_expired" {
                operation: Operation,
                started_generation: u64,
                current_generation: u64,
            }, retry Retry::CheckOutcome;
            StorageFailed = 40 => "storage_failed" {
                operation: Operation,
                #[serde(default, skip_serializing_if = "absent")]
                cause: Option<StorageCause>,
            }, retry Retry::CheckOutcome;
            Legacy = 42 => "legacy" {
                text: String,
            }, retry Retry::Never;
        }
    };
}
pub(super) use generic_refusals;
