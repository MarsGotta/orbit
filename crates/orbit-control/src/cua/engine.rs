//! The engine process: Orbit starts it in a process group of its own and uses
//! its socket only once the process listening there descends from it.
#[cfg(target_os = "macos")]
use super::previous_listener::{engine_process_listing, inspect_previous_socket};
use anyhow::{Context, Result, bail};
use orbit_core::topics::control::{EngineCleanup, EngineCleanupOutcome};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    time::Instant,
};

/// The engine subcommand that binds the socket Orbit hands it. Named once so a
/// driver that renames it is corrected in a single place.
const DRIVER_DAEMON_SUBCOMMAND: &str = "serve";
/// Bound process ancestry queries during CUA startup. The packaged 0.28.2
/// driver on macOS was measured with one extra `serve` process (43904 ->
/// 43906); four links allow three more wrappers while limiting local process
/// probes to four. This is not a timeout or proof of a binary's identity.
const MAX_ENGINE_PARENT_HOPS: usize = 4;
const ENGINE_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const ENGINE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The driver executable, checked for the same trust the bundle guarantees.
pub(super) fn verified_driver_binary(configured: &Path) -> Result<PathBuf> {
    let binary = std::fs::canonicalize(configured)
        .with_context(|| format!("No se encontró el driver CUA en {}", configured.display()))?;
    let metadata = std::fs::metadata(&binary)?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || (metadata.uid() != 0 && metadata.uid() != rustix::process::geteuid().as_raw())
    {
        bail!(
            "El ejecutable CUA debe ser un archivo de confianza, sin escritura para grupo u otros"
        );
    }
    Ok(binary)
}

/// A child of Orbit, in embedded mode, with the environment scrubbed. Being a
/// direct child is what keeps the engine inside Orbit's permission grant.
pub(super) fn driver_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| {
            matches!(
                name.to_str(),
                Some("HOME" | "PATH" | "TMPDIR" | "LANG" | "LC_ALL")
            )
        }))
        // Both forms are intentional: embedded mode forbids daemon autostart,
        // which is why Orbit starts the engine itself below.
        .env("CUA_DRIVER_EMBEDDED", "1")
        .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0")
        .kill_on_drop(true);
    command
}

/// A driver may fork its socket listener. The group belongs solely to this
/// connection, including failures before the MCP proxy is ready.
pub(super) struct EngineGroup {
    pub(super) child: Child,
    pub(super) pgid: rustix::process::Pid,
    pub(super) cleanup: Vec<EngineCleanup>,
    pub(super) inspection_notice: Option<String>,
}

