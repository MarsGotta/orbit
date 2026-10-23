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

mod authority;

mod history;

mod revocation;

mod connection;

mod screen;

mod tools;

mod results;

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
pub(super) struct ComputerHandle {
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
        /// «Adjuntar ventana»: the window's tree, read after its capture in
        /// a call of its own (the two together could outgrow one engine
        /// message), or the cause it could not be.
        text: Option<std::result::Result<String, String>>,
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
    /// The view and authority side of starting computer work: stage, detail
    /// and the revoked-by-default authority channel. No worker yet, so no
    /// ticket either; [`Self::begin_computer`] joins both halves.
    fn begin_computer_view(&mut self, stage: ComputerStage, detail: &str, generation: u64) {
        self.computer.pending_cancel_uncertain = false;
        let connected = !matches!(stage, ComputerStage::Connecting);
        let epoch = self
            .computer
            .view
            .as_ref()
            .and_then(|v| v.connection_epoch.clone());
        let mut authority = self.computer_authority();
        authority.generation = generation;
        self.computer.authority = Some(watch::channel(WorkerAuthority::active(authority)).0);
        self.computer.view = Some(ComputerControlView {
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
            .computer
            .work
            .start_logical(generation, self.turn.op.clone(), (), worker)
            .ok_or(orbit_core::refusal::Refusal::WorkInFlight {
                in_flight: 1,
                max: 1,
            })?;
        if let Some(view) = self.computer.view.as_mut() {
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

/// What a connection that came up delivers to the actor: the engine's own
/// facts, as a connection and never as a listing.
fn connected(service: ActionService) -> Result<ComputerCompletion> {
    let observation = json!({"driver_version": crate::cua::VERIFIED_DRIVER_VERSION});
    Ok(ComputerCompletion {
        service,
        result: Ok(ComputerResult::Connected(observation)),
    })
}

/// A computer worker's result goes back to the loop.
async fn deliver_computer(
    events: &tokio::sync::mpsc::Sender<WorkResult>,
    ticket: WorkTicket,
    event: ComputerCompletion,
) {
    crate::daemon::deliver_work(
        events,
        WorkResult::Control(crate::daemon::ControlWork::Computer {
            ticket,
            event: Box::new(Ok(event)),
        }),
    )
    .await;
}

impl ComputerHandle {
    /// A connection epoch is evidence of a live owner, never detector permission.
    pub(super) fn computer_connection_epoch(&self) -> Option<&str> {
        self.service
            .as_ref()
            .filter(|service| service.client().is_connected())
            .map(|service| service.client().connection_epoch())
            .or_else(|| {
                self.work.active().as_ref()?;
                self.view
                    .as_ref()
                    .filter(|view| view.connected)?
                    .connection_epoch
                    .as_deref()
            })
    }

    fn require_computer_idle(&self) -> Result<()> {
        if self.work.active().is_some() {
            bail!("Hay una operación de control en curso; no se repetirá el input");
        }
        Ok(())
    }

    /// The published side of an engine failure: its cause for the app and
    /// the remote caller, and when Orbit tries again.
    fn engine_unavailable_state(&mut self, kind: orbit_core::refusal::EngineCause, cause: &str) {
        self.engine_failure = Some(cause.chars().take(512).collect());
        self.engine_failure_kind = Some(kind);
        self.engine_retry_at = Some(Instant::now() + ENGINE_RETRY_DELAY);
        let delay_ms = i64::try_from(ENGINE_RETRY_DELAY.as_millis()).unwrap_or(i64::MAX);
        self.engine_retry_at_ms = Some(now().saturating_add(delay_ms));
    }

    pub(super) fn engine_is_ready(&self) -> bool {
        self.service
            .as_ref()
            .is_some_and(|service| service.client().is_connected())
    }

    /// Which of the two ways the engine failed, for the control topic's
    /// published cause: it never came up, or it dropped while running.
    pub(super) fn engine_failure_cause(&self) -> Option<orbit_core::refusal::EngineCause> {
        self.engine_failure_kind
    }

    /// The tool call in flight, as its history entry will record it, for the
    /// control topic's head. The browser-input path has no entry here: it
    /// publishes through the engine state and its own recorded outcome.
    pub(super) fn active_control_action(&self) -> Option<&ControlHistoryEntry> {
        self.active_general.as_ref()
    }

    /// Asked of the engine that is connected right now, so the answer cannot
    /// drift from what the engine will actually accept.
    pub(super) fn engine_tools(&self) -> Vec<orbit_core::computer_control::EngineTool> {
        self.service
            .as_ref()
            .filter(|service| service.client().is_connected())
            .map(|service| service.client().engine_tools())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
