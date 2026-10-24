//! Single-use native reads and validated in-memory receipts. No storage effects.
use super::{
    BridgeReleaseReason, Runtime, id, now,
    ticketed::{Deadline, DeadlineKind, Settlement, Ticketed, WorkTicket},
};
use anyhow::{Context, Result, bail};
use orbit_core::refusal::{Operation as RefusedOperation, ReadInterruption, Refusal, Resource};
use orbit_core::sources::{
    SourceFormat, SourceIntent, SourceProgress, SourceReceipt, SourceStage, WindowInventory,
};
use orbit_core::wire::native::{NativeResponseFields, NativeResponseIdRef, decode_result_body};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// Protects one listing or screenshot request from a lost helper reply. The
/// existing 10 s are not yet measured on macOS; one native request bounds
/// the wait. This is neither an inventory-size limit nor a retry interval.
const NATIVE_TIMEOUT: DeadlineKind = DeadlineKind::new(
    "source_native",
    Duration::from_secs(10),
    "One listing or screenshot helper reply; unmeasured on macOS, bounded to one call; not data or retry",
);
/// Protects one selected-text request from a lost helper reply. The existing
/// 5 s are not yet measured on macOS; one accessibility read bounds the wait.
/// This is neither a text-size limit nor a retry interval.
const TEXT_TIMEOUT: DeadlineKind = DeadlineKind::new(
    "selected_text",
    Duration::from_secs(5),
    "One accessibility text reply; unmeasured on macOS, bounded to one call; not data or retry",
);
mod content;
mod replies;

#[derive(Debug, thiserror::Error)]
pub(super) enum SourceStartError {
    #[error("source_start: {self:?}")]
    PrivacyStopped,
    #[error("source_start: {self:?}")]
    HelperNotRunning,
    #[error("source_start: {self:?}")]
    Busy,
    #[error("source_start: {self:?}")]
    NativeCommand(#[source] anyhow::Error),
    #[error("source_start: {self:?}")]
    NativeChannelBusy,
    #[error("source_start: {self:?}")]
    NativeChannelClosed,
}

#[derive(Default)]
pub(super) struct SourcesHandle {
    inventory_deadline: Option<Instant>,
    operation: Option<Operation>,
    reply: Ticketed<()>,
    pub(super) last_failure: Option<SourceFailure>,
    /// The capture each kind of remote request is uploading: a screenshot
    /// and a folder image can go up at once, each replacing only its own.
    pub(super) bridge_uploads: crate::daemon::BridgeSlots<BridgeUpload>,
}

#[derive(Clone)]
pub(super) struct SourceFailure {
    pub(super) operation_id: String,
    pub(super) refusal: Refusal,
}

pub(super) struct BridgeUpload {
    pub(super) capture_id: String,
    pub(super) screenshot: Option<orbit_core::wire::bridge::ScreenshotResult>,
    pub(super) work: Ticketed<()>,
}

#[derive(Clone)]
struct Operation {
    id: String,
    command_id: String,
    ticket: WorkTicket,
    session_id: Option<String>,
    host_identity: Option<super::native_host::HostIdentity>,
    deadline: Deadline,
    intent: Option<SourceIntent>,
}

fn source_timeout_refusal(deadline: Deadline) -> Refusal {
    Refusal::Timeout {
        operation: RefusedOperation::Capture,
        waited_ms: deadline.waited_ms(),
        deadline_ms: deadline.budget_ms(),
    }
}

fn source_contract_refusal(error: anyhow::Error, field: &str) -> Refusal {
    for cause in error.chain() {
        if let Some(refusal) = cause.downcast_ref::<Refusal>() {
            return refusal.clone();
        }
        if let Some(problem) = cause.downcast_ref::<orbit_core::wire::native::NativeDecodeProblem>()
        {
            return Refusal::helper_reply_invalid(problem.clone());
        }
    }
    Refusal::HelperReplyInvalid {
        field: field.into(),
        expected: "source_contract".into(),
        received_type: error.to_string(),
    }
}

impl Runtime {
    fn check_source_admission(&self) -> std::result::Result<(), SourceStartError> {
        if self.state.privacy_stopped() {
            return Err(SourceStartError::PrivacyStopped);
        }
        if !self.state.capabilities().native_helper() {
            return Err(SourceStartError::HelperNotRunning);
        }
        if self.sources.operation.is_some() || self.sources.reply.active().is_some() {
            return Err(SourceStartError::Busy);
        }
        Ok(())
    }

