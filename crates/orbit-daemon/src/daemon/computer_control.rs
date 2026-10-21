//! One explicit, ephemeral Cua connection. SQLite commits occur only in the
//! actor, never across an RPC; workers own the transport until their return.
use super::ticketed::{Settlement, Ticketed, WorkTicket};
use super::{Runtime, WorkResult, actions::ActionService, id, now};
use anyhow::{Context, Result, bail};
use orbit_core::authority::AuthorityChange;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::{
    Connection,
    actions::{ActionAuthority, ComputerControlView, ComputerStage},
    computer_control::{
        ControlHistoryEntry, ControlOutcome, MAX_CONTROL_HISTORY, ToolCall, engine,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// What one screen observation may carry: an app list, a window list or an
/// action receipt. Why this number and
/// what it protects, followed through the code: the value lands in
/// `view.observation`, which `computer_presentation_state` copies into the
/// `State` that the daemon loop rebuilds, compares and publishes on every
/// iteration and on every IPC response, so it is not a one-off payload but a
/// per-tick cost inside the 1 MiB IPC frame shared with recordings, calendar
/// and transcripts. On top of that, the contents of a page are not the user's to
/// bound, so the bound refuses rather than truncates: acting on a cut-down view
/// means clicking with incomplete information. 32 KiB came from the largest
/// browser input Orbit used to prepare from an observation; that input was
/// retired later and the number was kept, not derived again.
/// Left unchanged on purpose when the engine's catalogue outgrew it: the
/// catalogue got its own bound instead.
const MAX_OBSERVATION_BYTES: usize = 32 * 1024;
/// How long Orbit waits before starting another engine after one failed to
/// come up. Short enough to recover unattended, long enough not to spin.
const ENGINE_RETRY_DELAY: Duration = Duration::from_secs(5);
/// How often the daemon asks the helper again what macOS grants. The helper
/// reports the permissions when it starts and when one is asked for, never
/// when the user changes one in System Settings, so without asking
/// again a withdrawal would reach the engine only after a restart. What it
/// protects: how long the engine keeps running, and Orbit keeps saying it can
/// act, after the user withdraws a permission. Why 5 s: the same pause the engine
/// takes before a retry, well under the time it takes to come back from System
/// Settings to Orbit, and one status answer of a few hundred bytes every 5 s
/// is nothing beside the ~94 audio frames a second the same pipe carries. What
/// it is not: a security bound. macOS enforces a withdrawal itself on the next
/// call the engine makes; this is how soon Orbit says so and stops it.
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How long one of those status requests may stay unanswered before it is
/// given up. What it protects: the pending-order list (`pending_native`,
/// bounded at 64 by eviction). Only one poll is ever outstanding, so a helper
/// that never answers costs one entry rather than filling the list and
/// evicting the name a real order's answer needs. Why two intervals: the
/// helper answers `status` on its main thread in milliseconds, so an answer
/// two polls late is not coming — typically the helper was replaced, and its
/// successor never saw the question. What it is not: a liveness check of the
/// helper (the heartbeat watchdog is), nor a retry of anything with an effect:
/// asking again what macOS grants repeats nothing.
const PERMISSION_POLL_ANSWER_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct ComputerRuntime {
    /// Supplied by isolated actor fixtures; production resolves its own binary.
    #[cfg(test)]
    pub(in crate::daemon) daemon_executable: Option<PathBuf>,
    service: Option<ActionService>,
    /// The one worker holding the engine or a borrowed detector read. A
    /// second order is refused until this one settles or is revoked; it is
    /// not a limit on tools, observations or history.
    pub(in crate::daemon) work: Ticketed<()>,
    authority: Option<watch::Sender<WorkerAuthority>>,
    pending_cancel_uncertain: bool,
    view: Option<ComputerControlView>,
    /// The tool call in flight. Visible to the topic projector's tests.
    pub(in crate::daemon) active_general: Option<ControlHistoryEntry>,
    /// Earliest moment a new engine may be started after one failed to come up.
    engine_retry_at: Option<Instant>,
    /// The same retry deadline on the wall clock, captured once for topic
    /// consumers; it must not be recalculated on every daemon turn.
    pub(in crate::daemon) engine_retry_at_ms: Option<i64>,
    /// Why the engine is not available, so a remote request can say the cause
    /// instead of quietly taking a different route. Visible to the topic
    /// projector's tests, which arrange engine states directly.
    pub(in crate::daemon) engine_failure: Option<String>,
    /// Which of the two ways that failure happened: the engine never came up,
    /// or it dropped while it was running. The control topic publishes this
    /// distinction because the shared refusal carries a cause,
    /// not the daemon's detail text.
    pub(in crate::daemon) engine_failure_kind: Option<orbit_core::refusal::EngineCause>,
    /// How many tools the connected engine declared, kept while a worker has
    /// the connection so the status does not go blank during every action.
    /// Visible to the topic projector's tests.
    pub(in crate::daemon) declared_tools: Option<u32>,
    /// When the helper is next asked what macOS grants. `None`: at once.
    next_permission_poll: Option<Instant>,
    /// The one status request still waiting for its answer, and when it went.
    permission_poll: Option<(String, Instant)>,
    /// The scale of the last full screen Deimos was sent, which its
    /// desktop actions are given on. Memory only: after a restart Deimos has
    /// to look again, and a desktop action before that is refused by name.
    pub(in crate::daemon) desktop_scale: Option<orbit_core::sources::DesktopScale>,
    /// The full screen being uploaded, by capture id, with its scale; it
    /// becomes `desktop_scale` only when the reply carrying it is sent.
    pub(in crate::daemon) pending_desktop: Option<(String, orbit_core::sources::DesktopScale)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ComputerRevocation {
    Authority(AuthorityChange),
    Refusal(orbit_core::refusal::Refusal),
}

#[derive(Debug, Clone)]
struct WorkerAuthority {
    grant: ActionAuthority,
    revoked: Option<ComputerRevocation>,
}

impl std::ops::Deref for WorkerAuthority {
    type Target = ActionAuthority;

    fn deref(&self) -> &Self::Target {
        &self.grant
    }
}

impl WorkerAuthority {
    fn active(grant: ActionAuthority) -> Self {
        Self {
            grant,
            revoked: None,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DurableControlState {
    schema: u8,
    history: Vec<ControlHistoryEntry>,
}

impl Default for DurableControlState {
    fn default() -> Self {
        Self {
            schema: 1,
            history: Vec::new(),
        }
    }
}

pub(super) struct ComputerCompletion {
    service: ActionService,
    result: Result<ComputerResult>,
}

pub(super) enum ComputerResult {
    /// A connection that came up, with the engine's own facts. It is not a
    /// window listing and never answers a request (taken for one in a real
    /// call on 2026-10-18, it reached a screenshot the helper still had in
    /// hand and failed it).
    Connected(Value),
    Read(Value),
    BridgeScreenshot {
        observation: crate::cua::NativeObservation,
        application: String,
        title: String,
    },
    GeneralAction {
        operation_id: String,
        receipt: Value,
    },
    /// The full screen, read from its file and fitted for delivery.
    DesktopCapture {
        fitted: orbit_core::sources::FittedDesktop,
    },
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ComputerStartError {
    #[error("{refusal}")]
    Authority {
        refusal: orbit_core::refusal::Refusal,
    },
    #[error("El Mac no está emparejado con el hub")]
    NotPaired,
    #[error("{0}")]
    Busy(String),
    #[error("La llamada a la herramienta del motor no es válida o supera sus límites")]
    InvalidTool,
    #[error("El motor no declara la herramienta «{0}»")]
    UndeclaredTool(String),
    #[error("No se pudo construir un recibo válido para esta acción")]
    InvalidHistory,
    #[error("No se pudo guardar el historial de la acción: {0}")]
    Storage(#[source] anyhow::Error),
    #[error("El motor no está disponible")]
    EngineUnavailable,
    #[error("No se pudo preparar el directorio privado donde el motor deja la captura: {0}")]
    CaptureFile(#[source] anyhow::Error),
    /// A desktop action's coordinates, refused with their cause.
    #[error("{0}")]
    DesktopCoordinates(orbit_core::refusal::Refusal),
    #[error("La autoridad del worker desapareció antes del envío")]
    Internal,
}

fn remote_engine_refusal(_system_error: &str) -> orbit_core::refusal::Refusal {
    orbit_core::refusal::Refusal::EngineUnavailable {
        cause: orbit_core::refusal::EngineCause::Unhealthy,
    }
}

impl Runtime {
    fn computer_start_authority(&self) -> std::result::Result<ActionAuthority, ComputerStartError> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(ComputerStartError::Authority { refusal });
        }
        let authority = self.computer_authority();
        if authority.device_id.is_empty() {
            return Err(ComputerStartError::NotPaired);
        }
        Ok(authority)
    }

    /// A connection epoch is evidence of a live owner, never detector permission.
    pub(super) fn computer_connection_epoch(&self) -> Option<&str> {
        self.features
            .computer
            .service
            .as_ref()
            .filter(|service| service.client().is_connected())
            .map(|service| service.client().connection_epoch())
            .or_else(|| {
                self.features.computer.work.active().as_ref()?;
                self.features
                    .computer
                    .view
                    .as_ref()
                    .filter(|view| view.connected)?
                    .connection_epoch
                    .as_deref()
            })
    }

    fn computer_authority(&self) -> ActionAuthority {
        ActionAuthority {
            device_id: self.settings.device_id.clone().unwrap_or_default(),
            generation: self.state.generation(),
            now_ms: now(),
            online: self.state.connection() == Connection::Online,
            privacy_stopped: self.state.privacy_stopped(),
        }
    }

    /// Why the engine may not start work now, if it may not: the privacy
    /// stop, or a permission macOS does not grant or has not reported yet.
    /// Nothing else. No switch, and neither the hub nor the
    /// conversation, which only matter to a request that came through them.
    pub(super) fn computer_refusal(&self) -> Option<orbit_core::refusal::Refusal> {
        engine::admits(engine::SystemPermissions::ENGINE, self.engine_observed()).err()
    }

    /// What the Mac has told the daemon about what macOS grants, and whether
    /// the helper that tells it is there at all.
    pub(super) fn engine_observed(&self) -> engine::Observed {
        engine::Observed {
            privacy_stopped: self.state.privacy_stopped(),
            helper_connected: self.state.capabilities().native_helper(),
            accessibility: self.state.capabilities().accessibility_state(),
            screen_recording: self.state.capabilities().screen_state(),
        }
    }

    /// Why work already running on the engine must stop, if it must: only the
    /// known facts, the privacy stop and a permission macOS withdrew. A
    /// permission that is merely not known right now leaves it running.
    fn computer_revocation(&self) -> Option<orbit_core::refusal::Refusal> {
        engine::revokes(self.engine_observed())
    }

    fn require_computer_authority(&self) -> Result<ActionAuthority> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(refusal.into());
        }
        let authority = self.computer_authority();
        // Each receipt is filed under the paired device; without one there is
        // nothing to file it under. This is the pairing, not a second consent.
        if authority.device_id.is_empty() {
            bail!(
                "El control guarda cada recibo a nombre del Mac emparejado, y este Mac no está emparejado con el hub"
            );
        }
        Ok(authority)
    }

    fn require_computer_idle(&self) -> Result<()> {
        if self.features.computer.work.active().is_some() {
            bail!("Hay una operación de control en curso; no se repetirá el input");
        }
        Ok(())
    }

    pub(super) fn control_history(&self) -> Result<Vec<ControlHistoryEntry>> {
        Ok(self.load_control_state()?.history)
    }

    fn control_state_path(&self) -> PathBuf {
        self.paths.root.join("computer-control.json")
    }

    fn load_control_state(&self) -> Result<DurableControlState> {
        let bytes = match std::fs::read(self.control_state_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DurableControlState::default());
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() > 256 * 1024 {
            bail!("El estado local de control supera 256 KiB");
        }
        let value: DurableControlState = serde_json::from_slice(&bytes)?;
        if value.schema != 1
            || value.history.len() > MAX_CONTROL_HISTORY
            || value.history.iter().any(|entry| !entry.is_valid())
        {
            bail!("El estado local de control no es válido");
        }
        Ok(value)
    }

    fn save_control_state(&self, value: &DurableControlState) -> Result<()> {
        if value.schema != 1
            || value.history.len() > MAX_CONTROL_HISTORY
            || value.history.iter().any(|entry| !entry.is_valid())
        {
            bail!("El estado local de control no es válido");
        }
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > 256 * 1024 {
            bail!("El estado local de control supera 256 KiB");
        }
        crate::config::atomic_private_write(&self.control_state_path(), &bytes)
    }

    /// The engine's whole lifecycle, decided by `engine::decide` from
    /// what macOS grants and the privacy stop. It starts with both
    /// permissions and no order, switch, pairing or hub; it stops, naming the
    /// cause, when the privacy stop is latched or macOS withdraws one; and a
    /// permission that is not known yet neither starts it nor stops it.
    pub(super) async fn maintain_engine(&mut self) {
        if !self.closing
            && let Some(service) = self.features.computer.service.as_mut()
            && let Err(error) = service.client_mut().check_processes()
        {
            self.computer_failed(&error).await;
        }
        let running = self.features.computer.service.is_some()
            || self.features.computer.work.active().is_some();
        let facts = engine::Facts {
            observed: self.engine_observed(),
            closing: self.closing,
            running,
            retry_due: self
                .features
                .computer
                .engine_retry_at
                .is_none_or(|deadline| Instant::now() >= deadline),
        };
        match engine::decide(facts) {
            engine::Decision::Keep => {}
            engine::Decision::Stop(refusal) => self.revoke_computer(&refusal).await,
            engine::Decision::StopForExit => {
                self.revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
                    cause: orbit_core::refusal::ReadInterruption::Authority {
                        change: AuthorityChange::Closing,
                    },
                })
                .await
            }
            engine::Decision::Start => {
                if let Err(error) = self.connect_computer() {
                    // It never came up: the start itself failed.
                    self.engine_unavailable(
                        orbit_core::refusal::EngineCause::InitializationFailed,
                        &error,
                        &[],
                    );
                }
            }
        }
    }

    /// Asks the helper again what macOS grants, at a fixed interval, while
    /// there is something to notice: an engine running (a withdrawal must stop
    /// it), a permission known to be missing (a grant must start it) or one
    /// still unknown (the status from `ready` may have answered nothing). The answer arrives as an ordinary status response and
    /// updates the permissions in three values; the next maintenance decides
    /// from them. Never waits: a helper whose pipe is full skips a turn rather
    /// than stalling the actor.
    pub(super) fn poll_permissions(&mut self) {
        let engine_running = self.features.computer.service.is_some()
            || self.features.computer.work.active().is_some();
        // Unknown too: a `status` that answered `ok:false` leaves the
        // permissions unknown with the engine stopped, and nothing else
        // would ever ask again.
        let permission_missing = matches!(
            self.computer_refusal(),
            Some(
                orbit_core::refusal::Refusal::PermissionDenied { .. }
                    | orbit_core::refusal::Refusal::PermissionNotGranted { .. }
                    | orbit_core::refusal::Refusal::PermissionUnknown { .. }
            )
        );
        let now = Instant::now();
        if !self.state.capabilities().native_helper()
            || !(engine_running || permission_missing)
            || self
                .features
                .computer
                .next_permission_poll
                .is_some_and(|due| now < due)
        {
            return;
        }
        // One question at a time: the answer to the last one removes it from
        // the pending list. Until then, or until it is given up, nothing new
        // is asked, so an unanswered poll never piles up there.
        if let Some((id, sent_at)) = self.features.computer.permission_poll.clone()
            && self.pending_native.contains_key(&id)
        {
            if now.saturating_duration_since(sent_at) < PERMISSION_POLL_ANSWER_DEADLINE {
                return;
            }
            self.pending_native.remove(&id);
        }
        let command = match crate::native::command("status", json!({})) {
            Ok(command) => command,
            Err(error) => {
                self.log(
                    "native_command_invalid",
                    &[("command", "status"), ("cause", &error.to_string())],
                );
                return;
            }
        };
        if self.native.try_send(command.clone()).is_ok() {
            self.record_pending_native(&command);
            self.features.computer.permission_poll =
                command["id"].as_str().map(|id| (id.to_owned(), now));
        }
        self.features.computer.next_permission_poll = Some(now + PERMISSION_POLL_INTERVAL);
    }

    /// Says that the engine failed, never quietly: the cause's text,
    /// with `context` (which request, which operation), in daemon.log, and a
    /// typed `engine_failed` line in the report log, both under the turn's op.
    /// The text may name paths on the Mac, so it never leaves daemon.log.
    fn note_engine_failure(
        &mut self,
        kind: orbit_core::refusal::EngineCause,
        error: &anyhow::Error,
        context: &[(&str, &str)],
    ) {
        let kind_token = serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unrecognized".into());
        let cause = format!("{error:#}");
        let mut fields = vec![("kind", kind_token.as_str()), ("cause", cause.as_str())];
        fields.extend_from_slice(context);
        self.log("engine_failed", &fields);
        let mut notice =
            Notice::new(NoticeCode::EngineFailed).with_token(NoticeKey::Cause, &kind_token);
        notice = if let Some(too_large) = error.downcast_ref::<ReceiptTooLarge>() {
            notice
                .with_token(NoticeKey::Code, "receipt_too_large")
                .with(NoticeKey::Bytes, too_large.bytes)
                .with(NoticeKey::MaxBytes, too_large.max_bytes)
        } else if let Some(too_large) = error.downcast_ref::<crate::cua::ResponseTooLarge>() {
            notice
                .with_token(NoticeKey::Code, "response_too_large")
                .with(NoticeKey::Bytes, too_large.bytes)
                .with(NoticeKey::MaxBytes, too_large.max_bytes)
        } else {
            notice.with_token(NoticeKey::Code, "engine_error")
        };
        if context.iter().any(|(key, _)| *key == "fallback") {
            notice = notice.with_token(NoticeKey::Step, "helper_fallback");
        }
        self.record(notice);
        if let Some(exit) = error.downcast_ref::<crate::cua::EngineExit>() {
            use std::os::unix::process::ExitStatusExt;
            let (code, status) = if let Some(code) = exit.status.code() {
                ("exit", code)
            } else if let Some(signal) = exit.status.signal() {
                ("signal", signal)
            } else {
                ("wait_status", exit.status.into_raw())
            };
            self.say(
                Notice::new(NoticeCode::EngineExited)
                    .with_token(NoticeKey::Cause, exit.process)
                    .with_token(NoticeKey::Code, code)
                    .with(NoticeKey::Status, i64::from(status)),
            );
        }
    }

    /// Records why the engine is missing and when Orbit may try again.
    /// `kind` says which of the two failures it was — never came up, or
    /// dropped while running — for the published cause.
    pub(super) fn engine_unavailable(
        &mut self,
        kind: orbit_core::refusal::EngineCause,
        error: &anyhow::Error,
        context: &[(&str, &str)],
    ) {
        self.note_engine_failure(kind, error, context);
        self.engine_unavailable_state(kind, &format!("{error:#}"));
    }

    /// The published side of an engine failure: its cause for the app and
    /// the remote caller, and when Orbit tries again.
    fn engine_unavailable_state(&mut self, kind: orbit_core::refusal::EngineCause, cause: &str) {
        self.features.computer.engine_failure = Some(cause.chars().take(512).collect());
        self.features.computer.engine_failure_kind = Some(kind);
        self.features.computer.engine_retry_at = Some(Instant::now() + ENGINE_RETRY_DELAY);
        let delay_ms = i64::try_from(ENGINE_RETRY_DELAY.as_millis()).unwrap_or(i64::MAX);
        self.features.computer.engine_retry_at_ms = Some(now().saturating_add(delay_ms));
    }

    /// The engine as the app shows it and a remote caller is told about it.
    /// A known refusal comes first even while an engine is still connected:
    /// it is what the next request gets, and the next turn stops the engine.
    pub(super) fn engine_status(&self) -> engine::Status {
        if let Some(refusal) = self.computer_revocation()
            && let Some(status) = engine::Status::refused(refusal)
        {
            return status;
        }
        let connected = self
            .features
            .computer
            .service
            .as_ref()
            .is_some_and(|service| service.client().is_connected());
        let connecting = self.features.computer.work.active().is_some()
            && self
                .features
                .computer
                .view
                .as_ref()
                .is_some_and(|view| view.stage == ComputerStage::Connecting);
        // The count taken when the engine connected: this runs for every state
        // the daemon publishes, and counting the catalogue again would copy all
        // of it (147 909 bytes on a MacBook) each time.
        if (connected || self.features.computer.work.active().is_some() && !connecting)
            && let Some(tools) = self.features.computer.declared_tools
        {
            return engine::Status::Ready { tools };
        }
        if connected {
            // Only an engine handed over without connecting here has no count.
            let tools = self
                .features
                .computer
                .service
                .as_ref()
                .map_or(0, |service| service.client().engine_tools().len());
            return engine::Status::Ready {
                tools: u32::try_from(tools).unwrap_or(u32::MAX),
            };
        }
        if let Some(refusal) = self.computer_refusal()
            && let Some(status) = engine::Status::refused(refusal)
        {
            return status;
        }
        if connecting {
            return engine::Status::Starting;
        }
        if let Some(cause) = &self.features.computer.engine_failure {
            let retry_in_ms = self
                .features
                .computer
                .engine_retry_at
                .map_or(0, |deadline| {
                    u64::try_from(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .as_millis(),
                    )
                    .unwrap_or(u64::MAX)
                });
            return engine::Status::Failed {
                cause: cause.clone(),
                retry_in_ms,
            };
        }
        engine::Status::Starting
    }

    pub(super) fn engine_is_ready(&self) -> bool {
        self.features
            .computer
            .service
            .as_ref()
            .is_some_and(|service| service.client().is_connected())
    }

    /// Which of the two ways the engine failed, for the control topic's
    /// published cause: it never came up, or it dropped while running.
    pub(super) fn engine_failure_cause(&self) -> Option<orbit_core::refusal::EngineCause> {
        self.features.computer.engine_failure_kind
    }

    /// The tool call in flight, as its history entry will record it, for the
    /// control topic's head. The browser-input path has no entry here: it
    /// publishes through the engine state and its own recorded outcome.
    pub(super) fn active_control_action(&self) -> Option<&ControlHistoryEntry> {
        self.features.computer.active_general.as_ref()
    }

    /// Asked of the engine that is connected right now, so the answer cannot
    /// drift from what the engine will actually accept.
    pub(super) fn engine_tools(&self) -> Vec<orbit_core::computer_control::EngineTool> {
        self.features
            .computer
            .service
            .as_ref()
            .filter(|service| service.client().is_connected())
            .map(|service| service.client().engine_tools())
            .unwrap_or_default()
    }

    fn upsert_control_history(&self, entry: ControlHistoryEntry) -> Result<()> {
        let mut state = self.load_control_state()?;
        if let Some(existing) = state
            .history
            .iter_mut()
            .find(|existing| existing.operation_id == entry.operation_id)
        {
            // A receipt updates the same action. Moving it before a cursor
            // would make the next history page repeat earlier entries.
            *existing = entry;
        } else {
            state.history.insert(0, entry);
        }
        state.history.truncate(MAX_CONTROL_HISTORY);
        self.save_control_state(&state)
    }

    /// The packaged driver beside the daemon's own executable, and the private
    /// socket under Orbit's state root. Nothing outside the daemon chooses them.
    fn engine_paths(&self) -> Result<(PathBuf, PathBuf)> {
        #[cfg(test)]
        let daemon_executable = match &self.features.computer.daemon_executable {
            Some(path) => path.clone(),
            None => std::env::current_exe()
                .context("No se pudo localizar el ejecutable del daemon para buscar CUA")?,
        };
        #[cfg(not(test))]
        let daemon_executable = std::env::current_exe()
            .context("No se pudo localizar el ejecutable del daemon para buscar CUA")?;
        let daemon_executable = daemon_executable
            .canonicalize()
            .context("No se pudo resolver el ejecutable del daemon para buscar CUA")?;
        let driver = crate::cua::resolve_driver_path("", &daemon_executable)?;
        Ok((driver, crate::cua::default_socket_path(&self.paths.root)))
    }

    /// Starts the packaged engine. It needs what macOS grants and
    /// the privacy stop released, and nothing else — not the pairing, not the
    /// hub, and no order naming an executable.
    pub(super) fn connect_computer(&mut self) -> Result<()> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(refusal.into());
        }
        let generation = self.state.generation();
        self.require_computer_idle()?;
        if self.features.computer.service.is_some() {
            bail!("Desconecta el control actual antes de abrir otra conexión");
        }
        let (driver_path, socket_path) = self.engine_paths()?;
        if driver_path.as_os_str().len() > 4096 || socket_path.as_os_str().len() > 4096 {
            bail!("La ruta del motor CUA empaquetado supera el límite de 4096 bytes");
        }
        let events = self.work.work_events.clone();
        self.begin_computer(
            ComputerStage::Connecting,
            "Conectando al motor CUA local; no se inicia ningún navegador.",
            generation,
            move |ticket| {
                tokio::spawn(async move {
                    let result = ActionService::connect(crate::cua::CuaConfig {
                        driver_path,
                        socket_path,
                        request_timeout: Duration::from_secs(5),
                    })
                    .await;
                    let event = result.and_then(connected);
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Control(crate::daemon::ControlWork::Computer {
                            ticket,
                            event: Box::new(event),
                        }),
                    )
                    .await;
                })
            },
        )?;
        Ok(())
    }

    /// The engine's window list, which Deimos asks for through the bridge.
    pub(super) fn read_computer_windows(&mut self) -> Result<()> {
        let authority = self.require_computer_authority()?;
        self.require_computer_idle()?;
        let mut service = self
            .features
            .computer
            .service
            .take()
            .context("Conecta explícitamente el control primero")?;
        let events = self.work.work_events.clone();
        self.begin_computer(
            ComputerStage::Reading,
            "Observando el destino elegido; el contenido no autoriza acciones.",
            authority.generation,
            move |ticket| {
                tokio::spawn(async move {
                    let result = async {
                        let data = bounded_observation(&service.windows(None).await?)?;
                        Ok(ComputerResult::Read(data))
                    }
                    .await;
                    let event = ComputerCompletion { service, result };
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Control(crate::daemon::ControlWork::Computer {
                            ticket,
                            event: Box::new(Ok(event)),
                        }),
                    )
                    .await;
                })
            },
        )?;
        Ok(())
    }

    pub(super) fn read_bridge_screenshot(
        &mut self,
        pid: Option<i64>,
        window_id: Option<u64>,
        application: String,
        title: String,
    ) -> std::result::Result<(), ComputerStartError> {
        let authority = self.computer_start_authority()?;
        self.require_computer_idle()
            .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        // The full screen goes to a file of Orbit's, not into the
        // engine's answer, where a Retina screen does not fit.
        let desktop_file = match (pid, window_id) {
            (None, None) => Some(
                crate::cua::desktop_capture_path(&self.paths.root, &id())
                    .map_err(ComputerStartError::CaptureFile)?,
            ),
            _ => None,
        };
        let mut service = self
            .features
            .computer
            .service
            .take()
            .ok_or(ComputerStartError::EngineUnavailable)?;
        let events = self.work.work_events.clone();
        self.begin_computer(
            ComputerStage::Reading,
            "Capturando la ventana elegida con el motor general.",
            authority.generation,
            move |ticket| {
                tokio::spawn(async move {
                    let result = match (pid, window_id, desktop_file) {
                        (Some(pid), Some(window_id), _) => service
                            .client_mut()
                            .call_tool(
                                "get_window_state",
                                json!({
                                    "pid":pid,
                                    "window_id":window_id,
                                    "include_accessibility_tree":false,
                                    "include_screenshot":true,
                                    "max_dimension":1280
                                }),
                            )
                            .await
                            .map(|observation| ComputerResult::BridgeScreenshot {
                                observation,
                                application,
                                title,
                            }),
                        (None, None, Some(path)) => desktop_capture(&mut service, path)
                            .await
                            .map(|fitted| ComputerResult::DesktopCapture { fitted }),
                        _ => Err(anyhow::anyhow!("Destino de captura incompleto")),
                    };
                    let event = ComputerCompletion { service, result };
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Control(crate::daemon::ControlWork::Computer {
                            ticket,
                            event: Box::new(Ok(event)),
                        }),
                    )
                    .await;
                })
            },
        )
        .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        Ok(())
    }

    /// The view and authority side of starting computer work: stage, detail
    /// and the revoked-by-default authority channel. No worker yet, so no
    /// ticket either; [`Self::begin_computer`] joins both halves.
    fn begin_computer_view(&mut self, stage: ComputerStage, detail: &str, generation: u64) {
        self.features.computer.pending_cancel_uncertain = false;
        let connected = !matches!(stage, ComputerStage::Connecting);
        let epoch = self
            .features
            .computer
            .view
            .as_ref()
            .and_then(|v| v.connection_epoch.clone());
        let mut authority = self.computer_authority();
        authority.generation = generation;
        self.features.computer.authority =
            Some(watch::channel(WorkerAuthority::active(authority)).0);
        self.features.computer.view = Some(ComputerControlView {
            request_id: String::new(),
            stage,
            connected,
            connection_epoch: epoch,
            detail: detail.into(),
            observation: None,
        });
    }

    /// Start the ticketed computer worker and stamp the view with the
    /// ticket's canonical UUID, as the same identifier it has always been.
    /// The view half must already be in place.
    fn start_computer_work(
        &mut self,
        generation: u64,
        worker: impl FnOnce(WorkTicket) -> tokio::task::JoinHandle<()>,
    ) -> Result<WorkTicket> {
        let ticket = self
            .features
            .computer
            .work
            .start_logical(generation, self.turn.op.clone(), (), worker)
            .ok_or(orbit_core::refusal::Refusal::WorkInFlight {
                in_flight: 1,
                max: 1,
            })?;
        if let Some(view) = self.features.computer.view.as_mut() {
            view.request_id = ticket.id().to_owned();
        }
        Ok(ticket)
    }

    /// Start one computer worker: the view half, then the ticketed worker
    /// with its `request_id`. Sites that must subscribe to the authority
    /// channel between both halves call the halves separately.
    fn begin_computer(
        &mut self,
        stage: ComputerStage,
        detail: &str,
        generation: u64,
        worker: impl FnOnce(WorkTicket) -> tokio::task::JoinHandle<()>,
    ) -> Result<WorkTicket> {
        self.begin_computer_view(stage, detail, generation);
        self.start_computer_work(generation, worker)
    }

    /// One tool the engine declared, run now. No preparation, no
    /// confirmation, no short list and no per-application grant: the macOS
    /// permission was the decision. Without an engine this fails by name; it
    /// never diverts to another path.
    pub(super) fn call_engine_tool(
        &mut self,
        call: ToolCall,
        channel: &str,
    ) -> std::result::Result<(), ComputerStartError> {
        call.validate()
            .map_err(|_| ComputerStartError::InvalidTool)?;
        // A desktop action is given on the fitted image Deimos was
        // sent; the engine takes true screen pixels.
        let call = match orbit_core::sources::map_desktop_action(
            &call.arguments,
            self.features.computer.desktop_scale.as_ref(),
        ) {
            Ok(Some(arguments)) => ToolCall {
                tool: call.tool,
                arguments,
            },
            Ok(None) => call,
            Err(cause) => return Err(ComputerStartError::DesktopCoordinates(cause)),
        };
        let authority = self.computer_start_authority()?;
        self.require_computer_idle()
            .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        if !self.engine_is_ready() {
            return Err(ComputerStartError::EngineUnavailable);
        }
        if !self
            .features
            .computer
            .service
            .as_ref()
            .is_some_and(|service| service.client().declares_tool(&call.tool))
        {
            return Err(ComputerStartError::UndeclaredTool(call.tool));
        }
        let operation_id = id();
        let history = ControlHistoryEntry::new(
            &operation_id,
            &call.tool,
            &tool_application(&call),
            now(),
            ControlOutcome::Unknown,
        )
        .and_then(|entry| entry.from_channel(channel))
        .map_err(|_| ComputerStartError::InvalidHistory)?;
        self.upsert_control_history(history.clone())
            .map_err(ComputerStartError::Storage)?;
        self.features.computer.active_general = Some(history);
        let mut service = self
            .features
            .computer
            .service
            .take()
            .ok_or(ComputerStartError::EngineUnavailable)?;
        self.begin_computer_view(
            ComputerStage::Executing,
            &format!(
                "Ejecutando «{}» con el motor; un recibo perdido no se repite.",
                call.tool
            ),
            authority.generation,
        );
        let current = self
            .features
            .computer
            .authority
            .as_ref()
            .ok_or(ComputerStartError::Internal)?
            .subscribe();
        let events = self.work.work_events.clone();
        let generation = authority.generation;
        self.start_computer_work(generation, move |ticket| {
            tokio::spawn(async move {
                // Not the hub: a request that came through it was already admitted,
                // and losing it now only means the answer cannot be delivered.
                let still_authorized = || {
                    let value = current.borrow().clone();
                    value.revoked.is_none()
                        && !value.grant.privacy_stopped
                        && value.grant.generation == generation
                };
                // The ticket is cancelled with the worker; no unreachable
                // error text should misname a local cancellation as privacy.
                if !still_authorized() {
                    return;
                }
                let result = service
                    .client_mut()
                    .call_tool(&call.tool, call.arguments.clone())
                    .await;
                let result = result.and_then(|receipt| {
                    Ok(ComputerResult::GeneralAction {
                        operation_id,
                        receipt: orbit_core::limits::bounded_action_receipt(&receipt)
                            .map_err(|text| receipt_refused(&receipt, text))?,
                    })
                });
                let event = ComputerCompletion { service, result };
                crate::daemon::deliver_work(
                    &events,
                    WorkResult::Control(crate::daemon::ControlWork::Computer {
                        ticket,
                        event: Box::new(Ok(event)),
                    }),
                )
                .await;
            })
        })
        .map_err(|_| ComputerStartError::Internal)?;
        Ok(())
    }

    /// Invalidate before any fallible write or await. A queued late result cannot
    /// restore the client: its ticket is removed before the worker is aborted.
    pub(super) async fn revoke_computer(&mut self, refusal: &orbit_core::refusal::Refusal) {
        self.revoke_computer_with_cause(
            &refusal.to_string(),
            ComputerRevocation::Refusal(refusal.clone()),
        )
        .await;
    }

    pub(super) async fn revoke_computer_with_cause(
        &mut self,
        detail: &str,
        cause: ComputerRevocation,
    ) {
        let retiring_remote_control =
            self.conversation
                .bridge_request
                .as_ref()
                .is_some_and(|ask| {
                    matches!(
                        ask.capability,
                        orbit_core::device_bridge::Capability::PerformAction
                            | orbit_core::device_bridge::Capability::ControlTools
                    )
                });
        if retiring_remote_control {
            match &cause {
                ComputerRevocation::Authority(change) => {
                    let change = *change;
                    let reason =
                        if matches!(change, AuthorityChange::UserStop | AuthorityChange::Revoked) {
                            super::BridgeReleaseReason::PrivacyStopped
                        } else {
                            super::BridgeReleaseReason::Authority(change)
                        };
                    self.release_bridge_request(reason);
                }
                ComputerRevocation::Refusal(refusal) => {
                    self.release_bridge_request(super::BridgeReleaseReason::ComputerRevoked(
                        refusal,
                    ));
                }
            }
        }
        let had_active = self.features.computer.work.active().is_some()
            || self.features.computer.authority.is_some()
            || self.features.computer.service.is_some()
            || self.features.computer.active_general.is_some();
        let in_flight = self.features.computer.active_general.is_some();
        let mut history = self.features.computer.active_general.take();
        if let Some(entry) = history.as_mut() {
            entry.outcome = if in_flight {
                ControlOutcome::Unknown
            } else {
                ControlOutcome::Cancelled
            };
        }
        self.features.computer.pending_cancel_uncertain |= in_flight;
        self.revoke_computer_state(detail, had_active, cause);
        if let Some(entry) = history {
            let _ = self.upsert_control_history(entry);
        }
    }

    fn revoke_computer_state(&mut self, detail: &str, had_active: bool, cause: ComputerRevocation) {
        if let Some(authority) = self.features.computer.authority.take() {
            authority.send_replace(WorkerAuthority {
                grant: self.computer_authority(),
                revoked: Some(cause),
            });
        }
        self.features.computer.work.cancel();
        if let Some(mut service) = self.features.computer.service.take() {
            service.invalidate();
        }
        self.features.computer.declared_tools = None;
        if had_active && let Some(view) = self.features.computer.view.as_mut() {
            view.connected = false;
            view.connection_epoch = None;
            view.observation = None;
            view.stage = if self.features.computer.pending_cancel_uncertain {
                ComputerStage::Unknown
            } else {
                ComputerStage::Cancelled
            };
            view.detail = detail.into();
        }
    }

    pub(super) async fn computer_event(
        &mut self,
        ticket: &WorkTicket,
        event: Result<ComputerCompletion>,
    ) -> Result<()> {
        if self.features.computer.work.ticket() != Some(ticket) {
            return Ok(());
        }
        // Revocation must see the live input ticket: a sent input remains
        // uncertain even when its late result arrives after macOS withdraws permission.
        if let Some(refusal) = self.computer_revocation() {
            self.revoke_computer(&refusal).await;
            return Ok(());
        }
        if let Some(started_generation) = self
            .features
            .computer
            .work
            .expired_generation(ticket, self.state.generation())
        {
            let current_generation = self.state.generation();
            self.note_expired_work("computer_control", started_generation, current_generation);
            self.revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
                cause: orbit_core::refusal::ReadInterruption::GenerationChanged {
                    started_generation,
                    current_generation,
                },
            })
            .await;
            return Ok(());
        }
        // Identity and generation were checked above, with no await between
        // that check and settlement. Expiration was already retired there.
        if !matches!(
            self.features
                .computer
                .work
                .settle(ticket, self.state.generation()),
            Settlement::Current(())
        ) {
            return Ok(());
        }
        let completion = match event {
            Ok(completion) => completion,
            Err(error) => {
                self.computer_failed(&error).await;
                return Ok(());
            }
        };
        match completion.result {
            Ok(ComputerResult::Connected(data)) => {
                self.engine_answered(completion.service, data);
            }
            Ok(ComputerResult::Read(data)) => {
                let bridge_windows = self
                    .conversation
                    .bridge_request
                    .as_ref()
                    .is_some_and(|ask| {
                        ask.capability == orbit_core::device_bridge::Capability::ScreenSources
                    });
                self.engine_answered(completion.service, data.clone());
                if bridge_windows && !self.answer_bridge_from_computer_windows(&data).await? {
                    // The engine's list did not have the shape Deimos is
                    // answered with; the helper's is asked instead.
                    self.log("engine_answer_unused", &[("capability", "screen_sources")]);
                    self.source_windows().await?;
                } else if self
                    .conversation
                    .bridge_request
                    .as_ref()
                    .is_some_and(|ask| {
                        ask.capability == orbit_core::device_bridge::Capability::Screenshot
                    })
                {
                    if let Some((pid, window_id, application, title)) =
                        self.bridge_screenshot_target(&data)
                    {
                        self.read_bridge_screenshot(
                            Some(pid),
                            Some(window_id),
                            application,
                            title,
                        )?;
                    } else {
                        self.source_windows().await?;
                    }
                }
            }
            Ok(ComputerResult::BridgeScreenshot {
                observation,
                application,
                title,
            }) => {
                if completion.service.client().is_connected() {
                    self.features.computer.service = Some(completion.service);
                }
                if !self
                    .answer_bridge_from_computer_screenshot(&observation, &application, &title)
                    .await?
                {
                    self.log("engine_answer_unused", &[("capability", "screenshot")]);
                    self.source_windows().await?;
                }
            }
            Ok(ComputerResult::DesktopCapture { fitted }) => {
                if completion.service.client().is_connected() {
                    self.features.computer.service = Some(completion.service);
                }
                self.answer_bridge_with_desktop(fitted).await?;
            }
            Ok(ComputerResult::GeneralAction {
                operation_id,
                receipt,
            }) => {
                self.finish_remote_computer_action(&operation_id, &receipt);
                if let Some(mut history) = self.features.computer.active_general.take() {
                    history.outcome = ControlOutcome::Performed;
                    self.upsert_control_history(history)?;
                }
                if completion.service.client().is_connected() {
                    self.features.computer.service = Some(completion.service);
                }
                if let Some(view) = self.features.computer.view.as_mut() {
                    view.stage = ComputerStage::Completed;
                    view.request_id = operation_id;
                    view.observation = Some(receipt);
                    view.detail = "El motor devolvió un recibo del único intento; el efecto se comprueba con una observación nueva.".into();
                }
            }
            Err(error) => {
                let bridge_observation =
                    self.conversation
                        .bridge_request
                        .as_ref()
                        .is_some_and(|ask| {
                            matches!(
                                ask.capability,
                                orbit_core::device_bridge::Capability::ScreenSources
                                    | orbit_core::device_bridge::Capability::Screenshot
                            )
                        });
                // An answer too large for Orbit is the engine
                // answering, not the engine failing. It keeps its connection
                // and nothing says it is unhealthy.
                if bridge_observation
                    && let Some(too_large) = error
                        .downcast_ref::<crate::cua::ResponseTooLarge>()
                        .copied()
                {
                    if completion.service.client().is_connected() {
                        self.features.computer.service = Some(completion.service);
                    }
                    self.bridge_observation_too_large(too_large).await;
                    return Ok(());
                }
                // A full screen has nowhere else to come from: the helper
                // captures windows. It is refused by its cause, never
                // answered with some window.
                let full_screen = self
                    .conversation
                    .bridge_request
                    .as_ref()
                    .is_some_and(|ask| {
                        ask.capability == orbit_core::device_bridge::Capability::Screenshot
                            && ask.full_screen
                    });
                if full_screen {
                    if completion.service.client().is_connected() {
                        self.features.computer.service = Some(completion.service);
                    }
                    let refusal = match error.downcast_ref::<DesktopCaptureError>() {
                        Some(desktop) => {
                            self.log(
                                "desktop_capture_refused",
                                &[("cause", &desktop.to_string())],
                            );
                            desktop.refusal()
                        }
                        None => {
                            self.note_engine_failure(
                                orbit_core::refusal::EngineCause::Unhealthy,
                                &error,
                                &[("step", "full_screen")],
                            );
                            remote_engine_refusal("")
                        }
                    };
                    self.refuse_bridge_capture(refusal);
                    return Ok(());
                }
                if bridge_observation {
                    if completion.service.client().is_connected() {
                        self.features.computer.service = Some(completion.service);
                    }
                    if self.source_windows().await.is_ok() {
                        // The helper answers Deimos; the engine's failure is
                        // still a failure, and is said.
                        self.note_engine_failure(
                            orbit_core::refusal::EngineCause::Unhealthy,
                            &error,
                            &[("fallback", "helper")],
                        );
                        return Ok(());
                    }
                }
                self.computer_failed(&error).await;
            }
        }
        Ok(())
    }

    /// The engine answered: whatever failure it carried is over, the
    /// connection's facts are published and it is ready for the next work.
    /// Only that: what the answer was for is the caller's to route.
    fn engine_answered(&mut self, mut service: ActionService, data: Value) {
        self.features.computer.engine_failure = None;
        self.features.computer.engine_failure_kind = None;
        self.features.computer.engine_retry_at = None;
        self.features.computer.engine_retry_at_ms = None;
        self.features.computer.declared_tools =
            Some(u32::try_from(service.client().engine_tools().len()).unwrap_or(u32::MAX));
        let cleanups = service.client_mut().take_engine_cleanup();
        let inspection_notice = service.client_mut().take_engine_inspection_notice();
        if !cleanups.is_empty() {
            let mut lines = Vec::new();
            for c in cleanups {
                let reason_str = match c.reason {
                    orbit_core::topics::control::EngineCleanupReason::OrphanedFromDaemon => {
                        "huérfano de su daemon"
                    }
                };
                let outcome = match c.outcome {
                    orbit_core::topics::control::EngineCleanupOutcome::Stopped => {
                        "se confirmó detenido".to_owned()
                    }
                    orbit_core::topics::control::EngineCleanupOutcome::Survived => {
                        "sobrevivió a SIGKILL".to_owned()
                    }
                    orbit_core::topics::control::EngineCleanupOutcome::SignalFailed => {
                        format!(
                            "falló SIGKILL: {}",
                            c.failure.as_deref().unwrap_or("causa no comunicada")
                        )
                    }
                    orbit_core::topics::control::EngineCleanupOutcome::Unconfirmed => {
                        format!(
                            "no se pudo confirmar la parada: {}",
                            c.failure.as_deref().unwrap_or("causa no comunicada")
                        )
                    }
                };
                lines.push(format!(
                    "PID {} en {}: {}; {}",
                    c.pid, c.executable, reason_str, outcome
                ));
            }
            self.log("engine_sweep", &[("lines", &lines.join(" | "))]);
            self.say(Notice::new(NoticeCode::EngineSwept).with(NoticeKey::Count, lines.len()));
        } else if let Some(notice) = inspection_notice {
            self.log("engine_inspection", &[("notice", &notice)]);
            self.say(NoticeCode::EngineInspected);
        }
        if let Some(view) = self.features.computer.view.as_mut() {
            view.connected = true;
            view.connection_epoch = Some(service.client().connection_epoch().into());
            view.stage = ComputerStage::Ready;
            view.observation = Some(data);
            view.detail = "Observación recibida, no autorización de input. Los snapshots caducan en 15 segundos desde el inicio de lectura.".into();
        }
        self.features.computer.service = Some(service);
    }

    /// The engine's answer to what Deimos asked to see was too large for
    /// Orbit. A window is asked of the helper, which captures windows; a full
    /// screen it cannot capture, so that request is refused with the size and
    /// the ceiling instead of being answered with some window.
    async fn bridge_observation_too_large(&mut self, too_large: crate::cua::ResponseTooLarge) {
        let full_screen = self
            .conversation
            .bridge_request
            .as_ref()
            .is_some_and(|ask| ask.full_screen);
        let part = match too_large.part {
            crate::cua::TooLargePart::Message => "message",
            crate::cua::TooLargePart::Image => "image",
        };
        let bytes = too_large.bytes.to_string();
        let max_bytes = too_large.max_bytes.to_string();
        let fields = [
            ("part", part),
            ("bytes", bytes.as_str()),
            ("max_bytes", max_bytes.as_str()),
        ];
        if !full_screen && self.source_windows().await.is_ok() {
            self.log(
                "engine_answer_too_large",
                &[&fields[..], &[("step", "helper_fallback")]].concat(),
            );
            return;
        }
        self.log(
            "engine_answer_too_large",
            &[&fields[..], &[("step", "refused")]].concat(),
        );
        let refusal = orbit_core::refusal::Refusal::TooLarge {
            resource: orbit_core::refusal::Resource::Capture,
            bytes: u64::try_from(too_large.bytes).unwrap_or(u64::MAX),
            max_bytes: u64::try_from(too_large.max_bytes).unwrap_or(u64::MAX),
        };
        self.refuse_bridge_capture(refusal);
    }

    pub(super) async fn computer_failed(&mut self, error: &anyhow::Error) {
        let kind = if self
            .features
            .computer
            .view
            .as_ref()
            .is_some_and(|view| view.stage == ComputerStage::Connecting)
        {
            orbit_core::refusal::EngineCause::InitializationFailed
        } else {
            orbit_core::refusal::EngineCause::Unhealthy
        };
        let failed_history = self.features.computer.active_general.take();
        let operation_id = failed_history
            .as_ref()
            .map(|history| history.operation_id.clone());
        let tool = failed_history
            .as_ref()
            .map(|history| history.requested.clone())
            .unwrap_or_default();
        let request = self
            .conversation
            .bridge_request
            .as_ref()
            .map(|ask| ask.id.clone())
            .unwrap_or_default();
        let cause = format!("{error:#}");
        if let Some(mut history) = failed_history {
            history.outcome = ControlOutcome::Unknown;
            let _ = self.upsert_control_history(history);
        }
        // A remote request that was riding on this engine is answered with
        // the cause, here and now, instead of being left to time out or reappear
        // on a different path.
        // Said before anything is answered or revoked, so the line names
        // the request and the operation it cost.
        self.note_engine_failure(
            kind,
            error,
            &[
                ("operation", operation_id.as_deref().unwrap_or("")),
                ("tool", &tool),
                ("request", &request),
            ],
        );
        self.refuse_remote_engine_action(&cause, operation_id.as_deref())
            .await;
        self.revoke_computer(&orbit_core::refusal::Refusal::EngineUnavailable { cause: kind })
            .await;
        // The revocation clears what the next start may rely on; the cause
        // and the retry stay.
        self.engine_unavailable_state(kind, &cause);
        if let Some(view) = self.features.computer.view.as_mut() {
            view.stage = ComputerStage::Failed;
            view.detail = cause.chars().take(512).collect();
        }
    }

    /// Answers a remote engine action that cannot be completed, by name.
    async fn refuse_remote_engine_action(&mut self, _cause: &str, operation_id: Option<&str>) {
        let Some((ask, capability)) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::PerformAction
                    || ask.capability == orbit_core::device_bridge::Capability::ControlTools
            })
            .map(|ask| (ask.id.clone(), ask.capability))
        else {
            return;
        };
        self.conversation.bridge_request = None;
        let uncertain = capability == orbit_core::device_bridge::Capability::PerformAction
            && operation_id.is_some();
        let refusal = if let Some(operation_id) = operation_id.filter(|_| uncertain) {
            orbit_core::refusal::Refusal::OutcomeUnknown {
                operation_id: operation_id.to_owned(),
            }
        } else {
            remote_engine_refusal(_cause)
        };
        let notice = refusal.to_string();
        let result = self.send_frame(crate::hub::command(
            &ask,
            "device.reply",
            self.bridge_refusal_frame(&ask, &refusal),
        ));
        self.note_bridge_send(&result, &format!("the engine refusal ({notice})"));
    }

    pub(super) async fn maintain_computer(&mut self) -> Result<()> {
        self.poll_permissions();
        // The engine follows what macOS grants — started, kept or
        // stopped with its cause — every turn, with nobody asking.
        self.maintain_engine().await;
        Ok(())
    }
}

