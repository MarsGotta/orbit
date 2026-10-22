//! Encrypted persistence owned by the daemon's single storage worker. This
//! crate performs storage effects, including durable retry bookkeeping, only.
//! Scheduling, authorization, time and Keychain access belong to its caller.
//! Every sensitive payload is authenticated and encrypted before SQLite or a
//! temporary file receives it.
mod crypto;
mod migrations;
mod model;
mod queries;
mod store;
mod validation;
mod writes;
pub use model::*;
mod assistant_folders;
mod blobs;
mod call_preference;
pub use blobs::RecoveryScan;
mod reader;
pub use reader::ReadOnlyStore;
mod recording_notes;
pub use recording_notes::{MAX_PRIVATE_NOTES_BYTES, PendingPrivateNotes};
pub use store::Store;
/// Limits apply before serialization, disk reads, or collecting database rows.
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PAGE_SIZE: usize = 500;
pub const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
// One declaration produces both the enqueue names and the SQL filter. A new
// daemon kind cannot be declared without entering the queue traversal too.
macro_rules! queue_kinds {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// Work the daemon enqueues. Stored jobs retain string kinds so older
        /// domain records and retired work remain readable during migrations.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum QueueKind { $($variant),+ }

        impl From<QueueKind> for String {
            fn from(kind: QueueKind) -> Self {
                match kind { $(QueueKind::$variant => $name.into()),+ }
            }
        }

        /// Queue traversal excludes domain records and retired work.
        pub const QUEUE_KINDS: &[&str] = &[$($name),+];
    };
}

queue_kinds! {
    AudioProcessingReceipt => "audio_processing_receipt",
    CallRecoveryBoundary => "call_recovery_boundary",
    CaptureEpoch => "capture_epoch",
    ChunkUpload => "chunk_upload",
    RecordingFinalize => "recording_finalize",
    RecordingManifest => "recording_manifest",
}
/// Errors deliberately omit payloads, credentials and conversation content.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage database operation failed: {0}")]
    Database(#[source] rusqlite::Error),
    #[error("storage payload has an invalid structure")]
    Serialization,
    #[error("encrypted storage could not be authenticated; key or content differs")]
    Authentication,
    #[error("secure random generation failed")]
    Random,
    /// A payload larger than its format permits: its bytes and the maximum.
    #[error("{what} ocupa {bytes} bytes y el máximo es {max_bytes}")]
    TooLarge {
        what: &'static str,
        bytes: u64,
        max_bytes: u64,
    },
    #[error("{what}: {count}; debe estar entre {min_count} y {max_count}")]
    CountOutOfRange {
        what: &'static str,
        count: usize,
        min_count: usize,
        max_count: usize,
    },
    #[error(transparent)]
    NoteCommand(#[from] orbit_core::notes::NoteCommandError),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("a record with this identifier contains different data")]
    Conflict,
    #[error("record not found")]
    NotFound,
    #[error("a terminal job cannot return to a non-terminal state")]
    JobTransition,
    #[error(
        "El almacén orbit.sqlite3 tiene el esquema {version}; esta apertura requiere un esquema entre {min_version} y {max_version}"
    )]
    SchemaVersion {
        version: i64,
        min_version: i64,
        max_version: i64,
    },
    #[error("SQLite must contain the WAL-reset fix (3.51.3 or newer)")]
    SqliteVersion,
    #[error("the requested path contains a symbolic link or an unexpected file type")]
    UnsafePath,
    #[error("stored chunk is missing or corrupt; the recording requires review")]
    CorruptChunk,
}
/// A retryable SQLite writer conflict, preserving the extended result code for
/// the caller's user-visible refusal without exposing SQL or private data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteContention {
    /// Another connection currently owns the WAL writer reservation.
    Busy { code: i32 },
    /// This transaction read a WAL snapshot made stale by a committed writer.
    BusySnapshot { code: i32 },
}
impl Error {
    /// Classify only writer contention; every other storage failure remains
    /// distinguishable through its original typed variant.
    pub fn sqlite_contention(&self) -> Option<SqliteContention> {
        let Self::Database(rusqlite::Error::SqliteFailure(code, _)) = self else {
            return None;
        };
        if code.extended_code == rusqlite::ffi::SQLITE_BUSY_SNAPSHOT {
            Some(SqliteContention::BusySnapshot {
                code: code.extended_code,
            })
        } else if code.code == rusqlite::ErrorCode::DatabaseBusy {
            Some(SqliteContention::Busy {
                code: code.extended_code,
            })
        } else {
            None
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

// SQLite row closures use rusqlite's error type. Recover our typed size refusal
// at that boundary, leaving every other database error and its cause intact.
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::FromSqlConversionFailure(index, kind, cause) => {
                match cause.downcast::<Self>() {
                    Ok(error) => *error,
                    Err(cause) => Self::Database(rusqlite::Error::FromSqlConversionFailure(
                        index, kind, cause,
                    )),
                }
            }
            error => Self::Database(error),
        }
    }
}
