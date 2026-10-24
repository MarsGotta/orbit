//! Bounded verified CUA MCP client. Orbit starts the engine itself, owns its
//! socket directory, and forwards whatever tools the engine declares.
//!
//! Its pieces: `engine` starts the engine and accepts its socket only from a
//! process it started; `previous_listener` sweeps the orphaned engine a
//! previous daemon left on that socket (macOS); `mcp` is the exchange with the
//! proxy, `transport` its bounded lines, and `observation` what a tool answered.
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
    io::{AsyncBufRead, AsyncWrite, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

// Every piece is named by its path, so that the contract tests, which compile
// this file on its own (`tests/cua_contract.rs`), find the same files.
#[path = "cua/engine.rs"]
pub(crate) mod engine;
#[path = "cua/mcp.rs"]
mod mcp;
#[path = "cua/observation.rs"]
mod observation;
#[cfg(any(target_os = "macos", test))]
#[path = "cua/previous_listener.rs"]
mod previous_listener;
#[path = "cua/transport.rs"]
mod transport;

use engine::{EngineGroup, driver_command, start_engine, verified_driver_binary};
use observation::parse_observation;
pub use observation::{NativeImage, NativeObservation};
pub use transport::{ResponseTooLarge, TooLargePart};

pub const VERIFIED_DRIVER_VERSION: &str = "0.28.2";
const VERIFIED_MCP_VERSION: &str = "2025-06-18";
/// The directory name Orbit creates, privately, under its own state root.
const SOCKET_DIRECTORY: &str = "cua";
const SOCKET_FILE: &str = "driver.sock";
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

/// A collected exit, before Orbit invalidates or kills the connection. Only
/// process identity and status cross into the visible notice; no error text.
#[derive(Debug)]
pub struct EngineExit {
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
    /// grants Accessibility and Screen Recording, and that is the
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

/// The socket path Orbit uses when nobody named one. It lives inside Orbit's
/// own state root, which Orbit can keep private.
pub fn default_socket_path(root: &Path) -> PathBuf {
    root.join(SOCKET_DIRECTORY).join(SOCKET_FILE)
}

/// Where the engine writes one full-screen capture: Orbit's private
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
    pub fn check_processes(&mut self) -> Result<()> {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_startup_cleanup_for_test(
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
}

/// Resolve the engine in its nested app unless development
/// explicitly supplies an external absolute path. `daemon_executable` is the
/// executable the daemon runs in: the app, `Contents/MacOS/Orbit`.
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
mod process_cleanup_tests {
    #![allow(clippy::unwrap_used)]

    use super::previous_listener::{ObservedCleanupOutcome, observed_cleanup_outcome};
    use orbit_core::process_listing::{EngineSweepReason, EngineSweepTarget};
    use std::path::PathBuf;

    #[tokio::test]
    async fn the_serve_process_exit_is_observed_without_an_mcp_failure() -> anyhow::Result<()> {
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
    fn an_exact_engine_still_listed_is_reported_as_surviving_sigkill() {
        let listing = "43906 1 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n";
        assert_eq!(
            observed_cleanup_outcome(listing, &target(), 501),
            ObservedCleanupOutcome::Survived
        );
    }

    #[test]
    fn absence_or_pid_reuse_is_reported_as_the_old_engine_stopped() {
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

    use super::engine::start_engine_after_inspection;
    use super::previous_listener::{
        ListenerIdentity, inspect_previous_listener, inspect_previous_socket,
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
            || Err(anyhow::anyhow!("synthetic start failed")),
        )
        .await
        {
            Ok(_) => panic!("the synthetic start had to fail"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("synthetic start failed"), "{error}");
        assert!(error.contains("PID 43906"), "{error}");
        assert!(
            error.contains("/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"),
            "{error}"
        );
        assert!(error.contains("se confirmó detenido"), "{error}");
    }

    #[tokio::test]
    async fn a_real_test_socket_routes_its_peer_into_the_synthetic_census() {
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
    async fn ps_fails_no_sweep() {
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
    async fn ps_deadline_expires_without_a_signal() {
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
    async fn sweeps_parent_serve_if_orphaned() {
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
    async fn failed_census_after_sigkill_is_unconfirmed_with_cause() {
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
    async fn an_unrepresentable_parent_pid_keeps_its_failed_signal_receipt() {
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
