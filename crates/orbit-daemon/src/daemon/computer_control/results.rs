//! The engine's results, routed: a connection, a read for Deimos's windows or
//! screenshot, the full screen, one tool's receipt, or a failure said by its
//! cause.
use super::*;

impl Runtime {
    pub(in crate::daemon) async fn computer_event(
        &mut self,
        ticket: &WorkTicket,
        event: Result<ComputerCompletion>,
    ) -> Result<()> {
        if !self.admit_computer_result(ticket).await {
            return Ok(());
        }
        let completion = match event {
            Ok(completion) => completion,
            Err(error) => {
                self.computer_failed(&error).await;
                return Ok(());
            }
        };
        let service = completion.service;
        match completion.result {
            Ok(ComputerResult::Connected(data)) => self.engine_answered(service, data),
            Ok(ComputerResult::Read(data)) => self.on_engine_read(service, data).await?,
            Ok(ComputerResult::BridgeScreenshot {
                observation,
                application,
                title,
                text,
            }) => {
                self.computer.keep_connected_service(service);
                if let Some(text) = text {
                    self.assistant.window_read_by_engine(text);
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
                self.computer.keep_connected_service(service);
                self.answer_bridge_with_desktop(fitted).await?;
            }
            Ok(ComputerResult::GeneralAction {
                operation_id,
                receipt,
            }) => self.on_general_action(service, operation_id, receipt)?,
            Err(error) => self.on_engine_error(service, error).await,
        }
        Ok(())
    }

    /// Whether a worker's result still counts: its own ticket, no
    /// revocation, the same generation, and settled now. Revocation must
    /// see the live input ticket: a sent input remains uncertain even when
    /// its late result arrives after macOS withdraws permission.
    pub(super) async fn admit_computer_result(&mut self, ticket: &WorkTicket) -> bool {
        if self.computer.work.ticket() != Some(ticket) {
            return false;
        }
        if let Some(refusal) = self.computer_revocation() {
            self.revoke_computer(&refusal).await;
            return false;
        }
        if let Some(started_generation) = self
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
            return false;
        }
        // Identity and generation were checked above, with no await between
        // that check and settlement. Expiration was already retired there.
        matches!(
            self.computer.work.settle(ticket, self.state.generation()),
            Settlement::Current(())
        )
    }

