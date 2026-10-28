//! Starting the installer: the script is written to a file it keeps, run
//! detached from this app, and taken as started once it says it is ready.
use super::{
    READY_DEADLINE, READY_SIGNAL, UpdateInstallOutcome, UpdateService,
    records::{installer_io, storage_error},
};
use crate::AppCommand;
use orbit_core::refusal::{Refusal, UpdateCause};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout},
};

type InstallerLines = Lines<BufReader<ChildStdout>>;

impl UpdateService {
    pub(super) async fn run_installer(
        &self,
        script: String,
        verified: bool,
    ) -> Result<UpdateInstallOutcome, Refusal> {
        let script_path = keep_script(&script)?;
        let mut child = self.spawn_installer(&script_path, verified)?;
        let stdout = child.stdout.take().ok_or_else(|| Refusal::Update {
            cause: UpdateCause::InstallerIo {
                cause: "The installer output could not be opened".to_owned(),
            },
        })?;
        let mut lines = BufReader::new(stdout).lines();
        let ready = match tokio::time::timeout(READY_DEADLINE, ready_signal(&mut lines)).await {
            Ok(Ok(ready)) => ready,
            Ok(Err(refusal)) => {
                stop_installer(&mut child, &script_path).await?;
                return Err(refusal);
            }
            Err(_) => {
                stop_installer(&mut child, &script_path).await?;
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
        tokio::spawn(follow_installer(lines, child, script_path));
        Ok(UpdateInstallOutcome {
            next_command: AppCommand::Status(crate::features::status::StatusCommand::Quit),
        })
    }

    /// The installer, with its origin and whether its script was verified;
    /// it outlives this app, which it relaunches.
    fn spawn_installer(&self, script_path: &Path, verified: bool) -> Result<Child, Refusal> {
        tokio::process::Command::new("/bin/bash")
            .arg(script_path)
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
            .map_err(installer_io)
    }
}

/// The script in a temporary file kept on disk for the detached installer
/// to run; it is removed once the installer is stopped or has ended.
fn keep_script(script: &str) -> Result<PathBuf, Refusal> {
    let mut temporary = tempfile::NamedTempFile::new().map_err(storage_error)?;
    temporary
        .write_all(script.as_bytes())
        .map_err(storage_error)?;
    temporary.flush().map_err(storage_error)?;
    let (_, script_path) = temporary
        .keep()
        .map_err(|error| storage_error(error.error))?;
    Ok(script_path)
}

/// Whether the installer said it is ready before its output ended.
async fn ready_signal(lines: &mut InstallerLines) -> Result<bool, Refusal> {
    while let Some(line) = lines.next_line().await.map_err(installer_io)? {
        if line == READY_SIGNAL {
            return Ok(true);
        }
    }
    Ok(false)
}

/// An installer that never said it was ready: it is stopped, and its script
/// removed.
async fn stop_installer(child: &mut Child, script_path: &Path) -> Result<(), Refusal> {
    child.kill().await.map_err(installer_io)?;
    child.wait().await.map_err(installer_io)?;
    std::fs::remove_file(script_path).map_err(storage_error)?;
    Ok(())
}

/// A ready installer runs on: its output is drained so it never blocks on a
/// full pipe, and its script removed once it ends.
async fn follow_installer(mut lines: InstallerLines, mut child: Child, script_path: PathBuf) {
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
}
