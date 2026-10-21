//! The daemon inside the app (SIM-51): one process on the Mac, plus the helper.
//!
//! The daemon runs its own runtime on a thread. The app shares its typed
//! call queue and topic watches; only Orbit Pruebas exposes `orbit.sock`.
//! If the daemon ends without anyone asking, the app ends and says why.
use anyhow::{Context, Result, bail};
use orbit_core::orbit_bundle::{self, Member};
use orbit_core::protocol::Command;
use orbit_ipc::Paths;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// How long a daemon just started has to answer its first `Status`. A cold
/// daemon under the whole test suite answers in about half a second; this is
/// the bound the app has always used, and a daemon that misses it fails the
/// app's start with the time it waited, rather than being waited for without
/// end. `scripts/e2e-mac/prueba.py` reads this number for its «daemon» wait.
pub const DAEMON_READY_WAIT: Duration = Duration::from_secs(15);

/// How long quitting waits for the daemon's thread to end after asking it to
/// shut down, before the app exits anyway.
///
/// What it protects: the daemon's durable closure, which ends in this order:
/// the closure itself within `SHUTDOWN_BUDGET` (10 s), the critical loss
/// declaration within `SNAPSHOT_RECEIPT_GRACE` (1128 ms), the helper's own exit
/// within `HELPER_EXIT_GRACE` (3 s) and the last error reports within
/// `EXIT_FLUSH` (2 s): 16,128 ms at worst. The 872 ms left cover the runtime's
/// own drop. The assertion below compares it with those four constants, so a
/// change to any of them that outgrows this wait does not compile.
///
/// Why it is 17 s and no longer 15: the old wait was the first three plus
/// 872 ms and left out `EXIT_FLUSH`, so a closure that used its whole budget
/// with reports queued was cut during the flush. `install.sh --relaunch` waits
/// 30 s for the app, more than this.
///
/// What it is not: a pause on every quit. The wait ends when the thread ends,
/// which an ordinary closure does in well under a second. Nor is it a kill:
/// a thread cannot be killed, so at the deadline the app exits and the exit of
/// the process ends the daemon where it was, as the old group kill did.
/// `scripts/e2e-mac/prueba.py` reads this number for its «cerrar» wait.
pub const DAEMON_EXIT_WAIT: Duration = Duration::from_secs(17);

const _: () = assert!(
    DAEMON_EXIT_WAIT.as_millis()
        > orbit_daemon::daemon::SHUTDOWN_BUDGET.as_millis()
            + orbit_daemon::daemon::SNAPSHOT_RECEIPT_GRACE.as_millis()
            + orbit_daemon::native::HELPER_EXIT_GRACE.as_millis()
            + orbit_daemon::error_reports::EXIT_FLUSH.as_millis()
);

/// The daemon's thread, by this name in a crash report and in `ps -T`.
pub const THREAD_NAME: &str = "orbit-daemon";

/// The stack of the daemon's thread. Eight MiB is the stack the daemon's
/// top-level future had when it ran on the `orbit` binary's main thread (the
/// default main-thread stack on macOS and Linux); a spawned thread gets 2 MiB,
/// which REF-39 measured too small for a daemon start in a test fixture. It is
/// the same stack as before, not a new limit on anything the daemon does.
const DAEMON_STACK_BYTES: usize = 8 * 1024 * 1024;

/// How the daemon is started: the packaged one with the helper beside the app,
/// or the development one, outside a bundle, without a helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub development: bool,
    pub helper: Option<PathBuf>,
}

impl Launch {
    pub fn development() -> Self {
        Self {
            development: true,
            helper: None,
        }
    }

