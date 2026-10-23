//! Updates: why a search, a download or an installation of a new Orbit
//! did not finish.
use super::Retry;
use serde::{Deserialize, Serialize};

/// The existing size, upstream, timeout and parameter families cannot carry a
/// download URL, declared-versus-observed size, available manifest keys and
/// network cause together. Updates therefore have one typed family; the CLI
/// maps each cause to the corresponding exit-code family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateSizeSource {
    Declared,
    Observed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateNetworkCause {
    Dns,
    ConnectionRefused,
    Tls,
    Other { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallLogIssue {
    TooLarge,
    MissingResult { started_at: Option<String> },
    Io { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateCause {
    TooLarge {
        file: String,
        bytes: u64,
        max_bytes: u64,
        source: UpdateSizeSource,
    },
    HttpStatus {
        url: String,
        status: u16,
    },
    Deadline {
        url: String,
        waited_ms: u64,
        deadline_ms: u64,
    },
    NoResponse {
        url: String,
        cause: UpdateNetworkCause,
    },
    InvalidUtf8 {
        file: String,
        byte: usize,
    },
    MissingKey {
        key: String,
        available_keys: Vec<String>,
    },
    InvalidVersion {
        value: String,
    },
    InvalidFile {
        key: String,
        value: String,
    },
    InvalidDigest {
        key: String,
        value: String,
    },
    InstallerDigestMismatch {
        expected: String,
        actual: String,
    },
    InstallLog {
        bytes: Option<u64>,
        max_bytes: u64,
        reason: InstallLogIssue,
    },
    InvalidOrigin {
        bytes: usize,
    },
    InvalidRedirect {
        url: String,
        cause: String,
    },
    RedirectOutsideOrigin {
        from_origin: String,
        target_origin: String,
    },
    RedirectLimit {
        url: String,
        followed: usize,
        max_redirects: usize,
    },
    ClientBuild {
        cause: String,
    },
    UnsupportedArchitecture {
        value: String,
    },
    InstallerFailed {
        status: Option<i32>,
    },
    InstallerIo {
        cause: String,
    },
}

impl UpdateCause {
    /// An installer that ran or failed to start may have changed the Mac;
    /// anything else failed before installing.
    pub(super) fn retry(&self) -> Retry {
        match self {
            Self::InstallerFailed { .. } | Self::InstallerIo { .. } => Retry::CheckOutcome,
            Self::Deadline { .. } | Self::NoResponse { .. } | Self::HttpStatus { .. } => {
                Retry::AfterFix
            }
            Self::TooLarge { .. }
            | Self::InvalidUtf8 { .. }
            | Self::MissingKey { .. }
            | Self::InvalidVersion { .. }
            | Self::InvalidFile { .. }
            | Self::InvalidDigest { .. }
            | Self::InstallerDigestMismatch { .. }
            | Self::InstallLog { .. }
            | Self::InvalidOrigin { .. }
            | Self::InvalidRedirect { .. }
            | Self::RedirectOutsideOrigin { .. }
            | Self::RedirectLimit { .. }
            | Self::ClientBuild { .. }
            | Self::UnsupportedArchitecture { .. } => Retry::AfterFix,
        }
    }
}

macro_rules! updates_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            Update = 41 => "update" {
                cause: UpdateCause,
            }, retry cause.retry();
        }
    };
}
pub(super) use updates_refusals;
