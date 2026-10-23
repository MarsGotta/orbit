//! The assistant and its chat: the files a turn carries and the
//! conversations read from the hub.
use super::fmt;
use serde::{Deserialize, Serialize};

parameter_enum!(HubSessionField { Id => "id", Status => "status" });

/// Which of the attachment ceilings a refusal names (`attachments`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentCeiling {
    File,
    Message,
    Session,
}

/// Why a file cannot be attached or a turn cannot carry its files. Each
/// carries the file's name and the numbers of the rule that fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttachmentCause {
    /// The hub has no attachments (0.9.0): nothing is read or sent.
    Unsupported,
    TooLarge {
        ceiling: AttachmentCeiling,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<u64>,
        bytes: u64,
        /// What the conversation already holds, for the session ceiling.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_bytes: Option<u64>,
        max_bytes: u64,
    },
    TooMany {
        files: u64,
        max_files: u64,
    },
    /// No agent reads this type; `mime` is what its content looks like.
    TypeUnsupported {
        name: String,
        mime: String,
    },
    /// The file could not be read: on this Mac (`missing`, `permission`,
    /// `not_a_file`, `io`), or by the hub (`pdf_locked`, `pdf_damaged`,
    /// `render_failed`).
    Unreadable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        reason: String,
    },
    /// The agent refused it (Hermes's `image.attach_bytes` code).
    Refused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        upstream_code: Option<i64>,
    },
    /// The hub no longer has it: never sent within the hour, or sent already.
    NotFound {
        attachment_id: String,
    },
}

macro_rules! assistant_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            Attachment = 0 => "attachment" { cause: AttachmentCause }, retry Retry::AfterFix;
            HubSessionsUnpaged = 22 => "hub_sessions_unpaged" {
                delivered: u64,
                total: u64,
            }, retry Retry::AfterFix;
            HubSessionInvalidRow = 23 => "hub_session_invalid_row" {
                position: u64,
                rows: u64,
                field: HubSessionField,
                bytes: u64,
                max_bytes: u64,
            }, retry Retry::AfterFix;
            HubSessionsCursorMismatch = 24 => "hub_sessions_cursor_mismatch" {
                rows: u64,
            }, retry Retry::AfterFix;
        }
    };
}
pub(super) use assistant_refusals;
