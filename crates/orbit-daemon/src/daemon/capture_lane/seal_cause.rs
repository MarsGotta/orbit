//! Why a seal failed, as a code the notice carries (`cause`): the person is
//! told what to do about a full disk, a folder macOS will not let Orbit
//! write, or a store it cannot verify, never only that something failed.
use std::io::ErrorKind;

/// The seal's destination in this Mac is not a private regular file.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(in crate::daemon) struct DestinationInvalid(pub(in crate::daemon) &'static str);

/// The reserve of unsealed audio is full: what did not fit is a declared gap.
#[derive(Debug, thiserror::Error)]
#[error(
    "Se agotó la reserva de audio; los fragmentos anteriores siguen pendientes y la pérdida adicional está declarada"
)]
pub(in crate::daemon) struct ReserveExhausted;

fn io_cause(kind: ErrorKind) -> &'static str {
    if matches!(
        kind,
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded | ErrorKind::FileTooLarge
    ) {
        "disk_full"
    } else if matches!(
        kind,
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
    ) {
        "not_writable"
    } else {
        "disk_error"
    }
}

fn store_cause(error: &orbit_store::Error) -> &'static str {
    if let orbit_store::Error::Io(error) = error {
        return io_cause(error.kind());
    }
    if error.sqlite_contention().is_some() {
        return "store_busy";
    }
    if matches!(
        error,
        orbit_store::Error::Authentication | orbit_store::Error::CorruptChunk
    ) {
        return "integrity";
    }
    if matches!(error, orbit_store::Error::UnsafePath) {
        return "destination_invalid";
    }
    "store_error"
}

/// The cause of a failed seal, from the first error in its chain this knows.
pub(in crate::daemon) fn seal_cause(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(store) = cause.downcast_ref::<orbit_store::Error>() {
            return store_cause(store);
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return io_cause(io.kind());
        }
        if cause.is::<DestinationInvalid>() {
            return "destination_invalid";
        }
        if cause.is::<ReserveExhausted>() {
            return "reserve_exhausted";
        }
    }
    // An admission the writer refused (its queue full, or it stopped).
    "writer_unavailable"
}
