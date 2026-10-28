//! One tool the engine declared, run once: checked, recorded in the history
//! before it is tried, and never repeated when its receipt is lost.
use super::*;

/// The application a receipt is filed under, taken from the engine's own
/// arguments. Orbit does not interpret the call beyond naming it in the record.
pub(super) fn tool_application(call: &ToolCall) -> String {
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

impl Runtime {
    /// One tool the engine declared, run now. No preparation, no
    /// confirmation, no short list and no per-application grant: the macOS
    /// permission was the decision. Without an engine this fails by name; it
    /// never diverts to another path.
    pub(in crate::daemon) fn call_engine_tool(
        &mut self,
        call: ToolCall,
        channel: &str,
        agent: Option<orbit_core::computer_control::ControlAgent>,
    ) -> std::result::Result<(), ComputerStartError> {
        let call = self.computer.engine_tool_call(call)?;
        let authority = self.admit_engine_tool(&call)?;
        let operation_id = self.record_tool_attempt(&call, channel, agent)?;
        let mut service = self
            .computer
            .service
            .take()
            .ok_or(ComputerStartError::EngineUnavailable)?;
        self.begin_computer_view(
            ComputerStage::Executing,
            &format!(
                "Executing «{}» with the engine; a lost receipt will not be repeated.",
                call.tool
            ),
            authority.generation,
        );
        let current = self
            .computer
            .authority
            .as_ref()
            .ok_or(ComputerStartError::Internal)?
            .subscribe();
        let events = self.work_events.clone();
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
                deliver_computer(&events, ticket, ComputerCompletion { service, result }).await;
            })
        })
        .map_err(|_| ComputerStartError::Internal)?;
        Ok(())
    }

    /// The engine may run this tool now: authority, nothing else running,
    /// a ready engine that declared it.
    pub(super) fn admit_engine_tool(
        &self,
        call: &ToolCall,
    ) -> std::result::Result<ActionAuthority, ComputerStartError> {
        let authority = self.computer_start_authority()?;
        self.computer
            .require_computer_idle()
            .map_err(|error| ComputerStartError::Busy(error.to_string()))?;
        if !self.computer.engine_is_ready() {
            return Err(ComputerStartError::EngineUnavailable);
        }
        if !self
            .computer
            .service
            .as_ref()
            .is_some_and(|service| service.client().declares_tool(&call.tool))
        {
            return Err(ComputerStartError::UndeclaredTool(call.tool.clone()));
        }
        Ok(authority)
    }

    /// The attempt is in the history before it is made, its outcome unknown.
    pub(super) fn record_tool_attempt(
        &mut self,
        call: &ToolCall,
        channel: &str,
        agent: Option<orbit_core::computer_control::ControlAgent>,
    ) -> std::result::Result<String, ComputerStartError> {
        let operation_id = id();
        let history = ControlHistoryEntry::new(
            &operation_id,
            &call.tool,
            &tool_application(call),
            now(),
            ControlOutcome::Unknown,
        )
        .and_then(|entry| entry.from_channel(channel))
        .and_then(|entry| entry.from_agent(agent))
        .map_err(|_| ComputerStartError::InvalidHistory)?;
        self.upsert_control_history(history.clone())
            .map_err(ComputerStartError::Storage)?;
        self.computer.active_general = Some(history);
        Ok(operation_id)
    }
}

impl crate::daemon::computer_control::ComputerHandle {
    /// A desktop action is given on the fitted image Deimos was sent; the
    /// engine takes true screen pixels.
    pub(super) fn engine_tool_call(
        &self,
        call: ToolCall,
    ) -> std::result::Result<ToolCall, ComputerStartError> {
        call.validate()
            .map_err(|_| ComputerStartError::InvalidTool)?;
        match orbit_core::sources::map_desktop_action(&call.arguments, self.desktop_scale.as_ref())
        {
            Ok(Some(arguments)) => Ok(ToolCall {
                tool: call.tool,
                arguments,
            }),
            Ok(None) => Ok(call),
            Err(cause) => Err(ComputerStartError::DesktopCoordinates(cause)),
        }
    }
}
