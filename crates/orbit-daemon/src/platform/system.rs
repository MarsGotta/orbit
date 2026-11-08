//! The pieces a Mac hands the engine: `orbit-mac`'s effects behind the traits
//! of [`super`], and how long the account's name may take.
use super::{HelperLink, HelperPipes, HelperProcess, OwnerName, Platform, Secrets, StopSignals};
use crate::config::Paths;
use crate::daemon::ticketed::DeadlineKind;
use anyhow::Result;
use futures_util::{FutureExt, future::BoxFuture};
pub use orbit_mac::{ProcessHelper, UnixSignals};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::process::Child;

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

impl HelperLink for orbit_mac::ProcessHelper {
    fn open(&mut self) -> Result<HelperPipes> {
        let (reader, writer, child) = self.spawn()?;
        Ok(HelperPipes {
            reader: Box::new(reader),
            writer: Box::new(writer),
            process: Some(Box::new(HelperChild(child))),
        })
    }

    /// A test asks for a panic by creating `<helper>.panic` beside the helper.
    #[cfg(test)]
    fn panic_trigger(&self) -> Option<PathBuf> {
        Some(self.path().with_extension("panic"))
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
        orbit_mac::get_secret(&self.paths, account, self.development)
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        orbit_mac::set_secret(&self.paths, account, value, self.development)
    }
}

impl StopSignals for orbit_mac::UnixSignals {
    /// The next termination signal, by name. A stream that ended can never
    /// deliver again, so it waits for ever rather than for nothing.
    fn next(&mut self) -> BoxFuture<'_, &'static str> {
        self.recv().boxed()
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
        orbit_mac::account_full_name(ACCOUNT_NAME.duration()).boxed()
    }
}