    /// From the app's own executable, already canonical. Inside `Orbit.app`
    /// the helper is `Contents/MacOS/OrbitNative` of that same bundle; an
    /// executable in some other `.app` is refused by name; anything else is a
    /// development build.
    pub fn for_executable(executable: &Path) -> Result<Self> {
        if let Some(bundle) = orbit_bundle::bundle_of(executable, Member::App) {
            let helper = orbit_bundle::member_path(&bundle, Member::Helper)
                .context("Falta la ruta del helper en el bundle")?;
            if !helper.is_file() {
                bail!(
                    "El bundle de Orbit no contiene el helper esperado en {}",
                    helper.display()
                );
            }
            return Ok(Self {
                development: false,
                helper: Some(helper),
            });
        }
        if let Some(bundle) = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            && bundle
                .extension()
                .is_some_and(|extension| extension == "app")
        {
            if orbit_bundle::kind(bundle).is_some() {
                bail!(
                    "La aplicación se abrió desde {}, pero {} no es el ejecutable principal {}",
                    bundle.display(),
                    executable.display(),
                    Member::App.relative_path()
                );
            }
            bail!(
                "La aplicación se abrió desde {} y no es el bundle esperado {}",
                bundle.display(),
                orbit_bundle::NAME
            );
        }
        Ok(Self::development())
    }
}

pub use orbit_ipc::DaemonEnded as Ended;

/// What quitting found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Quit {
    Ended(Ended),
    /// The thread was still running at [`DAEMON_EXIT_WAIT`].
    DeadlinePassed {
        waited_ms: u128,
    },
}

#[derive(Clone)]
pub struct EmbeddedDaemon {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    ended: watch::Receiver<Option<Ended>>,
    local: watch::Receiver<Option<orbit_ipc::LocalClient>>,
    quitting: AtomicBool,
    asked: AtomicBool,
    /// The daemon's thread, so a test can tell it by its name on any
    /// platform (Linux's `/proc` is not the Mac's).
    #[cfg(test)]
    thread: std::thread::Thread,
}

impl EmbeddedDaemon {
    /// The daemon itself owns daemon.lock; a racing start cannot open storage.
    pub async fn start(paths: Paths, launch: Launch) -> Result<Self> {
        let daemon = Self::spawn(paths, launch)?;
        tokio::time::timeout(DAEMON_READY_WAIT, daemon.client())
            .await
            .with_context(|| {
                format!(
                    "El servicio no entregó su canal en {} ms",
                    DAEMON_READY_WAIT.as_millis()
                )
            })??;
        Ok(daemon)
    }

