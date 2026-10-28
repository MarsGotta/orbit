//! The engine's connection: started and kept up, retried after a failure,
//! its status and the system permissions it is polled for.
use super::*;

impl Runtime {
    /// The engine's whole lifecycle, decided by `engine::decide` from
    /// what macOS grants and the privacy stop. It starts with both
    /// permissions and no order, switch, pairing or hub; it stops, naming the
    /// cause, when the privacy stop is latched or macOS withdraws one; and a
    /// permission that is not known yet neither starts it nor stops it.
    pub(in crate::daemon) async fn maintain_engine(&mut self) {
        if !self.closing
            && let Some(service) = self.computer.service.as_mut()
            && let Err(error) = service.client_mut().check_processes()
        {
            self.computer_failed(&error).await;
        }
        let running = self.computer.service.is_some() || self.computer.work.active().is_some();
        let facts = engine::Facts {
            observed: self.engine_observed(),
            closing: self.closing,
            running,
            retry_due: self
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
    pub(in crate::daemon) fn poll_permissions(&mut self) {
        let engine_running =
            self.computer.service.is_some() || self.computer.work.active().is_some();
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
                .computer
                .next_permission_poll
                .is_some_and(|due| now < due)
        {
            return;
        }
        // One question at a time: the answer to the last one removes it from
        // the pending list. Until then, or until it is given up, nothing new
        // is asked, so an unanswered poll never piles up there.
        if let Some((id, sent_at)) = self.computer.permission_poll.clone()
            && self.helper.awaits_reply(&id)
        {
            if now.saturating_duration_since(sent_at) < PERMISSION_POLL_ANSWER_DEADLINE {
                return;
            }
            self.helper.forget_reply(&id);
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
        if self.helper.try_send(command.clone()).is_ok() {
            self.helper.record_pending(&command);
            self.computer.permission_poll = command["id"].as_str().map(|id| (id.to_owned(), now));
        }
        self.computer.next_permission_poll = Some(now + PERMISSION_POLL_INTERVAL);
    }

    /// Says that the engine failed, never quietly: the cause's text,
    /// with `context` (which request, which operation), in daemon.log, and a
    /// typed `engine_failed` line in the report log, both under the turn's op.
    /// The text may name paths on the Mac, so it never leaves daemon.log.
    pub(super) fn note_engine_failure(
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
    pub(in crate::daemon) fn engine_unavailable(
        &mut self,
        kind: orbit_core::refusal::EngineCause,
        error: &anyhow::Error,
        context: &[(&str, &str)],
    ) {
        self.note_engine_failure(kind, error, context);
        self.computer
            .engine_unavailable_state(kind, &format!("{error:#}"));
    }

    /// The engine as the app shows it and a remote caller is told about it.
    /// A known refusal comes first even while an engine is still connected:
    /// it is what the next request gets, and the next turn stops the engine.
    pub(in crate::daemon) fn engine_status(&self) -> engine::Status {
        if let Some(refusal) = self.computer_revocation()
            && let Some(status) = engine::Status::refused(refusal)
        {
            return status;
        }
        let connected = self
            .computer
            .service
            .as_ref()
            .is_some_and(|service| service.client().is_connected());
        let connecting = self.computer.work.active().is_some()
            && self
                .computer
                .view
                .as_ref()
                .is_some_and(|view| view.stage == ComputerStage::Connecting);
        // The count taken when the engine connected: this runs for every state
        // the daemon publishes, and counting the catalogue again would copy all
        // of it (147 909 bytes on a MacBook) each time.
        if (connected || self.computer.work.active().is_some() && !connecting)
            && let Some(tools) = self.computer.declared_tools
        {
            return engine::Status::Ready { tools };
        }
        if connected {
            // Only an engine handed over without connecting here has no count.
            let tools = self
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
        if let Some(cause) = &self.computer.engine_failure {
            let retry_in_ms = self.computer.engine_retry_at.map_or(0, |deadline| {
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

    /// The packaged driver beside the daemon's own executable, and the private
    /// socket under Orbit's state root. Nothing outside the daemon chooses them.
    pub(super) fn engine_paths(&self) -> Result<(PathBuf, PathBuf)> {
        #[cfg(test)]
        let daemon_executable = match &self.computer.daemon_executable {
            Some(path) => path.clone(),
            None => std::env::current_exe()
                .context("Could not locate the daemon executable to find CUA")?,
        };
        #[cfg(not(test))]
        let daemon_executable = std::env::current_exe()
            .context("Could not locate the daemon executable to find CUA")?;
        let daemon_executable = daemon_executable
            .canonicalize()
            .context("Could not resolve the daemon executable to find CUA")?;
        let driver = crate::cua::resolve_driver_path("", &daemon_executable)?;
        Ok((driver, crate::cua::default_socket_path(&self.paths.root)))
    }

    /// Starts the packaged engine. It needs what macOS grants and
    /// the privacy stop released, and nothing else — not the pairing, not the
    /// hub, and no order naming an executable.
    pub(in crate::daemon) fn connect_computer(&mut self) -> Result<()> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(refusal.into());
        }
        let generation = self.state.generation();
        self.computer.require_computer_idle()?;
        if self.computer.service.is_some() {
            bail!("Disconnect current control before opening another connection");
        }
        let (driver_path, socket_path) = self.engine_paths()?;
        if driver_path.as_os_str().len() > 4096 || socket_path.as_os_str().len() > 4096 {
            bail!("The bundled CUA engine path exceeds the 4096-byte limit");
        }
        let events = self.work_events.clone();
        self.begin_computer(
            ComputerStage::Connecting,
            "Connecting to the local CUA engine; no browser is started.",
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

    pub(in crate::daemon) async fn maintain_computer(&mut self) -> Result<()> {
        self.poll_permissions();
        // The engine follows what macOS grants — started, kept or
        // stopped with its cause — every turn, with nobody asking.
        self.maintain_engine().await;
        Ok(())
    }
}
