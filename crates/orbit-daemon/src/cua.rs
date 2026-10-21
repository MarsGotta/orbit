//! Bounded verified CUA MCP client. Orbit starts the engine itself, owns its
//! socket directory, and forwards whatever tools the engine declares.
use anyhow::{Context, Result, bail};
use orbit_core::orbit_bundle::{self, Member};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    os::unix::process::ExitStatusExt,
    path::{Component, Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::Instant,
};

pub const VERIFIED_DRIVER_VERSION: &str = "0.28.2";
const VERIFIED_MCP_VERSION: &str = "2025-06-18";
/// The largest message Orbit holds whole from the engine, or sends to it: a
/// frame is kept entire until it is parsed, so this protects the daemon's
/// memory from a single message. 1 MiB because the largest message Orbit can
/// use is a capture at the capture ceiling (`MAX_IMAGE_BASE64_BYTES`, the
/// 700 000 bytes of PNG the sources and the hub admit, 933 336 in base64) with
/// its envelope; the engine's whole catalogue measured 147 909 bytes on Mars's
/// Mac. It is not an image bound, and a longer answer costs its request, not
/// the connection: it is read to its end, dropped, and refused with its size.
/// A Retina full screen is several MiB of base64, which is why it never comes
/// inline: the engine writes it to a file of Orbit's (SIM-56).
const MAX_FRAME_BYTES: usize = 1_048_576;
/// The base64 of the largest capture Orbit can deliver: the sources
/// (`orbit_core::sources::MAX_SOURCE_BYTES`) and the hub's capture upload
/// admit 700 000 bytes of PNG. A larger image could not reach anyone, so it is
/// refused where it arrives, with its size. Not a message bound.
const MAX_IMAGE_BASE64_BYTES: usize = orbit_core::sources::MAX_SOURCE_BYTES.div_ceil(3) * 4;
// A capture at its own ceiling has to fit in one message: what is left of
// the message (115 240 bytes) is its envelope.
const _: () = assert!(MAX_IMAGE_BASE64_BYTES < MAX_FRAME_BYTES);
const MAX_NOTIFICATION_COUNT: usize = 64;
/// The engine subcommand that binds the socket Orbit hands it. Named once so a
/// driver that renames it is corrected in a single place.
const DRIVER_DAEMON_SUBCOMMAND: &str = "serve";
/// The directory name Orbit creates, privately, under its own state root.
const SOCKET_DIRECTORY: &str = "cua";
const SOCKET_FILE: &str = "driver.sock";
/// Bound process ancestry queries during CUA startup. The packaged 0.28.2
/// driver on macOS was measured with one extra `serve` process (43904 ->
/// 43906); four links allow three more wrappers while limiting local process
/// probes to four. This is not a timeout or proof of a binary's identity.
const MAX_ENGINE_PARENT_HOPS: usize = 4;
const ENGINE_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const ENGINE_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long Orbit waits, once the transport says the engine's end is gone —
/// end of file on what it writes, or a broken pipe on what Orbit writes to it
/// — to learn whether the process it started has died. An exit closes the
/// process's pipes before its status can be collected, so at that instant the
/// status is often not there yet: in half the broken pipes and 86 % of the
/// ends of file REF-18 measured. Over 44,000 deaths at birth on Linux, with
/// load averages of 44 to 91 and up to sixteen busy loops per CPU, the status
/// arrived at p99 within 43 ms of the report and at worst within 77 ms; 500 ms
/// is over six times that worst case. Linux only: tokio learns of the exit
/// through a pidfd there and through SIGCHLD on macOS, which is unmeasured.
/// It is not a deadline on the engine's work, which the request timeout
/// bounds: a process still running when it expires keeps its own message, so
/// all it costs is that much delay in reporting a live engine that closed its
/// pipes, and a death slower than this to collect is reported as the transport
/// saw it, never as a death that did not happen.
const ENGINE_EXIT_GRACE: Duration = Duration::from_millis(500);
/// The tools Orbit's own observation helpers call. This is not a filter on what
/// Deimos may reach: `describe` reports the engine's whole catalogue.
const ORBIT_READ_TOOLS: &[&str] = &["list_apps", "list_windows"];