    pub(crate) fn spawn(paths: Paths, launch: Launch) -> Result<Self> {
        let (ended_tx, ended) = watch::channel(None);
        let daemon_paths = paths.clone();
        let thread_end = ended.clone();
        let (local_tx, local) = watch::channel(None);
        let (connected, connection) = tokio::sync::oneshot::channel();
        let handle = std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .stack_size(DAEMON_STACK_BYTES)
            .spawn(move || {
                // The daemon's own runtime, as `#[tokio::main]` built it for
                // the `orbit` binary; the top-level future runs here, on this
                // thread, so it need not be `Send`.
                let ended = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        let result = runtime.block_on(async {
                            let run = orbit_daemon::daemon::run_embedded(
                                daemon_paths,
                                launch.development,
                                launch.helper,
                                connected,
                                thread_end,
                            );
                            let forward = async move {
                                if let Ok(client) = connection.await {
                                    local_tx.send_replace(Some(client));
                                }
                            };
                            let (result, ()) = tokio::join!(run, forward);
                            result
                        });
                        // Waits for its blocking tasks, as the end of the
                        // binary's `main` did, before the end is told.
                        drop(runtime);
                        match result {
                            Ok(()) => Ended::Closed,
                            Err(error) => Ended::Failed(format!("{error:#}")),
                        }
                    }
                    Err(error) => Ended::Failed(format!("no se pudo crear su runtime: {error}")),
                };
                ended_tx.send_replace(Some(ended));
            })
            .context("No se pudo arrancar el hilo del servicio de Orbit")?;
        let daemon = Self {
            inner: Arc::new(Inner {
                paths,
                ended,
                local,
                quitting: AtomicBool::new(false),
                asked: AtomicBool::new(false),
                #[cfg(test)]
                thread: handle.thread().clone(),
            }),
        };
        // The thread runs detached: its end is told on `ended`, never joined.
        drop(handle);
        Ok(daemon)
    }

    pub async fn client(&self) -> Result<orbit_ipc::LocalClient> {
        let mut local = self.inner.local.clone();
        let client = match local.wait_for(Option::is_some).await {
            Ok(value) => value.clone(),
            Err(_) => None,
        };
        if let Some(client) = client {
            return Ok(client);
        }
        bail!(
            "El servicio no arrancó; cerrojo {}: {}",
            self.inner.paths.root.join("daemon.lock").display(),
            self.ended().await.describe()
        )
    }

    /// As before, accepted Status establishes that the actor is serving. It
    /// also serves Status and the privacy stop during startup recovery; the
    /// app must be available then, without waiting for disk recovery to end.
    pub async fn wait_until_ready(&self) -> Result<()> {
        let ready = async {
            self.client()
                .await?
                .request_command(Command::Status)
                .await?;
            Ok::<(), anyhow::Error>(())
        };
        tokio::time::timeout(DAEMON_READY_WAIT, ready)
            .await
            .with_context(|| {
                format!(
                    "El servicio de Orbit no respondió en {} ms",
                    DAEMON_READY_WAIT.as_millis()
                )
            })??;
        eprintln!("daemon ready");
        Ok(())
    }

    /// When the thread ends, and how.
    pub async fn ended(&self) -> Ended {
        let mut ended = self.inner.ended.clone();
        match ended.wait_for(Option::is_some).await {
            Ok(value) => value.clone().unwrap_or(Ended::Vanished),
            Err(_) => Ended::Vanished,
        }
    }

    /// Whether someone asked the daemon to stop: an end after that is the
    /// answer, not a fall.
    pub fn quitting(&self) -> bool {
        self.inner.quitting.load(Ordering::SeqCst)
    }

    /// Quits: asks the daemon for `Shutdown` once, however many times this is
    /// called (`exit` fires `ExitRequested` again), and waits for its thread
    /// to end, at most [`DAEMON_EXIT_WAIT`] from the call.
    ///
    /// The reply to `Shutdown` arrives only after the closure, so the wait is
    /// for the thread, not for the reply. During the startup recovery the
    /// daemon refuses `Shutdown` (it serves only `Status`, subscriptions and
    /// the privacy stop); the refusal is said, the wait runs to its deadline
    /// and the app exits then, which ends the recovery where it was. The next
    /// start recovers again, as after any crash.
    pub async fn shutdown(&self) -> Quit {
        self.inner.quitting.store(true, Ordering::SeqCst);
        if let Some(ended) = self.inner.ended.borrow().clone() {
            return Quit::Ended(ended);
        }
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + DAEMON_EXIT_WAIT;
        let first = !self.inner.asked.swap(true, Ordering::SeqCst);
        let daemon = self.clone();
        let ask = async move {
            if !first {
                return;
            }
            let result = async {
                daemon
                    .client()
                    .await?
                    .request_command(Command::Shutdown)
                    .await
            }
            .await;
            if let Err(error) = result {
                eprintln!("daemon: la orden de cierre no se confirmó: {error:#}");
            }
        };
        let end = self.ended();
        let wait = async {
            tokio::pin!(ask);
            tokio::pin!(end);
            let mut asked = false;
            loop {
                tokio::select! {
                    biased;
                    ended = &mut end => return ended,
                    () = &mut ask, if !asked => asked = true,
                }
            }
        };
        match tokio::time::timeout_at(deadline, wait).await {
            Ok(ended) => Quit::Ended(ended),
            Err(_) => {
                let waited_ms = started.elapsed().as_millis();
                eprintln!(
                    "daemon: su hilo no terminó en {waited_ms} ms tras pedirle el cierre; Orbit sale igualmente"
                );
                Quit::DeadlinePassed { waited_ms }
            }
        }
    }
}

#[cfg(test)]
#[path = "daemon_thread_tests.rs"]
pub(crate) mod tests;