    /// The engine read the screen: Deimos's windows or screenshot come from
    /// it, or from the helper when the engine's answer does not fit.
    pub(super) async fn on_engine_read(
        &mut self,
        service: ActionService,
        data: Value,
    ) -> Result<()> {
        let bridge_windows = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Screen)
            .is_some_and(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::ScreenSources
            });
        self.engine_answered(service, data.clone());
        if bridge_windows && !self.answer_bridge_from_computer_windows(&data).await? {
            // The engine's list did not have the shape Deimos is
            // answered with; the helper's is asked instead.
            self.log("engine_answer_unused", &[("capability", "screen_sources")]);
            self.source_windows().await?;
        } else if self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Screen)
            .is_some_and(|ask| ask.capability == orbit_core::device_bridge::Capability::Screenshot)
        {
            if let Some((pid, window_id, application, title)) =
                self.assistant.bridge_screenshot_target(&data)
            {
                self.read_bridge_screenshot(Some(pid), Some(window_id), application, title)?;
            } else {
                self.source_windows().await?;
            }
        }
        Ok(())
    }

    /// The engine's receipt of the one attempt: the history says it was
    /// performed, and the effect is checked with a new observation.
    pub(super) fn on_general_action(
        &mut self,
        service: ActionService,
        operation_id: String,
        receipt: Value,
    ) -> Result<()> {
        self.finish_remote_computer_action(&operation_id, &receipt);
        if let Some(mut history) = self.computer.active_general.take() {
            history.outcome = ControlOutcome::Performed;
            self.upsert_control_history(history)?;
        }
        self.computer.keep_connected_service(service);
        if let Some(view) = self.computer.view.as_mut() {
            view.stage = ComputerStage::Completed;
            view.request_id = operation_id;
            view.observation = Some(receipt);
            view.detail = "El motor devolvió un recibo del único intento; el efecto se comprueba con una observación nueva.".into();
        }
        Ok(())
    }

    /// The engine's work failed. An answer too large for Orbit is the
    /// engine answering, not failing; a full screen is refused by its
    /// cause; a window read falls back on the helper; anything else is the
    /// engine's failure.
    pub(super) async fn on_engine_error(&mut self, service: ActionService, error: anyhow::Error) {
        // Only a screen read the engine failed is the screen request's: the
        // engine also serves the computer's tools and actions, whose own
        // request has a slot of its own.
        let reading = self.engine_was_reading();
        let screen = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Screen)
            .filter(|_| reading);
        let bridge_observation = screen.is_some_and(|ask| {
            matches!(
                ask.capability,
                orbit_core::device_bridge::Capability::ScreenSources
                    | orbit_core::device_bridge::Capability::Screenshot
            )
        });
        // It keeps its connection and nothing says it is unhealthy.
        if bridge_observation
            && let Some(too_large) = error
                .downcast_ref::<crate::cua::ResponseTooLarge>()
                .copied()
        {
            self.computer.keep_connected_service(service);
            self.bridge_observation_too_large(too_large).await;
            return;
        }
        // A full screen has nowhere else to come from: the helper
        // captures windows. It is refused by its cause, never
        // answered with some window.
        let full_screen = screen.is_some_and(|ask| {
            ask.capability == orbit_core::device_bridge::Capability::Screenshot && ask.full_screen
        });
        if full_screen {
            self.computer.keep_connected_service(service);
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
            return;
        }
        if bridge_observation {
            self.computer.keep_connected_service(service);
            if self.source_windows().await.is_ok() {
                // The helper answers Deimos; the engine's failure is
                // still a failure, and is said.
                self.note_engine_failure(
                    orbit_core::refusal::EngineCause::Unhealthy,
                    &error,
                    &[("fallback", "helper")],
                );
                return;
            }
        }
        self.computer_failed(&error).await;
    }

    /// The engine answered: whatever failure it carried is over, the
    /// connection's facts are published and it is ready for the next work.
    /// Only that: what the answer was for is the caller's to route.
    pub(super) fn engine_answered(&mut self, mut service: ActionService, data: Value) {
        self.computer.engine_failure = None;
        self.computer.engine_failure_kind = None;
        self.computer.engine_retry_at = None;
        self.computer.engine_retry_at_ms = None;
        self.computer.declared_tools =
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
        if let Some(view) = self.computer.view.as_mut() {
            view.connected = true;
            view.connection_epoch = Some(service.client().connection_epoch().into());
            view.stage = ComputerStage::Ready;
            view.observation = Some(data);
            view.detail = "Observación recibida, no autorización de input. Los snapshots caducan en 15 segundos desde el inicio de lectura.".into();
        }
        self.computer.service = Some(service);
    }

    /// The engine's answer to what Deimos asked to see was too large for
    /// Orbit. A window is asked of the helper, which captures windows; a full
    /// screen it cannot capture, so that request is refused with the size and
    /// the ceiling instead of being answered with some window.
    pub(super) async fn bridge_observation_too_large(
        &mut self,
        too_large: crate::cua::ResponseTooLarge,
    ) {
        let full_screen = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Screen)
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

    /// The engine's work in course was a read of the screen, not one of the
    /// computer's tools or actions.
    fn engine_was_reading(&self) -> bool {
        self.computer
            .view
            .as_ref()
            .is_some_and(|view| view.stage == ComputerStage::Reading)
    }

    pub(in crate::daemon) async fn computer_failed(&mut self, error: &anyhow::Error) {
        let kind = if self
            .computer
            .view
            .as_ref()
            .is_some_and(|view| view.stage == ComputerStage::Connecting)
        {
            orbit_core::refusal::EngineCause::InitializationFailed
        } else {
            orbit_core::refusal::EngineCause::Unhealthy
        };
        let failed_history = self.computer.active_general.take();
        let operation_id = failed_history
            .as_ref()
            .map(|history| history.operation_id.clone());
        let tool = failed_history
            .as_ref()
            .map(|history| history.requested.clone())
            .unwrap_or_default();
        let request = self
            .assistant
            .bridge_ask(if self.engine_was_reading() {
                crate::daemon::BridgeKind::Screen
            } else {
                crate::daemon::BridgeKind::Control
            })
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
        self.computer.engine_unavailable_state(kind, &cause);
        if let Some(view) = self.computer.view.as_mut() {
            view.stage = ComputerStage::Failed;
            view.detail = cause.chars().take(512).collect();
        }
    }

    /// Answers a remote engine action that cannot be completed, by name.
    pub(super) async fn refuse_remote_engine_action(
        &mut self,
        _cause: &str,
        operation_id: Option<&str>,
    ) {
        let Some((ask, capability)) = self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Control)
            .filter(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::PerformAction
                    || ask.capability == orbit_core::device_bridge::Capability::ControlTools
            })
            .map(|ask| (ask.id.clone(), ask.capability))
        else {
            return;
        };
        self.assistant
            .end_bridge_ask(crate::daemon::BridgeKind::Control);
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
}

impl crate::daemon::computer_control::ComputerHandle {
    /// A connection that is still up is kept for the next work.
    pub(super) fn keep_connected_service(&mut self, service: ActionService) {
        if service.client().is_connected() {
            self.service = Some(service);
        }
    }
}