    pub(super) async fn source_windows(&mut self) -> std::result::Result<(), SourceStartError> {
        self.check_source_admission()?;
        let operation_id = id();
        let command_id = format!("source-list-{operation_id}");
        let command = crate::native::command("screen_sources", json!({"id":command_id}))
            .map_err(SourceStartError::NativeCommand)?;
        self.state.views.source_inventory = None;
        self.state.views.source_receipt = None;
        self.sources.last_failure = None;
        self.sources.inventory_deadline = None;
        self.source_progress(
            &operation_id,
            SourceStage::Listing,
            "Consultando ventanas autorizadas del Mac…",
        );
        if let Err(error) = self.helper.try_send(command) {
            self.source_progress(
                &operation_id,
                SourceStage::Failed,
                "El Mac está ocupado; no se enumeraron ventanas.",
            );
            return Err(match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    SourceStartError::NativeChannelBusy
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    SourceStartError::NativeChannelClosed
                }
            });
        }
        let ticket = self
            .sources
            .reply
            .start_external(self.state.generation(), self.turn.op.clone(), ())
            .ok_or(SourceStartError::Busy)?;
        self.sources.operation = Some(Operation {
            id: operation_id,
            command_id,
            ticket,
            session_id: self.state.session_id().clone(),
            host_identity: self.helper.identity().cloned(),
            deadline: NATIVE_TIMEOUT.start(),
            intent: None,
        });
        Ok(())
    }

    pub(super) async fn source_capture_as(
        &mut self,
        token: &str,
        annotation: String,
        format: SourceFormat,
    ) -> Result<()> {
        self.check_source_admission()?;
        let target = self.selected_source_target(token, format)?;
        let (native_command, timeout, detail) = capture_plan(format);
        let requested_at_ms = now();
        let host_identity = self.helper.identity().cloned();
        let intent = SourceIntent {
            id: id(),
            format,
            device_id: self.settings.device_id.clone(),
            session_id: self.state.session_id().clone(),
            generation: self.state.generation(),
            requested_at_ms,
            expires_at_ms: requested_at_ms.saturating_add(timeout.duration().as_millis() as i64),
            target,
            annotation,
        };
        intent.validate()?;
        // Retire the entire local inventory before dispatch, not after a reply.
        self.state.views.source_inventory = None;
        self.sources.inventory_deadline = None;
        self.state.views.source_receipt = None;
        self.source_progress(&intent.id, SourceStage::Capturing, detail);
        let command_id = format!("source-capture-{}", intent.id);
        let result = crate::native::command(
            native_command,
            json!({"id":command_id,"window_binding":intent.target.binding()}),
        )
        .and_then(|command| {
            self.helper
                .try_send(command)
                .context("El Mac está ocupado; captura no enviada")
        });
        if let Err(error) = result {
            self.source_progress(
                &intent.id,
                SourceStage::Failed,
                "La captura no se envió. Vuelve a seleccionar si quieres otro intento.",
            );
            return Err(error);
        }
        let ticket = self
            .sources
            .reply
            .start_external(self.state.generation(), self.turn.op.clone(), ())
            .context("La espera del helper ya tiene una operación")?;
        self.sources.operation = Some(Operation {
            id: intent.id.clone(),
            command_id,
            ticket,
            session_id: self.state.session_id().clone(),
            host_identity,
            deadline: timeout.start(),
            intent: Some(intent),
        });
        Ok(())
    }

    /// The window the user chose from a listing still in force.
    fn selected_source_target(
        &mut self,
        token: &str,
        format: SourceFormat,
    ) -> Result<orbit_core::sources::SourceTarget> {
        self.sources.last_failure = None;
        if self
            .sources
            .inventory_deadline
            .is_none_or(|deadline| Instant::now() >= deadline)
        {
            self.state.views.source_inventory = None;
            bail!("La selección caducó; vuelve a listar y elige una ventana");
        }
        let inventory = self
            .state
            .views
            .source_inventory
            .as_ref()
            .context("Primero lista y elige una ventana")?;
        if format == SourceFormat::SelectedText && !inventory.selected_text_available {
            bail!(
                "Texto seleccionado no disponible: revisa Accesibilidad localmente y vuelve a listar. No se usará imagen ni portapapeles como alternativa."
            );
        }
        Ok(inventory.select(token)?)
    }

    fn source_progress(&mut self, operation_id: &str, stage: SourceStage, detail: &str) {
        self.state.views.source_request = Some(SourceProgress {
            id: operation_id.into(),
            stage: stage.clone(),
            detail: detail.into(),
        });
        if matches!(stage, SourceStage::Failed | SourceStage::Unknown)
            && self.assistant.source_is_remote(operation_id)
        {
            let code = self
                .sources
                .last_failure
                .as_ref()
                .map_or("not_reported", |failure| failure.refusal.code().name());
            self.record(
                orbit_core::notice::Notice::new(orbit_core::notice::NoticeCode::BridgeRefused)
                    .with_token(orbit_core::notice::NoticeKey::Code, code),
            );
            return;
        }
        // The stage is the notice; the detail stays in the source's view.
        let code = match stage {
            SourceStage::Failed | SourceStage::Unknown => {
                orbit_core::notice::NoticeCode::SourceFailed
            }
            SourceStage::Listing
            | SourceStage::Available
            | SourceStage::Capturing
            | SourceStage::Validated
            | SourceStage::Cancelled => orbit_core::notice::NoticeCode::SourceProgress,
        };
        let stage = serde_json::to_value(&stage).unwrap_or_default();
        self.say(orbit_core::notice::Notice::new(code).with_token(
            orbit_core::notice::NoticeKey::Stage,
            stage.as_str().unwrap_or("unknown"),
        ));
    }

    async fn close_source_operation(
        &mut self,
        operation: &Operation,
        stage: SourceStage,
        refusal: Refusal,
    ) -> Result<()> {
        let detail = refusal.to_string();
        self.sources.last_failure = Some(SourceFailure {
            operation_id: operation.id.clone(),
            refusal,
        });
        self.source_progress(&operation.id, stage, &detail);
        Ok(())
    }

    pub(super) async fn cancel_sources(&mut self) -> Result<()> {
        self.cancel_sources_with_cause(
            "cancelled_no_replay",
            "La lectura se canceló localmente; no se repetirá la captura.",
        )
        .await
    }

    pub(super) async fn cancel_sources_with_cause(
        &mut self,
        cause: &str,
        _explanation: &str,
    ) -> Result<()> {
        self.state.views.source_inventory = None;
        self.sources.inventory_deadline = None;
        let mut close_result = Ok(());
        if let Some(operation) = self.sources.operation.as_ref().cloned() {
            self.sources.reply.cancel();
            let refusal = if self.state.privacy_stopped() {
                Refusal::PrivacyStopped { since_ms: None }
            } else if cause == "helper_restarted" {
                Refusal::ReadInterrupted {
                    cause: ReadInterruption::HelperRestarted {
                        previous_generation: operation
                            .host_identity
                            .as_ref()
                            .map(|host| host.generation),
                        previous_instance: operation
                            .host_identity
                            .as_ref()
                            .map(|host| host.id.clone()),
                        current_instance: self.helper.identity().map(|host| host.id.clone()),
                        current_generation: self.helper.identity().map(|host| host.generation),
                    },
                }
            } else if cause == "generation_changed" {
                Refusal::ReadInterrupted {
                    cause: ReadInterruption::GenerationChanged {
                        started_generation: operation.ticket.generation(),
                        current_generation: self.state.generation(),
                    },
                }
            } else if cause == "native_timeout" {
                source_timeout_refusal(operation.deadline)
            } else {
                Self::bridge_refusal_code(cause)
            };
            close_result = self
                .close_source_operation(&operation, SourceStage::Unknown, refusal)
                .await;
            self.sources.operation = None;
        } else if let Some(progress) = &self.state.views.source_request
            && progress.stage == SourceStage::Available
        {
            let operation_id = progress.id.clone();
            self.source_progress(
                &operation_id,
                SourceStage::Cancelled,
                "Selección retirada; vuelve a listar explícitamente para elegir otra ventana.",
            );
        }
        self.cancel_sources_volatile();
        close_result
    }

    pub(super) fn cancel_sources_volatile(&mut self) {
        self.state.views.source_inventory = None;
        self.sources.inventory_deadline = None;
        // Release the remote requests, and their uploads, with the source's
        // recorded cause, if one closed, or the explicit cancellation cause
        // otherwise: every kind, as when there was one slot for all.
        self.release_bridge_requests(BridgeReleaseReason::Cancelled);
    }

    pub(super) async fn check_source_timeout(&mut self) -> Result<()> {
        if self
            .sources
            .inventory_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.sources.inventory_deadline = None;
            self.state.views.source_inventory = None;
            if let Some(progress) = &self.state.views.source_request
                && progress.stage == SourceStage::Available
            {
                let operation_id = progress.id.clone();
                self.source_progress(
                    &operation_id,
                    SourceStage::Cancelled,
                    "El inventario caducó; vuelve a listar y elegir una ventana.",
                );
            }
        }
        if self
            .sources
            .operation
            .as_ref()
            .is_some_and(|operation| operation.deadline.expired())
        {
            let operation = self.sources.operation.take().context("Operación ausente")?;
            self.sources.reply.cancel();
            self.close_source_operation(
                &operation,
                SourceStage::Unknown,
                source_timeout_refusal(operation.deadline),
            )
            .await?;
            // The bridge slot is freed here rather than from the helper's
            // answer: a timeout is one of the endings the spec lists, and
            // the caller should hear `timeout` rather than wait for a reply
            // that will never come.
            self.release_bridge_request(
                crate::daemon::BridgeKind::Screen,
                BridgeReleaseReason::Timeout {
                    waited_ms: operation.deadline.waited_ms(),
                    deadline_ms: operation.deadline.budget_ms(),
                },
            );
        }
        Ok(())
    }
}