#[derive(Debug, Clone)]
pub struct CuaConfig {
    pub driver_path: PathBuf,
    pub socket_path: PathBuf,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolDescription {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Platform-native app/window schemas vary; preserve their structured evidence.
#[derive(Debug, Clone, Serialize)]
pub struct NativeObservation {
    pub structured: Option<Value>,
    pub text: Vec<String>,
    pub images: Vec<NativeImage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeImage {
    pub data: String,
    pub mime_type: String,
}

/// A collected exit, before Orbit invalidates or kills the connection. Only
/// process identity and status cross into the visible notice; no error text.
#[derive(Debug)]
pub(crate) struct EngineExit {
    pub process: &'static str,
    pub status: ExitStatus,
}

impl std::fmt::Display for EngineExit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let process = if self.process == "engine" {
            "motor CUA"
        } else {
            "proxy MCP de CUA"
        };
        if let Some(code) = self.status.code() {
            write!(
                formatter,
                "El {process} terminó sin que Orbit lo pidiera (código {code})"
            )
        } else if let Some(signal) = self.status.signal() {
            write!(
                formatter,
                "El {process} terminó sin que Orbit lo pidiera (señal {signal})"
            )
        } else {
            write!(
                formatter,
                "El {process} terminó con estado de espera {}",
                self.status.into_raw()
            )
        }
    }
}

impl std::error::Error for EngineExit {}

pub type LocalCua = CuaClient<BufReader<ChildStdout>, ChildStdin>;
pub type OwnedCua =
    CuaClient<Box<dyn AsyncBufRead + Send + Unpin>, Box<dyn AsyncWrite + Send + Unpin>>;

/// A single mutable owner serializes requests. A transport failure poisons it;
/// there is no automatic reconnect, replay, browser preparation or grant.
pub struct CuaClient<R, W> {
    reader: R,
    writer: W,
    child: Option<Child>,
    /// The private process group for this connection's engine and descendants.
    engine: Option<EngineGroup>,
    startup_cleanup: Vec<orbit_core::topics::control::EngineCleanup>,
    startup_inspection_notice: Option<String>,
    timeout: Duration,
    alive: bool,
    initialized: bool,
    next_id: u64,
    connection_id: String,
    tools: Vec<ToolDescription>,
}

impl LocalCua {
    /// Starts the engine, makes its socket directory private, and connects.
    /// Nobody has to run anything by hand: the daemon calls this when macOS
    /// grants Accessibility and Screen Recording (REF-11), and that is the
    /// whole ceremony.
    pub async fn connect(config: CuaConfig) -> Result<Self> {
        validate_path(&config.driver_path)?;
        validate_path(&config.socket_path)?;
        validate_timeout(config.request_timeout)?;
        let binary = verified_driver_binary(&config.driver_path)?;
        // Before the engine runs, so it never gets to create the directory
        // world-readable and leave Orbit refusing its own socket.
        prepare_socket_directory(&config.socket_path)?;
        let mut engine = start_engine(&binary, &config.socket_path).await?;
        validate_private_socket(&config.socket_path)?;
        let mut command = driver_command(&binary);
        command
            .arg("mcp")
            .arg("--embedded")
            .arg("--socket")
            .arg(&config.socket_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .context("No se pudo iniciar el proxy MCP de CUA")?;
        let reader = BufReader::new(child.stdout.take().context("CUA no tiene stdout")?);
        let writer = child.stdin.take().context("CUA no tiene stdin")?;
        let mut client = Self::from_transport(reader, writer, config.request_timeout)?;
        client.child = Some(child);
        client.startup_cleanup = std::mem::take(&mut engine.cleanup);
        client.startup_inspection_notice = engine.inspection_notice.take();
        client.engine = Some(engine);
        client.initialize().await?;
        Ok(client)
    }
}

/// The driver executable, checked for the same trust the bundle guarantees.
fn verified_driver_binary(configured: &Path) -> Result<PathBuf> {
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
fn driver_command(binary: &Path) -> Command {
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
struct EngineGroup {
    child: Child,
    pgid: rustix::process::Pid,
    cleanup: Vec<orbit_core::topics::control::EngineCleanup>,
    inspection_notice: Option<String>,
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
                "No se pudo detener el grupo del motor CUA (PGID {}): {error}",
                self.pgid.as_raw_pid()
            ),
        }
    }
}

/// The socket path Orbit uses when nobody named one. It lives inside Orbit's
/// own state root, which Orbit can keep private.
pub fn default_socket_path(root: &Path) -> PathBuf {
    root.join(SOCKET_DIRECTORY).join(SOCKET_FILE)
}

/// Where the engine writes one full-screen capture (SIM-56): Orbit's private
/// engine directory, the one that holds its socket, under a name of its own
/// per request, so a capture never travels inside the engine's frame and two
/// requests never share a file. The directory is made private first, since
/// the engine writes there and Orbit reads it back.
pub fn desktop_capture_path(root: &Path, request: &str) -> Result<PathBuf> {
    let path = root
        .join(SOCKET_DIRECTORY)
        .join(format!("desktop-{request}.png"));
    prepare_socket_directory(&path)?;
    Ok(path)
}

/// Orbit owns the directory, so the engine cannot leave it group- or
/// world-readable and make its own socket unusable.
fn prepare_socket_directory(socket: &Path) -> Result<()> {
    let parent = socket
        .parent()
        .context("La ruta del socket CUA no tiene directorio")?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "No se pudo crear el directorio privado del socket CUA en {}",
            parent.display()
        )
    })?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).with_context(
        || {
            format!(
                "No se pudieron cerrar los permisos del directorio del socket CUA en {}",
                parent.display()
            )
        },
    )?;
    let metadata = std::fs::metadata(parent)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        bail!("El directorio del socket CUA debe pertenecer al usuario actual");
    }
    Ok(())
}

