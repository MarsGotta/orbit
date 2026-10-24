mod blocker;
mod download;
mod installer;
mod last_installation;
mod records;
mod retirement;

use crate::{AppCommand, AppError};
use download::Updater;
use orbit_core::{
    refusal::Refusal,
    update::{VersionVerdict, compare_running_version},
};
use std::time::Duration;

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