/// The helper's order for a capture format, its deadline and what the
/// person is told meanwhile.
fn capture_plan(format: SourceFormat) -> (&'static str, DeadlineKind, &'static str) {
    match format {
        SourceFormat::Png => (
            "screenshot",
            NATIVE_TIMEOUT,
            "Capturando la ventana elegida una sola vez…",
        ),
        SourceFormat::SelectedText => (
            "selected_text",
            TEXT_TIMEOUT,
            "Leyendo una selección verificable de la ventana elegida, sin portapapeles…",
        ),
    }
}

impl crate::daemon::assistant::AssistantHandle {
    fn source_is_remote(&self, operation_id: &str) -> bool {
        self.bridge_requests
            .get(crate::daemon::BridgeKind::Screen)
            .is_some_and(|ask| {
                ask.source_id
                    .as_deref()
                    .is_none_or(|source| source == operation_id)
            })
    }
}

impl SourcesHandle {
    /// True while a window listing or capture is still waiting on the helper.
    ///
    /// Exposed for the remote bridge, which has to tell "the answer has not
    /// arrived yet" from "the operation ended without one": the second is a
    /// failure it must report rather than keep waiting on.
    pub(super) fn source_operation_pending(&self) -> bool {
        self.operation.is_some()
    }
}

#[cfg(test)]
mod tests;
