//! The install record and log the installer leaves, and the refusals that
//! name what went wrong with them.
use super::{MAX_INSTALL_LOG_BYTES, UpdateService, UpdateState};
use orbit_core::refusal::{InstallLogIssue, Refusal, UpdateCause};
use std::path::Path;

impl UpdateService {
    /// A periodic check: a refusal goes to the update log, never to the person.
    pub async fn check_automatically(
        &self,
        log: &orbit_ipc::report::LogFile,
    ) -> Option<UpdateState> {
        match self.check_configured().await {
            UpdateState::Refused { refusal } => {
                write_update_log(log, format!("Automatic Orbit update check: {refusal}"));
                None
            }
            state @ (UpdateState::Disabled
            | UpdateState::Available { .. }
            | UpdateState::UpToDate
            | UpdateState::NotNewer { .. }) => Some(state),
        }
    }
}

pub(super) fn write_update_log(log: &orbit_ipc::report::LogFile, message: String) {
    eprintln!("{message}");
    if let Err(error) = log.append(format!("{message}\n").as_bytes()) {
        eprintln!("Could not write {}: {error}", log.path().display());
    }
}

pub(super) fn migrate_install_record(home: &Path) -> std::io::Result<std::path::PathBuf> {
    use orbit_core::orbit_bundle::{INSTALL_DATA_DIRECTORY, INSTALL_RECORD, USER_APPLICATIONS};
    let record = home.join(INSTALL_DATA_DIRECTORY).join(INSTALL_RECORD);
    match std::fs::metadata(&record) {
        Ok(_) => return Ok(record),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let legacy = home.join(USER_APPLICATIONS).join(INSTALL_RECORD);
    match std::fs::metadata(&legacy) {
        Ok(_) => {
            std::fs::create_dir_all(home.join(INSTALL_DATA_DIRECTORY))?;
            std::fs::rename(legacy, &record)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(record)
}

pub(super) fn install_log_refusal(bytes: Option<u64>, reason: InstallLogIssue) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::InstallLog {
            bytes,
            max_bytes: MAX_INSTALL_LOG_BYTES,
            reason,
        },
    }
}

pub(super) fn storage_error(error: std::io::Error) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::InstallerIo {
            cause: error.to_string(),
        },
    }
}

pub(super) fn installer_io(error: std::io::Error) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::InstallerIo {
            cause: error.to_string(),
        },
    }
}
