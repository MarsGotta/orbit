//! The pieces a Mac hands the engine: the helper launched as a child process,
//! the Keychain, the process's SIGINT and SIGTERM, and the account's name.
use super::{HelperLink, HelperPipes, HelperProcess, OwnerName, Platform, Secrets, StopSignals};
use crate::config::{Paths, atomic_private_write};
use crate::daemon::ticketed::DeadlineKind;
use anyhow::{Context, Result};
use futures_util::{FutureExt, future::BoxFuture};
use orbit_core::notes as core;
use std::{fs, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    process::{Child, Command},
    signal::unix::{Signal, SignalKind, signal},
};

impl Platform {
    /// What this machine gives the engine. It installs the signals, so it is
    /// called inside a tokio runtime and before anything else starts, so
    /// SIGTERM is never the default kill. Process-wide: inside the app they
    /// are the app's too, and the app installs none of its own, so a SIGTERM
    /// to Orbit closes the engine in order and the app follows it.
    pub fn system(paths: &Paths, development: bool, helper: Option<PathBuf>) -> Result<Platform> {
        let helper = match helper {
            Some(path) => Some(Box::new(ProcessHelper::new(path)?) as Box<dyn HelperLink>),
            None => None,
        };
        let signals = UnixSignals::install()?;
        Ok(Platform {
            helper,
            secrets: Arc::new(SystemSecrets {
                paths: paths.clone(),
                development,
            }),
            signals: Box::new(signals),
            owner: Arc::new(SystemOwner),
        })
    }
}

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
}

impl HelperLink for ProcessHelper {
    fn open(&mut self) -> Result<HelperPipes> {
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
        Ok(HelperPipes {
            reader: Box::new(reader),
            writer: Box::new(writer),
            process: Some(Box::new(HelperChild(child))),
        })
    }

    /// A test asks for a panic by creating `<helper>.panic` beside the helper.
    #[cfg(test)]
    fn panic_trigger(&self) -> Option<PathBuf> {
        Some(self.path.with_extension("panic"))
    }
}

/// The helper's child process; dropping it kills it (`kill_on_drop`).
struct HelperChild(Child);

impl HelperProcess for HelperChild {
    fn wait(&mut self) -> BoxFuture<'_, std::io::Result<std::process::ExitStatus>> {
        self.0.wait().boxed()
    }

    fn kill(&mut self) -> BoxFuture<'_, ()> {
        async move {
            let _ = self.0.kill().await;
        }
        .boxed()
    }
}

/// The Keychain on macOS; with `--dev`, `development-<account>.key` files in
/// the data directory.
pub struct SystemSecrets {
    pub paths: Paths,
    pub development: bool,
}

impl Secrets for SystemSecrets {
    fn get(&self, account: &str) -> Result<Option<String>> {
        get_secret(&self.paths, account, self.development)
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        set_secret(&self.paths, account, value, self.development)
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
}

impl StopSignals for UnixSignals {
    /// The next termination signal, by name. A stream that ended can never
    /// deliver again, so it waits for ever rather than for nothing.
    fn next(&mut self) -> BoxFuture<'_, &'static str> {
        async move {
            tokio::select! {
                Some(()) = self.interrupt.recv() => "SIGINT",
                Some(()) = self.terminate.recv() => "SIGTERM",
                else => std::future::pending::<&'static str>().await,
            }
        }
        .boxed()
    }
}

/// How long the account's full name may take. macOS answers `id -F` from
/// Directory Services in milliseconds; if opendirectoryd hangs, the read hangs
/// with it. Two seconds is far above a healthy answer; meanwhile the hub names
/// the microphone «Micrófono local». It bounds this one read, which runs off
/// the daemon loop.
const ACCOUNT_NAME: DeadlineKind = DeadlineKind::new(
    "account_name",
    Duration::from_secs(2),
    "the account name is read off the loop and never holds a privacy stop",
);

/// The account's full name, read off the loop within [`ACCOUNT_NAME`].
pub struct SystemOwner;

impl OwnerName for SystemOwner {
    fn read(&self) -> BoxFuture<'static, String> {
        account_full_name().boxed()
    }
}

/// The full name of the account (`pw_gecos`) from a program that prints it,
/// `id -F` on macOS: any failure or a late answer is «Yo», never an error.
// Used on macOS, and by its test everywhere.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) async fn read_account_name(program: &std::path::Path, deadline: Duration) -> String {
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
async fn account_full_name() -> String {
    #[cfg(target_os = "macos")]
    {
        read_account_name(std::path::Path::new("/usr/bin/id"), ACCOUNT_NAME.duration()).await
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
        let gecos = match tokio::time::timeout(ACCOUNT_NAME.duration(), read).await {
            Ok(Ok(gecos)) => gecos,
            Ok(Err(_)) | Err(_) => None,
        };
        core::owner_name(gecos.as_deref())
    }
}
