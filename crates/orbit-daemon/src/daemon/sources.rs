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
pub(super) struct SourceRuntime {
    inventory_deadline: Option<Instant>,
    operation: Option<Operation>,
    reply: Ticketed<()>,
    pub(super) last_failure: Option<SourceFailure>,
    pub(super) bridge_upload: Option<BridgeUpload>,
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
    /// True while a window listing or capture is still waiting on the helper.
    ///
    /// Exposed for the remote bridge, which has to tell "the answer has not
    /// arrived yet" from "the operation ended without one": the second is a
    /// failure it must report rather than keep waiting on.
    pub(super) fn source_operation_pending(&self) -> bool {
        self.features.sources.operation.is_some()
    }

    fn check_source_admission(&self) -> std::result::Result<(), SourceStartError> {
        if self.state.privacy_stopped() {
            return Err(SourceStartError::PrivacyStopped);
        }
        if !self.state.capabilities().native_helper() {
            return Err(SourceStartError::HelperNotRunning);
        }
        if self.features.sources.operation.is_some()
            || self.features.sources.reply.active().is_some()
        {
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
        self.features.sources.last_failure = None;
        self.features.sources.inventory_deadline = None;
        self.source_progress(
            &operation_id,
            SourceStage::Listing,
            "Consultando ventanas autorizadas del Mac…",
        );
        if let Err(error) = self.native.try_send(command) {
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
            .features
            .sources
            .reply
            .start_external(self.state.generation(), self.turn.op.clone(), ())
            .ok_or(SourceStartError::Busy)?;
        self.features.sources.operation = Some(Operation {
            id: operation_id,
            command_id,
            ticket,
            session_id: self.state.session_id().clone(),
            host_identity: self.features.native_host.identity().cloned(),
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
        self.features.sources.last_failure = None;
        if self
            .features
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
        let target = inventory.select(token)?;
        let (native_command, timeout, detail) = match format {
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
        };
        let requested_at_ms = now();
        let host_identity = self.features.native_host.identity().cloned();
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
        self.features.sources.inventory_deadline = None;
        self.state.views.source_receipt = None;
        self.source_progress(&intent.id, SourceStage::Capturing, detail);
        let command_id = format!("source-capture-{}", intent.id);
        let result = crate::native::command(
            native_command,
            json!({"id":command_id,"window_binding":intent.target.binding()}),
        )
        .and_then(|command| {
            self.native
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
            .features
            .sources
            .reply
            .start_external(self.state.generation(), self.turn.op.clone(), ())
            .context("La espera del helper ya tiene una operación")?;
        self.features.sources.operation = Some(Operation {
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

    pub(super) async fn source_response(&mut self, event: &Value) -> Result<bool> {
        let Some(command_id) = NativeResponseIdRef::from_value(event)
            .ok()
            .map(|response| response.id)
            .filter(|id| id.starts_with("source-list-") || id.starts_with("source-capture-"))
        else {
            return Ok(false);
        };
        if !self
            .features
            .sources
            .operation
            .as_ref()
            .is_some_and(|op| op.command_id == command_id)
        {
            return Ok(true); // Late or duplicate ACK cannot stop unrelated audio or capture again.
        }
        let operation = self
            .features
            .sources
            .operation
            .take()
            .context("Operación de fuente ausente")?;
        let settlement = self
            .features
            .sources
            .reply
            .settle(&operation.ticket, self.state.generation());
        let stale = if self.state.privacy_stopped() {
            Some(Refusal::PrivacyStopped { since_ms: None })
        } else if let Settlement::Expired {
            started_generation,
            current_generation,
        } = &settlement
        {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::GenerationChanged {
                    started_generation: *started_generation,
                    current_generation: *current_generation,
                },
            })
        } else if operation.session_id != self.state.session_id() {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::ConversationChanged,
            })
        } else if operation.deadline.expired() {
            Some(source_timeout_refusal(operation.deadline))
        } else if matches!(settlement, Settlement::Foreign) {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::SourceReplyReplaced,
            })
        } else {
            None
        };
        if let Some(refusal) = stale {
            self.close_source_operation(&operation, SourceStage::Unknown, refusal)
                .await?;
            return Ok(true);
        }
        let response = NativeResponseFields::from_value(event)?;
        if !response.ok {
            // Keep the closed helper code and its facts, never its untrusted message.
            let refusal = match response.error.as_ref() {
                Some(error) => Runtime::bridge_refusal_code(&error.code),
                None => Refusal::HelperReplyInvalid {
                    field: "error".into(),
                    expected: "native_error".into(),
                    received_type: "absent".into(),
                },
            };
            self.close_source_operation(&operation, SourceStage::Failed, refusal)
                .await?;
            return Ok(true);
        }
        let body = response
            .result
            .as_ref()
            .context("response source field=result type=absent")?;
        let result = if let Some(intent) = &operation.intent {
            self.accept_source_content(intent, body)
        } else {
            self.accept_source_inventory(&operation, body)
        };
        if let Err(error) = result {
            self.close_source_operation(
                &operation,
                SourceStage::Failed,
                source_contract_refusal(error, "source.result"),
            )
            .await?;
        }
        Ok(true)
    }

    fn accept_source_inventory(&mut self, operation: &Operation, result: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(result)?.len();
        if bytes > orbit_core::limits::MAX_WINDOW_INVENTORY_BYTES {
            return Err(Refusal::TooLarge {
                resource: Resource::Source,
                bytes: bytes as u64,
                max_bytes: orbit_core::limits::MAX_WINDOW_INVENTORY_BYTES as u64,
            }
            .into());
        }
        let inventory: WindowInventory =
            decode_result_body("screen_sources", result).map_err(Refusal::helper_reply_invalid)?;
        inventory.validate()?;
        // Conservative: the timer begins when the request was sent, not on receipt.
        let request_started = operation.deadline.started();
        let ttl =
            Duration::from_nanos(inventory.expires_monotonic_ns - inventory.observed_monotonic_ns);
        let deadline = request_started + ttl;
        if Instant::now() >= deadline {
            return Err(source_timeout_refusal(operation.deadline).into());
        }
        self.features.sources.inventory_deadline = Some(deadline);
        self.state.views.source_inventory = Some(inventory);
        self.source_progress(
            &operation.id,
            SourceStage::Available,
            "Elige una ventana y el formato para leerla una sola vez.",
        );
        Ok(())
    }

    fn accept_source_content(&mut self, intent: &SourceIntent, result: &Value) -> Result<()> {
        let (metadata, bytes) = content::decode(intent, result, now())?;
        let receipt = SourceReceipt {
            intent: intent.clone(),
            metadata,
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        receipt.validate()?;
        self.state.views.source_receipt = Some(receipt);
        self.source_progress(
            &intent.id,
            SourceStage::Validated,
            "Lectura validada en memoria; no se conserva en este Mac.",
        );
        Ok(())
    }

    pub(super) fn remote_source_response(&self, command_id: &str) -> bool {
        self.features
            .sources
            .operation
            .as_ref()
            .is_some_and(|operation| {
                operation.command_id == command_id && self.source_is_remote(&operation.id)
            })
    }

    fn source_is_remote(&self, operation_id: &str) -> bool {
        self.conversation
            .bridge_request
            .as_ref()
            .is_some_and(|ask| {
                matches!(
                    ask.capability,
                    orbit_core::device_bridge::Capability::ScreenSources
                        | orbit_core::device_bridge::Capability::Screenshot
                        | orbit_core::device_bridge::Capability::SelectedText
                ) && ask
                    .source_id
                    .as_deref()
                    .is_none_or(|source| source == operation_id)
            })
    }

    fn source_progress(&mut self, operation_id: &str, stage: SourceStage, detail: &str) {
        self.state.views.source_request = Some(SourceProgress {
            id: operation_id.into(),
            stage: stage.clone(),
            detail: detail.into(),
        });
        if matches!(stage, SourceStage::Failed | SourceStage::Unknown)
            && self.source_is_remote(operation_id)
        {
            let code = self
                .features
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
        self.features.sources.last_failure = Some(SourceFailure {
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
        self.features.sources.inventory_deadline = None;
        let mut close_result = Ok(());
        if let Some(operation) = self.features.sources.operation.as_ref().cloned() {
            self.features.sources.reply.cancel();
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
                        current_instance: self
                            .features
                            .native_host
                            .identity()
                            .map(|host| host.id.clone()),
                        current_generation: self
                            .features
                            .native_host
                            .identity()
                            .map(|host| host.generation),
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
            self.features.sources.operation = None;
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
        if let Some(mut upload) = self.features.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        self.state.views.source_inventory = None;
        self.features.sources.inventory_deadline = None;
        // Release the remote slot with the source's recorded cause, if one
        // closed, or the explicit cancellation cause otherwise.
        self.release_bridge_request(BridgeReleaseReason::Cancelled);
    }

    pub(super) async fn check_source_timeout(&mut self) -> Result<()> {
        if self
            .features
            .sources
            .inventory_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.features.sources.inventory_deadline = None;
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
            .features
            .sources
            .operation
            .as_ref()
            .is_some_and(|operation| operation.deadline.expired())
        {
            let operation = self
                .features
                .sources
                .operation
                .take()
                .context("Operación ausente")?;
            self.features.sources.reply.cancel();
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
            self.release_bridge_request(BridgeReleaseReason::Timeout {
                waited_ms: operation.deadline.waited_ms(),
                deadline_ms: operation.deadline.budget_ms(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