/// An engine receipt larger than the State carries
/// (`orbit_core::limits::MAX_ACTION_RECEIPT_BYTES`), typed so that its
/// numbers reach the `engine_failed` report line; its text is the bound's own.
#[derive(Debug)]
pub(super) struct ReceiptTooLarge {
    pub(super) bytes: usize,
    pub(super) max_bytes: usize,
}

impl std::fmt::Display for ReceiptTooLarge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Recibo de acción de {} bytes; máximo {}",
            self.bytes, self.max_bytes
        )
    }
}

impl std::error::Error for ReceiptTooLarge {}

/// Why `bounded_action_receipt` refused `receipt`: its size when that was the
/// reason, its own words otherwise.
fn receipt_refused(receipt: &impl Serialize, text: String) -> anyhow::Error {
    let max_bytes = orbit_core::limits::MAX_ACTION_RECEIPT_BYTES;
    match serde_json::to_vec(receipt) {
        Ok(bytes) if bytes.len() > max_bytes => ReceiptTooLarge {
            bytes: bytes.len(),
            max_bytes,
        }
        .into(),
        _ => anyhow::Error::msg(text),
    }
}

fn bounded_observation(value: &impl Serialize) -> Result<Value> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_OBSERVATION_BYTES {
        bail!(
            "Observación superior a 32 KiB; no se entrega truncada ni se habilita input sobre ella"
        );
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Why a full screen could not be delivered, with its numbers.
#[derive(Debug, thiserror::Error)]
pub(super) enum DesktopCaptureError {
    #[error("El motor respondió sin escribir la captura de pantalla que se le pidió")]
    Missing,
    #[error("No se pudo leer la captura de pantalla que escribió el motor: {0}")]
    Unreadable(String),
    #[error(transparent)]
    Fit(#[from] orbit_core::sources::DesktopFitError),
}

impl DesktopCaptureError {
    /// What Deimos is answered with.
    pub(super) fn refusal(&self) -> orbit_core::refusal::Refusal {
        use orbit_core::refusal::{Refusal, Resource};
        use orbit_core::sources::DesktopFitError;
        let too_large = |bytes: usize, max_bytes: usize| Refusal::TooLarge {
            resource: Resource::Capture,
            bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            max_bytes: u64::try_from(max_bytes).unwrap_or(u64::MAX),
        };
        match self {
            Self::Missing => Refusal::NotFound {
                resource: Resource::Capture,
            },
            Self::Fit(DesktopFitError::FileTooLarge { bytes, max_bytes })
            | Self::Fit(DesktopFitError::CannotFit {
                bytes, max_bytes, ..
            }) => too_large(*bytes, *max_bytes),
            Self::Unreadable(error) => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::DaemonOperation {
                    error: error.clone(),
                },
            },
            Self::Fit(cause) => Refusal::InvalidParameter {
                parameter: orbit_core::refusal::Parameter::Request,
                expected: orbit_core::refusal::Expected::DesktopFit {
                    cause: cause.clone(),
                },
            },
        }
    }
}

/// Asks the engine for the full screen in `path`, then takes the file back:
/// read whole under its ceiling and removed whatever happened, before
/// anything else is decided. The engine's own failure comes first; then the
/// file's; then the fit's.
async fn desktop_capture(
    service: &mut ActionService,
    path: PathBuf,
) -> Result<orbit_core::sources::FittedDesktop> {
    let target = path
        .to_str()
        .context("La ruta privada de la captura no es UTF-8")?
        .to_owned();
    let read = service
        .client_mut()
        .call_tool("get_desktop_state", json!({"screenshot_out_file": target}))
        .await;
    tokio::task::spawn_blocking(move || {
        let taken = take_desktop_file(&path);
        read?;
        let bytes = taken?;
        Ok(orbit_core::sources::fit_desktop_png(&bytes).map_err(DesktopCaptureError::from)?)
    })
    .await
    .context("La reducción de la pantalla terminó sin resultado")?
}

/// The engine's file, whole, if it is a regular file within
/// `MAX_DESKTOP_PNG_BYTES`; removed in every case.
fn take_desktop_file(path: &std::path::Path) -> std::result::Result<Vec<u8>, DesktopCaptureError> {
    let taken = read_desktop_file(path);
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DesktopCaptureError::Unreadable(format!(
                "no se pudo borrar después de leerla: {error}"
            )));
        }
    }
    taken
}

