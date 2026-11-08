//! The effects only a Mac has: the native helper launched as a child process,
//! the Keychain, the process's SIGINT and SIGTERM, and the account's name.
//! The engine reaches them behind its `mac` feature, through its own traits.
use anyhow::{Context, Result};
use orbit_core::notes as core;
use orbit_ipc::{Paths, atomic_private_write};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    signal::unix::{Signal, SignalKind, signal},
};

/// The native helper as a child of this process, over its stdin and stdout.
pub struct ProcessHelper {
    path: PathBuf,
}

impl ProcessHelper {
    pub fn new(path: PathBuf) -> Result<Self> {
        if !path.is_absolute() {
            anyhow::bail!("The helper requires an absolute path");
        }
        Ok(Self { path })
    }

    /// Launches the helper and takes its pipes; dropping the child kills it
    /// (`kill_on_drop`).
    pub fn spawn(&self) -> Result<(ChildStdout, ChildStdin, Child)> {
        let mut child = Command::new(&self.path)
            .arg("--stdio")
            .arg("--managed")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let reader = child.stdout.take().context("Helper stdout missing")?;
        let writer = child.stdin.take().context("Helper stdin missing")?;
        Ok((reader, writer, child))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn get_secret(paths: &Paths, account: &str, development: bool) -> Result<Option<String>> {
    if development {
        let path = paths.root.join(format!("development-{account}.key"));
        return match fs::read_to_string(path) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        };
    }
    #[cfg(target_os = "macos")]
    {
        let entry = keyring::Entry::new(orbit_core::orbit_bundle::KEYCHAIN_SERVICE, account)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!(
        "Keychain requiere macOS. Para fixtures aislados utiliza --dev --data-dir /ruta/de/prueba"
    )
}

pub fn set_secret(paths: &Paths, account: &str, value: &str, development: bool) -> Result<()> {
    if development {
        paths.initialize()?;
        return atomic_private_write(
            &paths.root.join(format!("development-{account}.key")),
            value.as_bytes(),
        );
    }
    #[cfg(target_os = "macos")]
    {
        keyring::Entry::new(orbit_core::orbit_bundle::KEYCHAIN_SERVICE, account)?
            .set_password(value)?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("Keychain is unavailable on this platform; the credential was not saved")
}

/// SIGINT and SIGTERM, installed before anything else starts. SIGTERM is what
/// launchd sends at logout and what `kill` sends; SIGINT is a terminal's
/// Ctrl-C. Both used to differ — only SIGINT was heard, so SIGTERM killed the
/// daemon without its closure — and both now open the same door.
pub struct UnixSignals {
    interrupt: Signal,
    terminate: Signal,
}

impl UnixSignals {
    pub fn install() -> Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())
                .context("Could not handle SIGINT for orderly closure")?,
            terminate: signal(SignalKind::terminate())
                .context("Could not handle SIGTERM for orderly closure")?,
        })
    }

    /// The next termination signal, by name. A stream that ended can never
    /// deliver again, so it waits for ever rather than for nothing.
    pub async fn recv(&mut self) -> &'static str {
        tokio::select! {
            Some(()) = self.interrupt.recv() => "SIGINT",
            Some(()) = self.terminate.recv() => "SIGTERM",
            else => std::future::pending::<&'static str>().await,
        }
    }
}

/// The full name of the account (`pw_gecos`) from a program that prints it,
/// `id -F` on macOS: any failure or a late answer is «Yo», never an error.
pub async fn read_account_name(program: &std::path::Path, deadline: Duration) -> String {
    let output = tokio::time::timeout(
        deadline,
        tokio::process::Command::new(program)
            .arg("-F")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let gecos = match output {
        Ok(Ok(output)) if output.status.success() => String::from_utf8(output.stdout).ok(),
        Ok(Ok(_) | Err(_)) | Err(_) => None,
    };
    core::owner_name(gecos.as_deref())
}

/// macOS keeps accounts in Directory Services, so `/etc/passwd` does not hold
/// them there; elsewhere (development) `/etc/passwd` is read.
pub async fn account_full_name(deadline: Duration) -> String {
    #[cfg(target_os = "macos")]
    {
        read_account_name(std::path::Path::new("/usr/bin/id"), deadline).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let read = tokio::task::spawn_blocking(|| {
            let uid = rustix::process::getuid().as_raw().to_string();
            std::fs::read_to_string("/etc/passwd")
                .ok()
                .and_then(|passwd| {
                    passwd.lines().find_map(|line| {
                        let fields = line.split(':').collect::<Vec<_>>();
                        if fields.get(2) == Some(&uid.as_str()) {
                            fields.get(4).map(|gecos| (*gecos).to_owned())
                        } else {
                            None
                        }
                    })
                })
        });
        let gecos = match tokio::time::timeout(deadline, read).await {
            Ok(Ok(gecos)) => gecos,
            Ok(Err(_)) | Err(_) => None,
        };
        core::owner_name(gecos.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    /// The account's name labels the user's microphone on the hub: read from the
    /// program that prints it, “Me” when it fails or is late.
    #[tokio::test]
    async fn the_account_name_is_read_with_a_deadline_and_uses_the_english_fallback_when_late()
    -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir()?;
        let script = |name: &str, body: &str| -> Result<std::path::PathBuf> {
            let path = directory.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            Ok(path)
        };
        let fast = script("fast-id", "echo 'Ada Lovelace,,,'")?;
        let slow = script("slow-id", "sleep 30; echo 'Nunca'")?;
        let failing = script("failing-id", "exit 1")?;
        // The programs that finish are not raced against the clock: macOS assesses
        // a freshly written executable on its first exec, measured at 108–297 ms
        // on an idle Mac (11–17 ms after), so a short deadline decided the result.
        // The call returns when the program exits; this bound is never reached.
        let unhurried = std::time::Duration::from_secs(60);
        assert_eq!(
            super::read_account_name(&fast, unhurried).await,
            "Ada Lovelace"
        );
        assert_eq!(super::read_account_name(&failing, unhurried).await, "Me");
        // Had it waited for the program, the name would be «Nunca».
        let deadline = std::time::Duration::from_millis(300);
        assert_eq!(super::read_account_name(&slow, deadline).await, "Me");
        Ok(())
    }
}
