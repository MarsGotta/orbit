mod blocker;
mod download;
mod records;
use records::{
    install_log_refusal, installer_io, migrate_install_record, storage_error, write_update_log,
};

use crate::{AppCommand, AppError, InstallResultSnapshot};
use download::Updater;
use orbit_core::{
    refusal::{InstallLogIssue, Operation, Refusal, UpdateCause},
    update::{InstallRecord, VersionVerdict, compare_running_version},
};
use std::{
    io::{Read, Write},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, BufReader};

// Six hours allows four periodic checks per day of continuous use, finding
// releases during the day without polling the server. Startup also checks;
// this spaces requests, not a deadline for an update.
pub const AUTOMATIC_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

// A build without an update publisher stays offline for updates.
pub const DEFAULT_UPDATE_BASE_URL: &str = match option_env!("ORBIT_UPDATE_BASE_URL") {
    Some(value) => value,
    None => "",
};

// READY follows only local checks. On the owner's Mac, N=80 at load 1.15 had p99
// and worst 13 ms. Five seconds is a ~385x floor for cold disk or busy Macs;
// it bounds stuck local checks; the package download and chain closure follow.
const READY_DEADLINE: Duration = Duration::from_secs(5);
const READY_SIGNAL: &str = "ORBIT_INSTALL_READY";
// Shell rotates over 64 KiB to 32 KiB; twice its threshold tolerates the
// following run. This bounds diagnostics reads, not downloads or installation.
const MAX_INSTALL_LOG_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Disabled,
    Available {
        version: String,
    },
    UpToDate,
    NotNewer {
        installed: String,
        published: String,
    },
    Refused {
        refusal: Refusal,
    },
}

#[derive(Clone)]
pub struct UpdateService {
    base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInstallOutcome {
    pub next_command: AppCommand,
}

impl UpdateService {
    pub fn last_installation_at(home: &Path) -> Result<Option<InstallResultSnapshot>, Refusal> {
        let record = migrate_install_record(home).map_err(storage_error)?;
        let log = home.join("Library/Logs/Orbit/instalador.log");
        match std::fs::metadata(&log) {
            Ok(metadata) => {
                if metadata.len() > MAX_INSTALL_LOG_BYTES {
                    return Err(install_log_refusal(
                        Some(metadata.len()),
                        InstallLogIssue::TooLarge,
                    ));
                }
                let mut file = std::fs::File::open(&log).map_err(|error| {
                    install_log_refusal(
                        Some(metadata.len()),
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
                            Some(metadata.len()),
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
                            return Ok(Some(InstallResultSnapshot {
                                version: (*version != "desconocida").then(|| (*version).to_owned()),
                                when: (*when).to_owned(),
                                failure: (*outcome == "fallo").then(|| (*reason).to_owned()),
                                note: (*outcome == "bien" && *reason != "instalado")
                                    .then(|| (*reason).to_owned()),
                                log_path: Some(log.display().to_string()),
                            }));
                        }
                    }
                }
                return Err(install_log_refusal(
                    Some(body.len() as u64),
                    InstallLogIssue::MissingResult { started_at: None },
                ));
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
                cause: Some(
                    format!("{}: {error}", orbit_core::orbit_bundle::INSTALL_RECORD).into(),
                ),
            }),
        }
    }