fn read_desktop_file(path: &std::path::Path) -> std::result::Result<Vec<u8>, DesktopCaptureError> {
    use std::io::Read as _;
    let max_bytes = orbit_core::sources::MAX_DESKTOP_PNG_BYTES;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DesktopCaptureError::Missing);
        }
        Err(error) => return Err(DesktopCaptureError::Unreadable(error.to_string())),
    };
    if !metadata.is_file() {
        return Err(DesktopCaptureError::Unreadable(
            "no es un fichero normal".into(),
        ));
    }
    let too_large = |bytes: u64| {
        DesktopCaptureError::Fit(orbit_core::sources::DesktopFitError::FileTooLarge {
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
            max_bytes,
        })
    };
    if metadata.len() > max_bytes as u64 {
        return Err(too_large(metadata.len()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(max_bytes as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| DesktopCaptureError::Unreadable(error.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(too_large(bytes.len() as u64));
    }
    Ok(bytes)
}

/// What a connection that came up delivers to the actor: the engine's own
/// facts, as a connection and never as a listing.
fn connected(service: ActionService) -> Result<ComputerCompletion> {
    let observation = json!({"driver_version": crate::cua::VERIFIED_DRIVER_VERSION});
    Ok(ComputerCompletion {
        service,
        result: Ok(ComputerResult::Connected(observation)),
    })
}

/// The application a receipt is filed under, taken from the engine's own
/// arguments. Orbit does not interpret the call beyond naming it in the record.
fn tool_application(call: &ToolCall) -> String {
    for key in ["bundle_id", "app", "application", "app_name"] {
        if let Some(value) = call.arguments.get(key).and_then(Value::as_str)
            && !value.is_empty()
            && value.len() <= 256
            && !value.chars().any(char::is_control)
        {
            return value.to_owned();
        }
    }
    call.arguments
        .get("pid")
        .and_then(Value::as_i64)
        .map_or_else(|| "Escritorio".to_owned(), |pid| format!("pid {pid}"))
}

#[cfg(test)]
mod tests;
