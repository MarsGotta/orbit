//! The last installation's result, as Ajustes shows it: the installer's log
//! when there is one, otherwise the install record.
use super::{
    MAX_INSTALL_LOG_BYTES, UpdateService,
    records::{install_log_refusal, migrate_install_record, storage_error},
};
use crate::InstallResultSnapshot;
use orbit_core::{
    refusal::{InstallLogIssue, Operation, Refusal},
    update::InstallRecord,
};
use std::{io::Read, path::Path};

impl UpdateService {
    pub fn last_installation_at(home: &Path) -> Result<Option<InstallResultSnapshot>, Refusal> {
        let record = migrate_install_record(home).map_err(storage_error)?;
        let log = home.join("Library/Logs/Orbit/instalador.log");
        match std::fs::metadata(&log) {
            Ok(metadata) => {
                let body = read_installer_log(&log, metadata.len())?;
                return last_result(&body, &log).map(Some);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(install_log_refusal(
                    None,
                    InstallLogIssue::Io {
                        detail: error.to_string(),
                    },
                ));
            }
        }
        read_install_record(&record)
    }
}

/// The installer's log of `length` bytes, refused when it is over its bound
/// or cannot be read.
fn read_installer_log(log: &Path, length: u64) -> Result<String, Refusal> {
    if length > MAX_INSTALL_LOG_BYTES {
        return Err(install_log_refusal(Some(length), InstallLogIssue::TooLarge));
    }
    let mut file = std::fs::File::open(log).map_err(|error| {
        install_log_refusal(
            Some(length),
            InstallLogIssue::Io {
                detail: error.to_string(),
            },
        )
    })?;
    let mut body = String::new();
    (&mut file)
        .take(MAX_INSTALL_LOG_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| {
            install_log_refusal(
                Some(length),
                InstallLogIssue::Io {
                    detail: error.to_string(),
                },
            )
        })?;
    if body.len() as u64 > MAX_INSTALL_LOG_BYTES {
        return Err(install_log_refusal(
            Some(body.len() as u64),
            InstallLogIssue::TooLarge,
        ));
    }
    Ok(body)
}

/// The log's last run: its `resultado|` line, or a refusal when the last run
/// started (`Inicio`) and wrote none, or no run wrote one.
fn last_result(body: &str, log: &Path) -> Result<InstallResultSnapshot, Refusal> {
    for line in body.lines().rev() {
        if let Some(started_at) = line.strip_prefix("Inicio ") {
            return Err(install_log_refusal(
                Some(body.len() as u64),
                InstallLogIssue::MissingResult {
                    started_at: Some(started_at.to_owned()),
                },
            ));
        }
        if line.starts_with("resultado|") {
            let fields: Vec<_> = line.splitn(5, '|').collect();
            if let [
                "resultado",
                version,
                when,
                outcome @ ("bien" | "fallo"),
                reason,
            ] = fields.as_slice()
            {
                return Ok(InstallResultSnapshot {
                    version: (*version != "desconocida").then(|| (*version).to_owned()),
                    when: (*when).to_owned(),
                    failure: (*outcome == "fallo").then(|| (*reason).to_owned()),
                    note: (*outcome == "bien" && *reason != "instalado")
                        .then(|| (*reason).to_owned()),
                    log_path: Some(log.display().to_string()),
                });
            }
        }
    }
    Err(install_log_refusal(
        Some(body.len() as u64),
        InstallLogIssue::MissingResult { started_at: None },
    ))
}

/// The installation the install record names, for a Mac whose installer left
/// no log.
fn read_install_record(record: &Path) -> Result<Option<InstallResultSnapshot>, Refusal> {
    match std::fs::read_to_string(record) {
        Ok(body) => {
            let parsed = InstallRecord::parse(&body);
            Ok(parsed
                .fields
                .get("installed")
                .map(|when| InstallResultSnapshot {
                    version: parsed.version.map(|version| version.to_string()),
                    when: when.clone(),
                    failure: None,
                    note: None,
                    log_path: None,
                }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Refusal::StorageFailed {
            operation: Operation::Storage,
            cause: Some(format!("{}: {error}", orbit_core::orbit_bundle::INSTALL_RECORD).into()),
        }),
    }
}