/// A socket left by another daemon is not by itself proof that its listener is
/// our engine. On macOS a bounded census must prove the exact bundle, user and
/// orphaned ancestry before signaling; otherwise only retire the pathname.
async fn start_engine(binary: &Path, socket: &Path) -> Result<EngineGroup> {
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

async fn start_engine_after_inspection<S>(
    socket: &Path,
    engine_cleanup: Vec<orbit_core::topics::control::EngineCleanup>,
    inspection_notice: Option<String>,
    spawn_engine: S,
) -> Result<EngineGroup>
where
    S: FnOnce() -> Result<Child>,
{
    let cleanup_on_failure = if engine_cleanup.is_empty() {
        None
    } else {
        Some(
            engine_cleanup
                .iter()
                .map(|cleanup| {
                    let outcome = match cleanup.outcome {
                        orbit_core::topics::control::EngineCleanupOutcome::Stopped => {
                            "se confirmó detenido".to_owned()
                        }
                        orbit_core::topics::control::EngineCleanupOutcome::Survived => {
                            "sobrevivió a SIGKILL".to_owned()
                        }
                        orbit_core::topics::control::EngineCleanupOutcome::SignalFailed => format!(
                            "falló SIGKILL: {}",
                            cleanup.failure.as_deref().unwrap_or("causa no comunicada")
                        ),
                        orbit_core::topics::control::EngineCleanupOutcome::Unconfirmed => format!(
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
    };
    let result: Result<EngineGroup> = async {
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
    let deadline = Instant::now() + ENGINE_STARTUP_TIMEOUT;
    let mut last_probe_error = None;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = engine.child.try_wait() {
            bail!("El motor CUA terminó al arrancar ({status}); no se reintenta en este ciclo");
        }
        if let Ok(stream) = tokio::net::UnixStream::connect(socket).await {
            // On macOS Tokio reads LOCAL_PEEREPID and then calls getpeereid.
            // A listener can close this probe between those calls. Keep the
            // answer tied to this stream and retry a closed probe while the
            // engine still runs; never accept a peer without a verified PID.
            let fact = match stream.peer_cred() {
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
            };
            match decide_peer_probe(fact, child_pid, socket) {
                PeerProbeDecision::Accept => return Ok(engine),
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
    .await;
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

#[cfg(any(target_os = "macos", test))]
async fn inspect_previous_socket<L, F, K, S, SF>(
    binary: &Path,
    socket: &Path,
    get_listing: L,
    kill_process: K,
    sleep: S,
) -> ListenerInspection
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    K: Fn(rustix::process::Pid, rustix::process::Signal) -> Result<(), rustix::io::Errno>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let stream = match tokio::net::UnixStream::connect(socket).await {
        Ok(stream) => stream,
        Err(_) => {
            return ListenerInspection {
                cleanups: Vec::new(),
                notice: None,
            };
        }
    };
    let credential = match stream.peer_cred() {
        Ok(credential) => credential,
        Err(error) => {
            return ListenerInspection::skipped(format!(
                "No se pudo identificar al proceso del socket CUA previo en {}: {error}; no se envió ninguna señal",
                socket.display()
            ));
        }
    };
    let Some(peer) = credential.pid() else {
        return ListenerInspection::skipped(format!(
            "El socket CUA previo en {} no comunicó el PID de su proceso; no se envió ninguna señal",
            socket.display()
        ));
    };
    inspect_previous_listener(
        binary,
        socket,
        ListenerIdentity {
            peer,
            peer_uid: credential.uid(),
            own_uid: rustix::process::geteuid().as_raw(),
        },
        get_listing,
        kill_process,
        sleep,
    )
    .await
}

#[cfg(any(target_os = "macos", test))]
struct ListenerIdentity {
    peer: i32,
    peer_uid: u32,
    own_uid: u32,
}

#[cfg(any(target_os = "macos", test))]
async fn inspect_previous_listener<L, F, K, S, SF>(
    binary: &Path,
    socket: &Path,
    identity: ListenerIdentity,
    get_listing: L,
    kill_process: K,
    sleep: S,
) -> ListenerInspection
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    K: Fn(rustix::process::Pid, rustix::process::Signal) -> Result<(), rustix::io::Errno>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let ListenerIdentity {
        peer,
        peer_uid,
        own_uid,
    } = identity;
    let Ok(peer_pid) = u32::try_from(peer) else {
        eprintln!(
            "El socket CUA previo ({}) informó el PID inválido {peer}; se retira la ruta sin señalar ningún proceso",
            socket.display()
        );
        return ListenerInspection::skipped(format!(
            "El socket CUA previo informó el PID inválido {peer}; no se envió ninguna señal"
        ));
    };
    if peer_uid != own_uid {
        eprintln!(
            "El socket CUA previo ({}) escucha al PID {peer_pid} del usuario {peer_uid}, no al usuario {own_uid} de Orbit; se retira la ruta sin señalarlo",
            socket.display()
        );
        return ListenerInspection::skipped(format!(
            "El socket CUA previo escucha al PID {peer_pid} del usuario {peer_uid}, no al usuario {own_uid}; no se envió ninguna señal"
        ));
    }
    let Some(bundle) = orbit_bundle::bundle_of(binary, Member::Engine) else {
        eprintln!(
            "El socket CUA previo ({}) escucha al PID {peer_pid}, pero {} no es el motor de un paquete Orbit; se retira la ruta sin señalarlo",
            socket.display(),
            binary.display()
        );
        return ListenerInspection::skipped(format!(
            "El socket CUA previo escucha al PID {peer_pid}, pero {} no es un motor de un paquete Orbit; no se envió ninguna señal",
            binary.display()
        ));
    };
    let listing = match tokio::time::timeout(
        orbit_core::process_listing::PS_LISTING_TIMEOUT,
        get_listing(),
    )
    .await
    {
        Ok(Ok(listing)) => listing,
        Ok(Err(error)) => {
            eprintln!(
                "El socket CUA previo ({}) escucha al PID {peer_pid}, pero no se pudo censar su identidad: {error:#}; se retira la ruta sin señalarlo",
                socket.display()
            );
            return ListenerInspection::skipped(format!(
                "No se pudo censar el PID {peer_pid} del socket CUA previo: {error:#}; no se envió ninguna señal"
            ));
        }
        Err(_) => {
            return ListenerInspection::skipped(format!(
                "El censo del PID {peer_pid} del socket CUA previo venció a los {} ms; no se envió ninguna señal",
                orbit_core::process_listing::PS_LISTING_TIMEOUT.as_millis()
            ));
        }
    };
    match orbit_core::process_listing::decide_engine_listener(&listing, peer_pid, own_uid, &bundle)
    {
        orbit_core::process_listing::EngineListenerDecision::Keep(reason) => {
            eprintln!(
                "El socket CUA previo ({}) escucha al PID {peer_pid}; el censo no autoriza barrerlo ({reason}), así que se retira la ruta sin señalarlo",
                socket.display()
            );
            ListenerInspection::skipped(format!(
                "No se barrió el PID {peer_pid} del socket CUA previo: {reason}; no se envió ninguna señal"
            ))
        }
        orbit_core::process_listing::EngineListenerDecision::Sweep(targets) => {
            let mut cleanups = Vec::new();
            let mut killed_targets = Vec::new();
            for target in targets {
                let Some(pid) = i32::try_from(target.pid)
                    .ok()
                    .and_then(rustix::process::Pid::from_raw)
                else {
                    eprintln!(
                        "No se pudo representar el PID {} del motor CUA huérfano en {}; se retira la ruta sin señalarlo",
                        target.pid,
                        target.executable.display()
                    );
                    cleanups.push(orbit_core::topics::control::EngineCleanup {
                        pid: target.pid,
                        executable: target.executable.display().to_string(),
                        reason:
                            orbit_core::topics::control::EngineCleanupReason::OrphanedFromDaemon,
                        outcome: orbit_core::topics::control::EngineCleanupOutcome::SignalFailed,
                        failure: Some(
                            "el PID no cabe en el identificador de proceso del sistema".into(),
                        ),
                    });
                    continue;
                };
                eprintln!(
                    "Aviso: se barre el motor CUA PID {} en {} porque sobrevivió a su daemon y ya no desciende de ningún daemon vivo del paquete",
                    target.pid,
                    target.executable.display()
                );
                match kill_process(pid, rustix::process::Signal::KILL) {
                    Err(rustix::io::Errno::SRCH) => {
                        cleanups.push(orbit_core::topics::control::EngineCleanup {
                            pid: target.pid,
                            executable: target.executable.display().to_string(),
                            reason: orbit_core::topics::control::EngineCleanupReason::OrphanedFromDaemon,
                            outcome: orbit_core::topics::control::EngineCleanupOutcome::Stopped,
                            failure: None,
                        });
                    }
                    Err(error) => {
                        eprintln!(
                            "Aviso: el motor CUA PID {} en {} sigue en marcha porque SIGKILL falló: {error}",
                            target.pid,
                            target.executable.display()
                        );
                        cleanups.push(orbit_core::topics::control::EngineCleanup {
                            pid: target.pid,
                            executable: target.executable.display().to_string(),
                            reason: orbit_core::topics::control::EngineCleanupReason::OrphanedFromDaemon,
                            outcome: orbit_core::topics::control::EngineCleanupOutcome::SignalFailed,
                            failure: Some(error.to_string()),
                        });
                    }
                    Ok(()) => {
                        killed_targets.push(target);
                    }
                }
            }

            if !killed_targets.is_empty() {
                let start = std::time::Instant::now();
                let mut last_observation = None;
                let mut observation_error = None;
                while start.elapsed() < orbit_core::process_listing::KILL_OBSERVATION_WAIT {
                    sleep(std::time::Duration::from_millis(10)).await;
                    match get_listing().await {
                        Ok(listing) => {
                            let all_stopped = killed_targets.iter().all(|t| {
                                observed_cleanup_outcome(&listing, t, own_uid)
                                    == ObservedCleanupOutcome::Stopped
                            });
                            last_observation = Some(listing);
                            if all_stopped {
                                break;
                            }
                        }
                        Err(error) => {
                            observation_error = Some(error.to_string());
                            last_observation = None;
                            break;
                        }
                    }
                }

                for target in killed_targets {
                    let outcome = last_observation
                        .as_ref()
                        .map(|listing| observed_cleanup_outcome(listing, &target, own_uid));
                    let (out, fail) = match outcome {
                        Some(ObservedCleanupOutcome::Survived) => {
                            eprintln!(
                                "Aviso: el motor CUA PID {} en {} sobrevivió a SIGKILL; no se afirma que se haya detenido",
                                target.pid,
                                target.executable.display()
                            );
                            (
                                orbit_core::topics::control::EngineCleanupOutcome::Survived,
                                None,
                            )
                        }
                        Some(ObservedCleanupOutcome::Stopped) => {
                            eprintln!(
                                "Se confirmó el barrido del motor CUA PID {} en {}: motivo, huérfano de su daemon",
                                target.pid,
                                target.executable.display()
                            );
                            (
                                orbit_core::topics::control::EngineCleanupOutcome::Stopped,
                                None,
                            )
                        }
                        None => (
                            orbit_core::topics::control::EngineCleanupOutcome::Unconfirmed,
                            Some(observation_error.clone().unwrap_or_else(|| {
                                "no llegó ningún censo posterior a SIGKILL".into()
                            })),
                        ),
                    };
                    cleanups.push(orbit_core::topics::control::EngineCleanup {
                        pid: target.pid,
                        executable: target.executable.display().to_string(),
                        reason:
                            orbit_core::topics::control::EngineCleanupReason::OrphanedFromDaemon,
                        outcome: out,
                        failure: fail,
                    });
                }
            }
            ListenerInspection {
                cleanups,
                notice: None,
            }
        }
    }
}
#[cfg(any(target_os = "macos", test))]
struct ListenerInspection {
    cleanups: Vec<orbit_core::topics::control::EngineCleanup>,
    notice: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
impl ListenerInspection {
    fn skipped(notice: String) -> Self {
        Self {
            cleanups: Vec::new(),
            notice: Some(notice),
        }
    }
}
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservedCleanupOutcome {
    Stopped,
    Survived,
}

#[cfg(any(target_os = "macos", test))]
fn observed_cleanup_outcome(
    listing: &str,
    target: &orbit_core::process_listing::EngineSweepTarget,
    own_uid: u32,
) -> ObservedCleanupOutcome {
    if orbit_core::process_listing::tree_entries(listing)
        .into_iter()
        .any(|entry| {
            entry.pid == target.pid
                && entry.user_id == own_uid
                && Path::new(entry.executable) == target.executable
        })
    {
        ObservedCleanupOutcome::Survived
    } else {
        ObservedCleanupOutcome::Stopped
    }
}

#[cfg(target_os = "macos")]
async fn engine_process_listing() -> Result<String> {
    tokio::task::spawn_blocking(crate::app_control::engine_process_listing)
        .await
        .context("No se pudo recoger la tarea del censo de motores CUA")?
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

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> CuaClient<R, W> {
    /// Retains the exact initialized connection while transferring sole ownership
    /// to the daemon worker. No reconnect or transport initialization is performed.
    pub fn into_owned(self) -> OwnedCua
    where
        R: Send + 'static,
        W: Send + 'static,
    {
        CuaClient {
            reader: Box::new(self.reader),
            writer: Box::new(self.writer),
            child: self.child,
            engine: self.engine,
            startup_cleanup: self.startup_cleanup,
            startup_inspection_notice: self.startup_inspection_notice,
            timeout: self.timeout,
            alive: self.alive,
            initialized: self.initialized,
            next_id: self.next_id,
            connection_id: self.connection_id,
            tools: self.tools,
        }
    }
    /// An already-authorized transport supplied by the host (also used by fixtures).
    pub fn from_transport(reader: R, writer: W, timeout: Duration) -> Result<Self> {
        validate_timeout(timeout)?;
        let connection_id = uuid::Uuid::new_v4().to_string();
        Ok(Self {
            reader,
            writer,
            timeout,
            connection_id,
            child: None,
            engine: None,
            startup_cleanup: Vec::new(),
            startup_inspection_notice: None,
            alive: true,
            initialized: false,
            next_id: 0,
            tools: Vec::new(),
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            bail!("La sesión CUA ya está inicializada");
        }
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion":VERIFIED_MCP_VERSION, "capabilities":{},
                    "clientInfo":{"name":"orbit", "version":env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        let Some(actual_driver_version) = result["serverInfo"]["version"].as_str() else {
            self.poison();
            bail!("Versión de cua-driver ausente; se esperaba {VERIFIED_DRIVER_VERSION}");
        };
        if actual_driver_version != VERIFIED_DRIVER_VERSION {
            self.poison();
            bail!(
                "Versión de cua-driver {actual_driver_version}; se esperaba {VERIFIED_DRIVER_VERSION}"
            );
        }
        if result["protocolVersion"] != VERIFIED_MCP_VERSION {
            self.poison();
            bail!("Versión MCP no verificada; se esperaba {VERIFIED_MCP_VERSION}");
        }
        self.notify("notifications/initialized").await?;
        let tools = self.request("tools/list", json!({})).await?;
        if tools
            .get("nextCursor")
            .is_some_and(|value| !value.is_null())
        {
            self.poison();
            bail!("Catálogo CUA paginado no compatible con este adaptador");
        }
        let descriptions: Vec<ToolDescription> = serde_json::from_value(tools["tools"].clone())?;
        if descriptions.len() > orbit_core::computer_control::MAX_ENGINE_TOOLS {
            self.poison();
            bail!("Catálogo CUA demasiado grande");
        }
        // The catalogue's own bound, applied where the catalogue actually
        // arrives. An engine that answers with something absurd is refused
        // here, by name, instead of being kept and failing later on somebody
        // else's limit.
        let catalogue_bytes = serde_json::to_vec(&descriptions)?.len();
        if catalogue_bytes > orbit_core::computer_control::MAX_ENGINE_CATALOGUE_BYTES {
            self.poison();
            bail!(
                "El catálogo del motor ocupa {catalogue_bytes} bytes y supera su límite de {} bytes; un catálogo no se entrega recortado",
                orbit_core::computer_control::MAX_ENGINE_CATALOGUE_BYTES
            );
        }
        // Kept whole, on purpose: the catalogue Deimos is offered is the one the
        // engine declares, so a newer engine grows a tool without touching Orbit.
        self.tools = descriptions;
        if !ORBIT_READ_TOOLS
            .iter()
            .all(|name| self.tools.iter().any(|tool| tool.name == *name))
        {
            self.poison();
            bail!("El driver no ofrece las herramientas de lectura verificadas");
        }
        self.initialized = true;
        Ok(())
    }

    /// Everything the engine declared, exactly as it declared it.
    pub fn describe(&self) -> &[ToolDescription] {
        &self.tools
    }

    /// The catalogue in the shape the projection and the remote answer carry.
    pub fn engine_tools(&self) -> Vec<orbit_core::computer_control::EngineTool> {
        self.tools
            .iter()
            .take(orbit_core::computer_control::MAX_ENGINE_TOOLS)
            .map(|tool| orbit_core::computer_control::EngineTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .filter(orbit_core::computer_control::EngineTool::is_valid)
            .collect()
    }

    /// The identity of this connection: a reconnect is a different one.
    pub fn connection_epoch(&self) -> &str {
        &self.connection_id
    }

    /// Closes this connection for good: it is not reused or reconnected.
    pub fn invalidate(&mut self) {
        self.poison();
    }

    pub fn is_connected(&self) -> bool {
        self.alive && self.initialized
    }

    /// Nonblocking: maintenance checks idle children without sending a tool.
    /// Workers own the same children while a request is in flight; transport
    /// failures collect their status before poisoning can kill them.
    pub(crate) fn check_processes(&mut self) -> Result<()> {
        if let Some(engine) = self.engine.as_mut()
            && let Some(status) = engine
                .child
                .try_wait()
                .context("No se pudo consultar la salida del motor CUA")?
        {
            return Err(EngineExit {
                process: "engine",
                status,
            }
            .into());
        }
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child
                .try_wait()
                .context("No se pudo consultar la salida del proxy MCP de CUA")?
        {
            return Err(EngineExit {
                process: "proxy",
                status,
            }
            .into());
        }
        Ok(())
    }

    pub fn take_engine_cleanup(&mut self) -> Vec<orbit_core::topics::control::EngineCleanup> {
        std::mem::take(&mut self.startup_cleanup)
    }

    pub fn take_engine_inspection_notice(&mut self) -> Option<String> {
        self.startup_inspection_notice.take()
    }

    #[cfg(test)]
    pub(crate) fn set_startup_cleanup_for_test(
        &mut self,
        cleanup: Vec<orbit_core::topics::control::EngineCleanup>,
    ) {
        self.startup_cleanup = cleanup;
    }

    pub async fn windows(&mut self, pid: Option<i64>) -> Result<NativeObservation> {
        if pid.is_some_and(|pid| pid <= 0) {
            bail!("PID inválido");
        }
        let mut arguments = json!({"on_screen_only":false});
        if let Some(pid) = pid {
            arguments["pid"] = json!(pid);
        }
        self.read_tool("list_windows", arguments).await
    }

    pub fn declares_tool(&self, name: &str) -> bool {
        self.initialized && self.tools.iter().any(|tool| tool.name == name)
    }

    /// Runs one tool the engine declared, with the arguments as given. Orbit
    /// checks that the engine has the tool and that the frame is bounded; it
    /// does not second-guess a vocabulary that is the engine's to define.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<NativeObservation> {
        if !self.declares_tool(name) {
            bail!("El motor no declara «{name}» en esta instalación");
        }
        if !arguments.is_object() {
            bail!("Los argumentos de la herramienta deben ser un objeto");
        }
        let result = self
            .request("tools/call", json!({"name":name,"arguments":arguments}))
            .await?;
        parse_observation(&result)
    }

    async fn read_tool(&mut self, name: &str, arguments: Value) -> Result<NativeObservation> {
        if !self.initialized || !ORBIT_READ_TOOLS.contains(&name) {
            bail!("Herramienta CUA fuera del permiso de lectura");
        }
        let result = self
            .request("tools/call", json!({"name":name,"arguments":arguments}))
            .await?;
        parse_observation(&result)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        if !self.alive {
            bail!("La conexión CUA está cerrada; no se reenvían operaciones");
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("Secuencia MCP agotada")?;
        let id = self.next_id;
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let timeout = self.timeout;
        // Cancellation drops this future before the match below. Leave the
        // transport poisoned in that case, so buffered responses cannot be reused.
        self.alive = false;
        let operation = async {
            write_json(&mut self.writer, &message).await?;
            for _ in 0..MAX_NOTIFICATION_COUNT {
                let response = read_json(&mut self.reader).await?;
                if response["jsonrpc"] != "2.0" {
                    bail!("Respuesta MCP inválida");
                }
                if response.get("method").is_some() {
                    if response.get("id").is_some() {
                        bail!("CUA solicitó una operación de cliente no autorizada");
                    }
                    match response["method"].as_str() {
                        Some("notifications/progress" | "notifications/message") => continue,
                        _ => {
                            bail!("La sesión MCP cambió; es necesario volver a validar su catálogo")
                        }
                    }
                }
                if response["id"].as_u64() != Some(id) {
                    bail!("Identidad de respuesta MCP incorrecta");
                }
                if response.get("error").is_some() {
                    bail!("CUA rechazó la petición MCP");
                }
                return response
                    .get("result")
                    .cloned()
                    .context("Respuesta MCP sin resultado");
            }
            bail!("Exceso de notificaciones MCP");
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(value)) => {
                self.alive = true;
                Ok(value)
            }
            // The answer was read to its end: the connection is in step, and
            // the engine, which answered, is not killed for its size (SIM-56).
            // It is taken as this request's answer. Had it been a
            // notification instead, the real answer would reach the next
            // request with the wrong identity, and that request would be
            // refused by name and close the connection: one request lost,
            // none repeated.
            Ok(Err(error)) if error.downcast_ref::<ResponseTooLarge>().is_some() => {
                self.alive = true;
                Err(error)
            }
            Ok(Err(error)) => {
                // Before poisoning, because poisoning kills the process and a
                // killed process cannot be told apart from one that died on
                // its own.
                let error = self
                    .name_the_death_behind_the_silence(error, Sent::Request)
                    .await;
                self.poison();
                Err(error)
            }
            Err(_) => {
                let status = self.check_processes();
                self.poison();
                status?;
                bail!("Tiempo de espera CUA agotado; no se reenvía la petición")
            }
        }
    }

    /// End of file says the engine stopped talking, never why. A driver binary
    /// the system starts and that dies in the act — what macOS makes of a
    /// broken executable, where Linux refuses to start it at all — leaves
    /// nothing behind but that silence. So when the process Orbit started is
    /// already gone, the death is the cause and gets named, with its status.
    /// When it is still running the connection really did close, and the
    /// message stays as it was: no death is attributed that did not happen.
    /// A write that finds the pipe broken is the same silence, found by the
    /// write before the read gets to it (REF-18). What was in flight decides
    /// what the death can truthfully be called: a death after the engine
    /// answered is named as that, not as a binary that never started.
    async fn name_the_death_behind_the_silence(
        &mut self,
        error: anyhow::Error,
        sent: Sent<'_>,
    ) -> anyhow::Error {
        if let Err(exit) = self.check_processes() {
            if let Some(death) = exit.downcast_ref::<EngineExit>() {
                let status = death.status;
                return exit.context(death_context(status, sent, self.next_id));
            }
            return exit.context(error);
        }
        if !the_engine_end_is_gone(&error) {
            return error;
        }
        let Some(child) = self.child.as_mut() else {
            return error;
        };
        let Ok(Ok(status)) = tokio::time::timeout(ENGINE_EXIT_GRACE, child.wait()).await else {
            return error;
        };
        anyhow::Error::new(EngineExit {
            process: "proxy",
            status,
        })
        .context(death_context(status, sent, self.next_id))
    }

    /// Sent only after `initialize` was answered. A failure keeps its cause in
    /// the message, since what reaches the person is the message alone: the
    /// engine's death with its code, a timeout with its ceiling, or whatever
    /// else the write reported.
    async fn notify(&mut self, method: &str) -> Result<()> {
        match tokio::time::timeout(
            self.timeout,
            write_json(&mut self.writer, &json!({"jsonrpc":"2.0","method":method})),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                // Before poisoning, as for a request: poisoning kills the
                // process, and a killed process looks like one that died.
                let error = self
                    .name_the_death_behind_the_silence(error, Sent::Notification(method))
                    .await;
                self.poison();
                bail!("No se pudo inicializar la sesión CUA: {error:#}")
            }
            Err(_) => {
                self.poison();
                bail!(
                    "No se pudo inicializar la sesión CUA: tiempo de espera agotado tras {} ms al enviar la notificación {method}; no se reenvía",
                    self.timeout.as_millis()
                )
            }
        }
    }

    fn poison(&mut self) {
        self.alive = false;
        self.initialized = false;
        self.tools.clear();
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        // The engine goes with the connection: a reconnect starts a fresh one
        // rather than reattaching to a process whose state Orbit cannot see.
        drop(self.engine.take());
    }
}

fn death_context(status: ExitStatus, sent: Sent<'_>, next_id: u64) -> String {
    match sent {
        Sent::Request if next_id > 1 => format!(
            "El motor CUA terminó ({status}) sin responder a esta petición, tras haber respondido a las anteriores; no se reenvía la petición"
        ),
        Sent::Request => format!(
            "El motor CUA terminó nada más arrancar ({status}) sin responder al protocolo MCP; su binario puede estar roto"
        ),
        Sent::Notification(method) => format!(
            "El motor CUA terminó ({status}) sin recibir la notificación {method}, tras haber respondido a las peticiones anteriores; no se reenvía la notificación"
        ),
    }
}

/// Resolve the engine in its nested app unless development
/// explicitly supplies an external absolute path. `daemon_executable` is the
/// executable the daemon runs in: the app, `Contents/MacOS/Orbit` (SIM-51).
pub fn resolve_driver_path(configured: &str, daemon_executable: &Path) -> Result<PathBuf> {
    if !configured.is_empty() {
        return Ok(PathBuf::from(configured));
    }
    let bundle = orbit_bundle::bundle_of(daemon_executable, Member::App).with_context(|| {
        format!(
            "{} no es la app de un bundle de Orbit",
            daemon_executable.display()
        )
    })?;
    orbit_bundle::member_path(&bundle, Member::Engine)
        .context("El bundle de Orbit no tiene ruta para el motor")
}

fn validate_timeout(timeout: Duration) -> Result<()> {
    if timeout.is_zero() || timeout > Duration::from_secs(30) {
        bail!("Timeout CUA fuera de límites");
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        bail!("CUA requiere rutas absolutas sin traversal");
    }
    Ok(())
}

fn validate_private_socket(path: &Path) -> Result<()> {
    validate_path(path)?;
    let metadata = std::fs::symlink_metadata(path)
        .context("El motor CUA no dejó su socket donde Orbit lo esperaba")?;
    let uid = rustix::process::geteuid().as_raw();
    let parent = std::fs::metadata(path.parent().context("Socket sin directorio")?)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || !parent.is_dir()
        || parent.uid() != uid
        || parent.mode() & 0o077 != 0
    {
        bail!("El socket CUA y su directorio deben ser privados del usuario actual");
    }
    Ok(())
}

fn parse_observation(result: &Value) -> Result<NativeObservation> {
    if result["isError"] == true {
        // The engine's own words are the only cause there is; without them a
        // refused read is indistinguishable from any other (SIM-25, run 4).
        // Bounded: the line goes to daemon.log, never to the report log.
        let said: String = result["content"]
            .as_array()
            .and_then(|items| items.iter().find_map(|item| item["text"].as_str()))
            .unwrap_or("sin texto")
            .chars()
            .take(400)
            .collect();
        bail!("CUA rechazó la herramienta de lectura: {said}");
    }
    let structured = result
        .get("structuredContent")
        .filter(|value| !value.is_null())
        .cloned();
    let mut text = Vec::new();
    let mut images = Vec::new();
    for content in result["content"]
        .as_array()
        .context("Contenido MCP ausente")?
    {
        if content["type"] == "image" {
            if !images.is_empty() {
                bail!("CUA devolvió más de una imagen para una sola observación");
            }
            let data = content["data"].as_str().context("Imagen MCP sin datos")?;
            let mime_type = content["mimeType"]
                .as_str()
                .context("Imagen MCP sin tipo")?;
            if mime_type != "image/png" {
                bail!("Imagen MCP de tipo {mime_type}; Orbit sólo admite image/png");
            }
            if data.len() > MAX_IMAGE_BASE64_BYTES {
                return Err(ResponseTooLarge {
                    part: TooLargePart::Image,
                    bytes: data.len(),
                    max_bytes: MAX_IMAGE_BASE64_BYTES,
                }
                .into());
            }
            images.push(NativeImage {
                data: data.to_owned(),
                mime_type: mime_type.to_owned(),
            });
            continue;
        }
        if content["type"] != "text" {
            bail!("Contenido CUA no solicitado");
        }
        text.push(
            content["text"]
                .as_str()
                .context("Texto MCP inválido")?
                .to_owned(),
        );
    }
    Ok(NativeObservation {
        structured,
        text,
        images,
    })
}

async fn write_json<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        bail!(
            "El mensaje para el motor ocupa {} bytes y un mensaje del motor admite {MAX_FRAME_BYTES} bytes; no se envió",
            bytes.len() + 1
        );
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// End of file on the MCP transport, carried as a type rather than a string so
/// the caller that can tell *why* the engine stopped talking recognizes this
/// one failure and leaves every other one exactly as it was.
#[derive(Debug)]
struct ConnectionClosed;

impl std::fmt::Display for ConnectionClosed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Conexión MCP cerrada antes de responder")
    }
}

impl std::error::Error for ConnectionClosed {}

/// What Orbit had just sent when the transport went silent.
#[derive(Clone, Copy)]
enum Sent<'a> {
    /// A request, which the engine owed an answer.
    Request,
    /// A notification, by its method, which is never answered.
    Notification(&'a str),
}

/// Whether a transport failure says the engine's end of the pipes is gone: end
/// of file on what it writes, or a broken pipe on what Orbit writes to it. The
/// exit of the process closes both, so either can be how Orbit first learns of
/// a death; which one depends on whether Orbit's write lands before the exit
/// or after it (REF-18 measured both on the same fixture). Every other failure
/// is left exactly as it was.
fn the_engine_end_is_gone(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ConnectionClosed>().is_some()
        || error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
}

/// An answer of the engine refused for its size, with both numbers, so that
/// whoever asked is told them (SIM-56). It is not an engine failure: the
/// engine answered, and the connection stays in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseTooLarge {
    pub part: TooLargePart,
    pub bytes: usize,
    pub max_bytes: usize,
}

/// Which bound the answer met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooLargePart {
    /// The whole message, against `MAX_FRAME_BYTES`.
    Message,
    /// The image inside it, in base64, against `MAX_IMAGE_BASE64_BYTES`.
    Image,
}

impl std::fmt::Display for ResponseTooLarge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.part {
            TooLargePart::Message => write!(
                formatter,
                "La respuesta del motor ocupa {} bytes y un mensaje del motor admite {} bytes; se leyó entera y se descartó, y la conexión sigue",
                self.bytes, self.max_bytes
            ),
            TooLargePart::Image => write!(
                formatter,
                "La imagen del motor ocupa {} bytes en base64 y una captura admite {} bytes en base64 ({} bytes de PNG)",
                self.bytes,
                self.max_bytes,
                orbit_core::sources::MAX_SOURCE_BYTES
            ),
        }
    }
}

impl std::error::Error for ResponseTooLarge {}

/// One line of the transport. A line longer than `MAX_FRAME_BYTES` is not
/// kept: it is read to its end and dropped, so the next line is the next
/// message, and it is refused with its length. A sender that never ends its
/// line is bounded by the request's own timeout.
async fn read_json<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    let mut frame = Vec::new();
    let mut dropped = 0_usize;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(ConnectionClosed.into());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if dropped == 0 && frame.len() + count <= MAX_FRAME_BYTES {
            frame.extend_from_slice(&available[..count]);
        } else {
            dropped = dropped.saturating_add(frame.len()).saturating_add(count);
            frame = Vec::new();
        }
        reader.consume(count);
        if newline.is_some() {
            if dropped > 0 {
                return Err(ResponseTooLarge {
                    part: TooLargePart::Message,
                    bytes: dropped,
                    max_bytes: MAX_FRAME_BYTES,
                }
                .into());
            }
            return Ok(serde_json::from_slice(&frame)?);
        }
    }
}

impl LocalCua {
    /// A connection over the stdin and stdout of a proxy process that is
    /// already running, keeping the process, so a transport that breaks is
    /// reported by how the process ended when it has ended. `connect` builds
    /// the same client right after starting the proxy.
    pub fn from_process(mut child: Child, timeout: Duration) -> Result<Self> {
        let reader = BufReader::new(child.stdout.take().context("CUA no tiene stdout")?);
        let writer = child.stdin.take().context("CUA no tiene stdin")?;
        let mut client = Self::from_transport(reader, writer, timeout)?;
        client.child = Some(child);
        Ok(client)
    }
}

#[cfg(test)]
mod ref46_process_cleanup_tests {
    #![allow(clippy::unwrap_used)]

    use super::{ObservedCleanupOutcome, observed_cleanup_outcome};
    use orbit_core::process_listing::{EngineSweepReason, EngineSweepTarget};
    use std::path::PathBuf;

    #[tokio::test]
    async fn sim98_the_serve_process_exit_is_observed_without_an_mcp_failure() -> anyhow::Result<()>
    {
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", "exit 31"])
            .env_clear()
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        let pgid = rustix::process::Pid::from_raw(i32::try_from(
            child.id().ok_or_else(|| anyhow::anyhow!("fixture PID"))?,
        )?)
        .ok_or_else(|| anyhow::anyhow!("fixture group"))?;
        tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await??;
        let mut client = super::CuaClient::from_transport(
            tokio::io::BufReader::new(tokio::io::empty()),
            tokio::io::sink(),
            std::time::Duration::from_secs(5),
        )?;
        client.engine = Some(super::EngineGroup {
            child,
            pgid,
            cleanup: Vec::new(),
            inspection_notice: None,
        });
        let error = client
            .check_processes()
            .err()
            .ok_or_else(|| anyhow::anyhow!("serve exit was not observed"))?;
        let exit = error
            .downcast_ref::<super::EngineExit>()
            .ok_or_else(|| anyhow::anyhow!("exit status lost: {error:#}"))?;
        assert_eq!(exit.process, "engine");
        assert_eq!(exit.status.code(), Some(31));
        Ok(())
    }

    fn target() -> EngineSweepTarget {
        EngineSweepTarget {
            pid: 43_906,
            executable: PathBuf::from(
                "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
            ),
            reason: EngineSweepReason::OrphanedFromDaemon,
        }
    }

    #[test]
    fn a2_an_exact_engine_still_listed_is_reported_as_surviving_sigkill() {
        let listing = "43906 1 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n";
        assert_eq!(
            observed_cleanup_outcome(listing, &target(), 501),
            ObservedCleanupOutcome::Survived
        );
    }

    #[test]
    fn a2_absence_or_pid_reuse_is_reported_as_the_old_engine_stopped() {
        assert_eq!(
            observed_cleanup_outcome("", &target(), 501),
            ObservedCleanupOutcome::Stopped
        );
        assert_eq!(
            observed_cleanup_outcome(
                "43906 1 501 /Applications/Editor.app/Contents/MacOS/editor\n",
                &target(),
                501,
            ),
            ObservedCleanupOutcome::Stopped
        );
    }

    use super::{
        ListenerIdentity, inspect_previous_listener, inspect_previous_socket,
        start_engine_after_inspection,
    };
    use std::sync::{Arc, Mutex};

    fn same_user(peer: i32) -> ListenerIdentity {
        ListenerIdentity {
            peer,
            peer_uid: 501,
            own_uid: 501,
        }
    }

    #[tokio::test]
    async fn failed_new_engine_start_names_the_sweep_already_done() {
        use orbit_core::topics::control::{
            EngineCleanup, EngineCleanupOutcome, EngineCleanupReason,
        };
        let directory = tempfile::tempdir().unwrap();
        let error = match start_engine_after_inspection(
            &directory.path().join("new.sock"),
            vec![EngineCleanup {
                pid: 43_906,
                executable: "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver".into(),
                reason: EngineCleanupReason::OrphanedFromDaemon,
                outcome: EngineCleanupOutcome::Stopped,
                failure: None,
            }],
            None,
            || Err(anyhow::anyhow!("falló el arranque sintético")),
        )
        .await
        {
            Ok(_) => panic!("el arranque sintético debía fallar"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("falló el arranque sintético"), "{error}");
        assert!(error.contains("PID 43906"), "{error}");
        assert!(
            error.contains("/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"),
            "{error}"
        );
        assert!(error.contains("se confirmó detenido"), "{error}");
    }

    #[tokio::test]
    async fn a1_a_real_test_socket_routes_its_peer_into_the_synthetic_census() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("old.sock");
        let _listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let peer = std::process::id();
        let uid = rustix::process::geteuid().as_raw();
        let listing = format!("{peer} 1 {uid} {}\n", binary.display());
        let calls = AtomicUsize::new(0);
        let signals = Mutex::new(Vec::new());
        let inspection = inspect_previous_socket(
            &binary,
            &socket,
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(listing.clone())
                } else {
                    Ok(String::new())
                }
            },
            |pid, signal| {
                signals.lock().unwrap().push((pid, signal));
                Ok(())
            },
            |_| async {},
        )
        .await;
        assert_eq!(inspection.cleanups.len(), 1);
        assert_eq!(inspection.cleanups[0].pid, peer);
        assert_eq!(signals.lock().unwrap().len(), 1);
        assert_eq!(
            u32::try_from(signals.lock().unwrap()[0].0.as_raw_nonzero().get()),
            Ok(peer)
        );
    }

    #[tokio::test]
    async fn a1_ps_fails_no_sweep() {
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let socket = PathBuf::from("/tmp/orbit-cua.sock");
        let peer_pid = 43_906;

        let killed = Arc::new(Mutex::new(Vec::new()));
        let killed_clone = killed.clone();

        let inspection = inspect_previous_listener(
            &binary,
            &socket,
            same_user(peer_pid),
            || async { Err(anyhow::Error::msg("timeout")) },
            |pid, sig| {
                killed_clone.lock().unwrap().push((pid, sig));
                Ok(())
            },
            |_| async {},
        )
        .await;

        assert_eq!(inspection.cleanups.len(), 0);
        assert!(inspection.notice.as_deref().is_some_and(
            |notice| notice.contains("timeout") && notice.contains("no se envió ninguna señal")
        ));
        assert_eq!(killed.lock().unwrap().len(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a1_ps_deadline_expires_without_a_signal() {
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let socket = PathBuf::from("/tmp/orbit-cua.sock");
        let signals = Mutex::new(Vec::new());
        let inspection = inspect_previous_listener(
            &binary,
            &socket,
            same_user(43_906),
            std::future::pending::<anyhow::Result<String>>,
            |pid, signal| {
                signals.lock().unwrap().push((pid, signal));
                Ok(())
            },
            |_| async {},
        )
        .await;
        assert!(inspection.cleanups.is_empty());
        assert!(signals.lock().unwrap().is_empty());
        assert!(inspection.notice.as_deref().is_some_and(
            |notice| notice.contains("2000 ms") && notice.contains("no se envió ninguna señal")
        ));
    }

    #[tokio::test]
    async fn a1_sweeps_parent_serve_if_orphaned() {
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let socket = PathBuf::from("/tmp/orbit-cua.sock");
        let child_pid = 43_906;
        let parent_pid = 43_905;

        let listing = format!(
            "  PID  PPID   UID COMM
{}     1   501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver
{} {}   501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver
",
            parent_pid, child_pid, parent_pid
        );

        let killed = Arc::new(Mutex::new(Vec::new()));
        let killed_clone = killed.clone();

        let inspection = inspect_previous_listener(
            &binary,
            &socket,
            same_user(child_pid),
            || async { Ok(listing.clone()) },
            |pid, sig| {
                killed_clone.lock().unwrap().push((pid, sig));
                Ok(())
            },
            |_| async {},
        )
        .await;

        assert_eq!(inspection.cleanups.len(), 2);
        assert_eq!(killed.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a2_failed_census_after_sigkill_is_unconfirmed_with_cause() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let socket = PathBuf::from("/tmp/orbit-cua.sock");
        let calls = AtomicUsize::new(0);
        let signals = Mutex::new(Vec::new());
        let inspection = inspect_previous_listener(
            &binary,
            &socket,
            same_user(43_906),
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok("43906 1 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n".into())
                } else {
                    Err(anyhow::Error::msg("ps no respondió en 2000 ms"))
                }
            },
            |pid, signal| {
                signals.lock().unwrap().push((pid, signal));
                Ok(())
            },
            |_| async {},
        )
        .await;
        assert_eq!(signals.lock().unwrap().len(), 1);
        assert_eq!(inspection.cleanups.len(), 1);
        assert_eq!(
            inspection.cleanups[0].outcome,
            orbit_core::topics::control::EngineCleanupOutcome::Unconfirmed
        );
        assert_eq!(
            inspection.cleanups[0].failure.as_deref(),
            Some("ps no respondió en 2000 ms")
        );
    }

    #[tokio::test]
    async fn a2_an_unrepresentable_parent_pid_keeps_its_failed_signal_receipt() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let binary = PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
        );
        let socket = PathBuf::from("/tmp/orbit-cua.sock");
        let calls = AtomicUsize::new(0);
        let signals = Mutex::new(Vec::new());
        let inspection = inspect_previous_listener(
            &binary, &socket, same_user(43_906),
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok("2147483648 1 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n43906 2147483648 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n".into())
                } else {
                    Ok(String::new())
                }
            },
            |pid, signal| { signals.lock().unwrap().push((pid, signal)); Ok(()) },
            |_| async {},
        ).await;
        assert_eq!(signals.lock().unwrap().len(), 1);
        assert_eq!(inspection.cleanups.len(), 2);
        assert!(inspection.cleanups.iter().any(|cleanup| {
            cleanup.pid == 2_147_483_648
                && cleanup.outcome
                    == orbit_core::topics::control::EngineCleanupOutcome::SignalFailed
                && cleanup
                    .failure
                    .as_deref()
                    .is_some_and(|failure| failure.contains("no cabe"))
        }));
    }
}