#[derive(Debug)]
pub(crate) enum PeerProbeFact {
    CredentialError(std::io::Error),
    PidMissing,
    InvalidPid(i32),
    Ancestry {
        peer: i32,
        result: Result<Option<usize>, anyhow::Error>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PeerProbeDecision {
    Accept,
    Retry(String),
    Reject(String),
}

/// Decide solely from facts observed on one connected probe. Unknown identity
/// is never success; a known foreign identity is never a reason to keep polling.
pub(crate) fn decide_peer_probe(
    fact: PeerProbeFact,
    child_pid: u32,
    socket: &Path,
) -> PeerProbeDecision {
    match fact {
        PeerProbeFact::CredentialError(error) => {
            PeerProbeDecision::Retry(format!("peer_cred() falló: {error}"))
        }
        PeerProbeFact::PidMissing => {
            PeerProbeDecision::Retry("peer_cred().pid() no devolvió un PID".to_owned())
        }
        PeerProbeFact::InvalidPid(peer) => PeerProbeDecision::Reject(format!(
            "El nuevo socket CUA informó un PID inválido: {peer}"
        )),
        PeerProbeFact::Ancestry {
            result: Err(error), ..
        } => PeerProbeDecision::Retry(format!("la consulta del parentesco falló: {error:#}")),
        PeerProbeFact::Ancestry {
            result: Ok(None),
            peer,
        } => PeerProbeDecision::Reject(format!(
            "El nuevo socket CUA ({}) escucha al PID {peer}, pero no desciende del motor que lanzó Orbit (PID {child_pid}) en {MAX_ENGINE_PARENT_HOPS} pasos de parentesco; no se usa",
            socket.display()
        )),
        PeerProbeFact::Ancestry {
            result: Ok(Some(_)),
            ..
        } => PeerProbeDecision::Accept,
    }
}

impl Drop for EngineGroup {
    fn drop(&mut self) {
        match rustix::process::kill_process_group(self.pgid, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
            Err(error) => eprintln!(
                "Could not stop the CUA engine group (PGID {}): {error}",
                self.pgid.as_raw_pid()
            ),
        }
    }
}

/// A socket left by another daemon is not by itself proof that its listener is
/// our engine. On macOS a bounded census must prove the exact bundle, user and
/// orphaned ancestry before signaling; otherwise only retire the pathname.
pub(super) async fn start_engine(binary: &Path, socket: &Path) -> Result<EngineGroup> {
    #[cfg(target_os = "macos")]
    let inspection = inspect_previous_socket(
        binary,
        socket,
        engine_process_listing,
        rustix::process::kill_process,
        tokio::time::sleep,
    )
    .await;
    #[cfg(not(target_os = "macos"))]
    let engine_cleanup = Vec::new();
    #[cfg(target_os = "macos")]
    let engine_cleanup = inspection.cleanups;
    #[cfg(not(target_os = "macos"))]
    let inspection_notice = None;
    #[cfg(target_os = "macos")]
    let inspection_notice = inspection.notice;
    start_engine_after_inspection(socket, engine_cleanup, inspection_notice, || {
        let mut command = driver_command(binary);
        command
            .arg(DRIVER_DAEMON_SUBCOMMAND)
            .arg("--socket")
            .arg(socket)
            // The packaged macOS 0.28.2 driver forks A -> B; B keeps serving if
            // only A is killed. A new group confines cleanup to this connection.
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
            .spawn()
            .context("No se pudo iniciar el motor CUA empaquetado")
    })
    .await
}

/// Starts the engine once the previous socket was inspected. A start that
/// fails after the sweep says what the sweep had already done.
pub(super) async fn start_engine_after_inspection<S>(
    socket: &Path,
    engine_cleanup: Vec<EngineCleanup>,
    inspection_notice: Option<String>,
    spawn_engine: S,
) -> Result<EngineGroup>
where
    S: FnOnce() -> Result<Child>,
{
    let cleanup_on_failure = describe_cleanup(&engine_cleanup);
    let result =
        launch_verified_engine(socket, engine_cleanup, inspection_notice, spawn_engine).await;
    match (result, cleanup_on_failure) {
        (Err(error), Some(cleanup)) => {
            let cause = error.to_string();
            Err(error.context(format!(
                "El motor nuevo falló después del barrido CUA ({cause}); {cleanup}"
            )))
        }
        (result, _) => result,
    }
}

/// The sweep done before this start, one engine at a time, or `None` when
/// nothing was swept.
fn describe_cleanup(engine_cleanup: &[EngineCleanup]) -> Option<String> {
    if engine_cleanup.is_empty() {
        return None;
    }
    Some(
        engine_cleanup
            .iter()
            .map(|cleanup| {
                let outcome = match cleanup.outcome {
                    EngineCleanupOutcome::Stopped => "se confirmó detenido".to_owned(),
                    EngineCleanupOutcome::Survived => "sobrevivió a SIGKILL".to_owned(),
                    EngineCleanupOutcome::SignalFailed => format!(
                        "falló SIGKILL: {}",
                        cleanup.failure.as_deref().unwrap_or("causa no comunicada")
                    ),
                    EngineCleanupOutcome::Unconfirmed => format!(
                        "no se pudo confirmar la parada: {}",
                        cleanup.failure.as_deref().unwrap_or("causa no comunicada")
                    ),
                };
                format!(
                    "PID {} en {}: huérfano de su daemon; {outcome}",
                    cleanup.pid, cleanup.executable
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Retires the abandoned pathname, starts the engine in its own group and
/// waits until its socket answers from it.
async fn launch_verified_engine<S>(
    socket: &Path,
    engine_cleanup: Vec<EngineCleanup>,
    inspection_notice: Option<String>,
    spawn_engine: S,
) -> Result<EngineGroup>
where
    S: FnOnce() -> Result<Child>,
{
    // Only the pathname is removed. A listener with an open descriptor keeps
    // running, but cannot receive the new connection through this pathname.
    if std::fs::symlink_metadata(socket)
        .is_ok_and(|value| value.file_type().is_socket() || value.file_type().is_symlink())
    {
        std::fs::remove_file(socket).context("No se pudo retirar el socket CUA abandonado")?;
    }
    let child = spawn_engine()?;
    let child_pid = child
        .id()
        .context("El motor CUA terminó antes de identificar su proceso")?;
    let pgid = i32::try_from(child_pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .context("El motor CUA informó un PID inválido para su grupo")?;
    let mut engine = EngineGroup {
        child,
        pgid,
        cleanup: engine_cleanup,
        inspection_notice,
    };
    await_own_listener(&mut engine, socket, child_pid).await?;
    Ok(engine)
}

/// Polls the socket until a process descending from the engine Orbit started
/// answers it; the engine's death, a foreign listener or the startup budget
/// end the wait.
async fn await_own_listener(engine: &mut EngineGroup, socket: &Path, child_pid: u32) -> Result<()> {
    let deadline = Instant::now() + ENGINE_STARTUP_TIMEOUT;
    let mut last_probe_error = None;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = engine.child.try_wait() {
            bail!("El motor CUA terminó al arrancar ({status}); no se reintenta en este ciclo");
        }
        if let Ok(stream) = tokio::net::UnixStream::connect(socket).await {
            let fact = probe_peer(&stream, child_pid, deadline).await;
            match decide_peer_probe(fact, child_pid, socket) {
                PeerProbeDecision::Accept => return Ok(()),
                PeerProbeDecision::Retry(cause) => last_probe_error = Some(cause),
                PeerProbeDecision::Reject(cause) => bail!("{cause}"),
            }
            tokio::time::sleep(ENGINE_POLL_INTERVAL).await;
            continue;
        }
        tokio::time::sleep(ENGINE_POLL_INTERVAL).await;
    }
    if let Some(error) = last_probe_error {
        bail!(
            "No se pudo verificar el nuevo socket CUA tras {} ms: {error}; el control no está disponible",
            ENGINE_STARTUP_TIMEOUT.as_millis()
        );
    }
    bail!("El motor CUA no abrió su socket a tiempo; el control no está disponible")
}

/// What one connected probe says of the process at the other end.
async fn probe_peer(
    stream: &tokio::net::UnixStream,
    child_pid: u32,
    deadline: Instant,
) -> PeerProbeFact {
    // On macOS Tokio reads LOCAL_PEEREPID and then calls getpeereid.
    // A listener can close this probe between those calls. Keep the
    // answer tied to this stream and retry a closed probe while the
    // engine still runs; never accept a peer without a verified PID.
    match stream.peer_cred() {
        Err(error) => PeerProbeFact::CredentialError(error),
        Ok(credential) => match credential.pid() {
            None => PeerProbeFact::PidMissing,
            Some(peer) => match u32::try_from(peer) {
                Err(_) => PeerProbeFact::InvalidPid(peer),
                Ok(peer_pid) => PeerProbeFact::Ancestry {
                    peer,
                    result: engine_parent_hops(peer_pid, child_pid, deadline).await,
                },
            },
        },
    }
}

async fn engine_parent_hops(
    peer_pid: u32,
    child_pid: u32,
    deadline: Instant,
) -> Result<Option<usize>> {
    let mut current = peer_pid;
    for hops in 0..=MAX_ENGINE_PARENT_HOPS {
        if current == child_pid {
            return Ok(Some(hops));
        }
        if hops == MAX_ENGINE_PARENT_HOPS {
            break;
        }
        let Some(parent) = parent_pid(current, deadline)
            .await
            .map_err(|error| parent_lookup_context(peer_pid, child_pid, hops, error))?
        else {
            return Ok(None);
        };
        if parent == current {
            return Ok(None);
        }
        current = parent;
    }
    Ok(None)
}

pub(crate) fn parent_lookup_context(
    peer_pid: u32,
    child_pid: u32,
    hops: usize,
    error: anyhow::Error,
) -> anyhow::Error {
    anyhow::anyhow!(
        "No se pudo comprobar el parentesco del PID par {peer_pid} con el motor PID {child_pid} en el paso {hops}/{MAX_ENGINE_PARENT_HOPS}: {error:#}"
    )
}

#[cfg(target_os = "linux")]
async fn parent_pid(pid: u32, _deadline: Instant) -> Result<Option<u32>> {
    let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
        .await
        .with_context(|| format!("No se pudo leer /proc/{pid}/stat"))?;
    // The command name is parenthesized and may contain spaces; the fields
    // after its final closing parenthesis begin with state, then parent PID.
    let (_, fields) = stat
        .rsplit_once(") ")
        .with_context(|| format!("/proc/{pid}/stat no contiene el cierre del nombre"))?;
    let parent = fields
        .split_whitespace()
        .nth(1)
        .with_context(|| format!("/proc/{pid}/stat no contiene PPID"))?
        .parse::<u32>()
        .with_context(|| format!("/proc/{pid}/stat contiene un PPID inválido"))?;
    Ok((parent != 0).then_some(parent))
}

#[cfg(target_os = "macos")]
async fn parent_pid(pid: u32, deadline: Instant) -> Result<Option<u32>> {
    // Use the remaining 10 s engine startup budget, not a new timeout: a
    // stalled ps must not leave startup waiting forever for one parent hop.
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = tokio::time::timeout(
        remaining,
        Command::new("/bin/ps")
            .args(["-o", "ppid=", "-p"])
            .arg(pid.to_string())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| ps_timeout_message(pid, remaining))?
    .with_context(|| format!("No se pudo consultar el padre del PID {pid} con ps"))?;
    if !output.status.success() {
        bail!(
            "ps no pudo consultar el padre del PID {pid} ({})",
            output.status
        );
    }
    let raw = std::str::from_utf8(&output.stdout)
        .with_context(|| format!("ps devolvió un PPID no UTF-8 para el PID {pid}"))?;
    let parent = raw
        .trim()
        .parse::<u32>()
        .with_context(|| format!("ps devolvió un PPID inválido para el PID {pid}: {raw:?}"))?;
    Ok((parent != 0).then_some(parent))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn ps_timeout_message(pid: u32, remaining: Duration) -> String {
    format!(
        "ps no respondió en los {} ms que quedaban de los {} ms del arranque al consultar el padre del PID {pid}",
        remaining.as_millis(),
        ENGINE_STARTUP_TIMEOUT.as_millis()
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn parent_pid(pid: u32, _deadline: Instant) -> Result<Option<u32>> {
    bail!("Esta plataforma no permite consultar el padre del PID {pid} para validar CUA")
}
