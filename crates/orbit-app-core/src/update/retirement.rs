//! Retiring the copies an update leaves behind, once the new app runs: the
//! parked and older Orbit bundles nothing runs, and the old install record.
use super::{UpdateService, records::write_update_log};
use std::path::Path;

impl UpdateService {
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
}

fn retire_previous_from_listing(
    home: &Path,
    current: &Path,
    system: &Path,
    listing: &str,
    log: &orbit_ipc::report::LogFile,
) -> std::io::Result<()> {
    use orbit_core::orbit_bundle;
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
        retire_in(directory, &current, &user, listing, log)?;
    }
    retire_legacy_record(home, &user, log);
    Ok(())
}

/// Each Orbit bundle in `directory` that the decision retires is removed;
/// every one kept says why in the update log.
fn retire_in(
    directory: &Path,
    current: &Path,
    user: &Path,
    listing: &str,
    log: &orbit_ipc::report::LogFile,
) -> std::io::Result<()> {
    use orbit_core::orbit_bundle::{self, Retirement};
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
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
        match orbit_bundle::retirement(&path, current, user, true, Some(listing)) {
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
    Ok(())
}

/// The install record of before, next to the user's apps, once the new one
/// is in place.
fn retire_legacy_record(home: &Path, user: &Path, log: &orbit_ipc::report::LogFile) {
    use orbit_core::orbit_bundle;
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
}

#[cfg(test)]
mod tests {
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
}