    /// Called only after the embedded daemon answered Status: it has opened
    /// the store before publishing its local client. Development and Pruebas
    /// installations do not retire the user's apps.
    pub async fn retire_previous(
        home: &Path,
        current: &Path,
        log: &orbit_ipc::report::LogFile,
    ) -> std::io::Result<()> {
        use orbit_core::orbit_bundle;
        let system = Path::new(orbit_bundle::SYSTEM_APPLICATIONS);
        if current != system.join(orbit_bundle::NAME) {
            return Ok(());
        }
        let result = async {
            let timeout = orbit_core::process_listing::PS_LISTING_TIMEOUT;
            let output = tokio::time::timeout(
                timeout,
                tokio::process::Command::new("ps")
                    .args(["-A", "-ww", "-o", "pid=,ppid=,comm="])
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .map_err(|_| {
                std::io::Error::other(orbit_core::process_listing::ps_timeout_message(timeout))
            })??;
            if !output.status.success() {
                return Err(std::io::Error::other(format!(
                    "ps: estado {}; se conservan las copias",
                    output.status
                )));
            }
            let listing = String::from_utf8(output.stdout).map_err(std::io::Error::other)?;
            retire_previous_from_listing(home, current, system, &listing, log)
        }
        .await;
        if let Err(error) = &result {
            write_update_log(
                log,
                format!("Orbit: se conservan las copias anteriores: {error}"),
            );
        }
        result
    }

    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub async fn check(base_url: &str) -> UpdateState {
        if base_url.trim().is_empty() {
            return UpdateState::Disabled;
        }
        let result = async {
            let updater = Updater::new(base_url)?;
            let manifest = updater.manifest().await?;
            compare_running_version(&manifest, env!("CARGO_PKG_VERSION"))
        }
        .await;
        match result {
            Ok(VersionVerdict::Newer { published }) => {
                UpdateState::Available { version: published }
            }
            Ok(VersionVerdict::Same) => UpdateState::UpToDate,
            Ok(VersionVerdict::NotNewer {
                installed,
                published,
            }) => UpdateState::NotNewer {
                installed,
                published,
            },
            Err(refusal) => UpdateState::Refused { refusal },
        }
    }

    pub async fn check_configured(&self) -> UpdateState {
        Self::check(&self.base_url).await
    }

    pub async fn install(&self) -> Result<UpdateInstallOutcome, AppError> {
        self.install_refusal().await.map_err(|refusal| {
            let mut error = AppError::update(refusal.to_string());
            error.refusal = Some(Box::new(refusal));
            error
        })
    }

    pub async fn install_when_idle(
        &self,
        client: &crate::DaemonClient,
    ) -> Result<Option<UpdateInstallOutcome>, Refusal> {
        if client.composed_snapshot().settings.update.state
            == crate::SettingsUpdateState::Installing
        {
            return Ok(None);
        }
        if let Some(reason) = client.update_installation_blocker().await {
            eprintln!("Orbit: se espera para actualizar: {reason}");
            return Ok(None);
        }
        let previous = client.composed_snapshot().settings.update;
        if previous.state == crate::SettingsUpdateState::Installing {
            return Ok(None);
        }
        client.set_update(crate::UpdateSnapshot {
            state: crate::SettingsUpdateState::Installing,
            version: previous.version.clone(),
            ..crate::UpdateSnapshot::default()
        });
        let updater = Updater::new(&self.base_url)?;
        let (script, verified) = updater.installer_script().await?;
        // A recording may have started while downloading the installer.
        if let Some(reason) = client.update_installation_blocker().await {
            eprintln!("Orbit: se espera para actualizar: {reason}");
            client.set_update(previous);
            return Ok(None);
        }
        self.run_installer(script, verified).await.map(Some)
    }

    pub async fn install_refusal(&self) -> Result<UpdateInstallOutcome, Refusal> {
        let updater = Updater::new(&self.base_url)?;
        let (script, verified) = updater.installer_script().await?;
        self.run_installer(script, verified).await
    }

    async fn run_installer(
        &self,
        script: String,
        verified: bool,
    ) -> Result<UpdateInstallOutcome, Refusal> {
        let mut temporary = tempfile::NamedTempFile::new().map_err(storage_error)?;
        temporary
            .write_all(script.as_bytes())
            .map_err(storage_error)?;
        temporary.flush().map_err(storage_error)?;
        let (_, script_path) = temporary
            .keep()
            .map_err(|error| storage_error(error.error))?;
        let mut child = tokio::process::Command::new("/bin/bash")
            .arg(&script_path)
            .arg("--relaunch")
            .env("ORBIT_INSTALL_BASE", &self.base_url)
            .env("ORBIT_INSTALL_READY_SIGNAL", READY_SIGNAL)
            .env(
                "ORBIT_INSTALLER_VERIFIED",
                if verified { "true" } else { "false" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .map_err(installer_io)?;
        let stdout = child.stdout.take().ok_or_else(|| Refusal::Update {
            cause: UpdateCause::InstallerIo {
                cause: "No se abrió la salida del instalador".to_owned(),
            },
        })?;
        let mut lines = BufReader::new(stdout).lines();
        let ready = async {
            while let Some(line) = lines.next_line().await.map_err(installer_io)? {
                if line == READY_SIGNAL {
                    return Ok(true);
                }
            }
            Ok::<bool, Refusal>(false)
        };
        let ready = match tokio::time::timeout(READY_DEADLINE, ready).await {
            Ok(Ok(ready)) => ready,
            Ok(Err(refusal)) => {
                child.kill().await.map_err(installer_io)?;
                child.wait().await.map_err(installer_io)?;
                std::fs::remove_file(&script_path).map_err(storage_error)?;
                return Err(refusal);
            }
            Err(_) => {
                child.kill().await.map_err(installer_io)?;
                child.wait().await.map_err(installer_io)?;
                std::fs::remove_file(&script_path).map_err(storage_error)?;
                return Err(Refusal::Update {
                    cause: UpdateCause::Deadline {
                        url: "install.sh: anuncio de espera".to_owned(),
                        waited_ms: READY_DEADLINE.as_millis() as u64,
                        deadline_ms: READY_DEADLINE.as_millis() as u64,
                    },
                });
            }
        };
        if !ready {
            let status = child.wait().await.map_err(installer_io)?;
            std::fs::remove_file(&script_path).map_err(storage_error)?;
            return Err(Refusal::Update {
                cause: UpdateCause::InstallerFailed {
                    status: status.code(),
                },
            });
        }
        tokio::spawn(async move {
            loop {
                match lines.next_line().await {
                    Ok(Some(_line)) => {}
                    Ok(None) => break,
                    Err(error) => {
                        eprintln!("install.sh output drain failed: {error}");
                        break;
                    }
                }
            }
            if let Err(error) = child.wait().await {
                eprintln!("install.sh wait failed: {error}");
            }
            if let Err(error) = std::fs::remove_file(script_path) {
                eprintln!("install.sh temporary cleanup failed: {error}");
            }
        });
        Ok(UpdateInstallOutcome {
            next_command: AppCommand::Status(crate::features::status::StatusCommand::Quit),
        })
    }
}

fn retire_previous_from_listing(
    home: &Path,
    current: &Path,
    system: &Path,
    listing: &str,
    log: &orbit_ipc::report::LogFile,
) -> std::io::Result<()> {
    use orbit_core::orbit_bundle::{self, Retirement};
    // Resolve directory aliases before comparing paths: ~/Applications may
    // point at /Applications, in which case its Orbit.app is the current app.
    let current = current.canonicalize()?;
    let system = system.canonicalize()?;
    let user = home.join(orbit_bundle::USER_APPLICATIONS);
    let user = match user.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => user,
        Err(error) => return Err(error),
    };
    for directory in [&system, &user] {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            // Never traverse a symlink to a bundle outside these directories.
            if !entry.file_type()?.is_dir() {
                if orbit_bundle::kind(&entry.path()).is_some() {
                    write_update_log(
                        log,
                        format!(
                            "Orbit: se conserva {}: no es una carpeta de app (puede ser un enlace)",
                            entry.path().display()
                        ),
                    );
                }
                continue;
            }
            let path = entry.path().canonicalize()?;
            match orbit_bundle::retirement(&path, &current, &user, true, Some(listing)) {
                Retirement::Remove => match std::fs::remove_dir_all(&path) {
                    Ok(()) => write_update_log(
                        log,
                        format!(
                            "Orbit: retirada {}: la nueva app abrió el almacén y la copia no está en marcha",
                            path.display()
                        ),
                    ),
                    Err(error) => write_update_log(
                        log,
                        format!("Orbit: se conserva {}: {error}", path.display()),
                    ),
                },
                Retirement::Keep(reason) => write_update_log(
                    log,
                    format!("Orbit: se conserva {}: {reason}", path.display()),
                ),
                Retirement::Unrelated => {}
            }
        }
    }
    // The new receipt has already been read/migrated by setup.
    if home
        .join(orbit_bundle::INSTALL_DATA_DIRECTORY)
        .join(orbit_bundle::INSTALL_RECORD)
        .is_file()
    {
        let legacy = user.join(orbit_bundle::INSTALL_RECORD);
        match std::fs::remove_file(&legacy) {
            Ok(()) => write_update_log(
                log,
                format!("Orbit: registro antiguo retirado: {}", legacy.display()),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => write_update_log(
                log,
                format!(
                    "Orbit: registro antiguo conservado: {}: {error}",
                    legacy.display()
                ),
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    fn shell_limit(name: &str) -> Option<u64> {
        include_str!("../../../scripts/release/install.sh")
            .lines()
            .find_map(|line| line.strip_prefix(name)?.parse().ok())
    }

    #[test]
    fn installer_log_bounds_agree_with_the_script() {
        assert_eq!(shell_limit("LOG_ROTATE_BYTES="), Some(65_536));
        assert_eq!(shell_limit("LOG_KEEP_BYTES="), Some(32_768));
        assert_eq!(
            Some(MAX_INSTALL_LOG_BYTES),
            shell_limit("LOG_ROTATE_BYTES=").map(|bytes| bytes * 2)
        );
    }
}

#[cfg(test)]
mod installation_tests {
    #[test]
    fn retirement_executes_the_decision_in_temporary_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        use orbit_core::orbit_bundle::{
            INSTALL_DATA_DIRECTORY, INSTALL_RECORD, NAME, PARKED_PREFIX,
        };
        let directory = tempfile::tempdir()?;
        let home = directory.path().join("home");
        let system = directory.path().join("system-applications");
        let current = system.join(NAME);
        let previous = system.join(format!("{PARKED_PREFIX}fixture"));
        let legacy = home.join("Applications").join(NAME);
        let receipt = home.join("Applications").join(INSTALL_RECORD);
        std::fs::create_dir_all(&legacy)?;
        std::fs::write(&receipt, "version=fixture")?;
        std::fs::create_dir_all(home.join(INSTALL_DATA_DIRECTORY))?;
        std::fs::write(
            home.join(INSTALL_DATA_DIRECTORY).join(INSTALL_RECORD),
            "version=nueva",
        )?;
        std::fs::create_dir_all(&current)?;
        std::fs::create_dir_all(&previous)?;
        std::fs::write(current.join("marker"), "nueva")?;
        let log = orbit_ipc::report::LogFile::new(
            directory.path().join("updates.log"),
            orbit_ipc::report::MAX_BYTES,
        );
        super::retire_previous_from_listing(&home, &current, &system, "1 0 /sbin/launchd", &log)?;
        assert!(!previous.exists(), "the selected bundle was not removed");
        assert!(!legacy.exists(), "the user bundle was not removed");
        assert!(!receipt.exists(), "the user receipt was not removed");
        assert_eq!(std::fs::read_to_string(current.join("marker"))?, "nueva");
        let diagnostic = std::fs::read_to_string(log.path())?;
        assert!(diagnostic.contains("retirada"));
        assert!(diagnostic.contains(previous.to_str().ok_or("fixture path")?));
        // A directory alias must never turn the current system app into the
        // apparently old user app. The link and target both belong to this fixture.
        std::fs::remove_dir(home.join("Applications"))?;
        std::os::unix::fs::symlink(&system, home.join("Applications"))?;
        super::retire_previous_from_listing(&home, &current, &system, "1 0 /sbin/launchd", &log)?;
        assert_eq!(std::fs::read_to_string(current.join("marker"))?, "nueva");

        Ok(())
    }

    #[tokio::test]
    async fn unknown_capture_never_launches_the_installer() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let paths = orbit_ipc::Paths::resolve(Some(directory.path().into()))?;
        let client = crate::DaemonClient::connect(paths, crate::client_fixture::idle());
        let result = super::UpdateService::new("not-an-origin")
            .install_when_idle(&client)
            .await;
        assert_eq!(result, Ok(None));
        Ok(())
    }
}
