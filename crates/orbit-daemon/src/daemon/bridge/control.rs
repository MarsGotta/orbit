//! Control of the Mac for Deimos: the engine's tool catalogue, one action at
//! a time, the receipt of its single attempt, and every way the engine says
//! no.
use super::{BridgeAsk, result_payload};
use crate::daemon::{Runtime, hub};
use orbit_core::computer_control::engine::Status;
use orbit_core::refusal::{EngineCause, Expected, Operation, Parameter, Refusal};
use orbit_core::wire::bridge::{
    ActionReceipt, BridgeResult, ControlToolsResult, DeviceReplyParams, PerformActionResult,
};
use serde_json::{Value, json};

impl Runtime {
    /// The catalogue comes from the engine, asked of it, not from a list
    /// Orbit keeps. A newer engine's tool appears here on its own.
    pub(super) async fn serve_control_tools(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        if !self.engine_is_ready() {
            return Err(self.bridge_engine_refusal());
        }
        // One page, never the whole catalogue. The engine's is
        // 147 909 bytes and the link refuses anything over 128 KiB, so
        // a reply that asked for a whole frame was a reply that never
        // left the Mac.
        let catalogue = self.engine_tools();
        let page = orbit_core::device_bridge::catalogue_page(&catalogue, ask.catalogue_after);
        let result = BridgeResult::ControlTools(ControlToolsResult {
            tools: page.tools,
            after_index: page.after_index,
            total: page.total,
            next_after_index: page.next_after_index,
        });
        self.conversation.bridge_request = None;
        self.send(
            hub::command(&ask.id, "device.reply", result_payload(&ask.id, result))
                .map_err(|_| Refusal::HubUnreachable { since_ms: None })?,
        )
        .await
        .map_err(|_| Refusal::HubUnreachable { since_ms: None })
    }

    /// The engine or nothing. There is no adapter fallback left to divert
    /// into, and an absent engine is said out loud.
    pub(super) fn serve_perform_action(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        let call = ask.tool_call.clone().ok_or(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::ValidSelection,
        })?;
        self.call_engine_tool(call, "Deimos (canal no indicado)")
            .map_err(|error| self.bridge_computer_start_failure(error))
    }

    /// A start's refusal, with the engine's next retry when it failed.
    pub(super) fn bridge_start_frame(&self, request_id: &str, refusal: &Refusal) -> Value {
        let mut reply = DeviceReplyParams::refused(request_id.to_owned(), refusal.clone(), None);
        if let Status::Failed { retry_in_ms, .. } = self.engine_status()
            && matches!(refusal, Refusal::EngineUnavailable { .. })
            && let Some(ref mut fields) = reply.refusal
        {
            fields.retry_in_ms = Some(retry_in_ms);
        }
        json!(reply)
    }

    pub(super) fn bridge_engine_refusal(&self) -> Refusal {
        use orbit_core::refusal::{missing_permission_name, missing_system_permission_refusal};
        match self.engine_status() {
            Status::Failed { .. } => Refusal::EngineUnavailable {
                cause: EngineCause::InitializationFailed,
            },
            Status::Starting => Refusal::EngineUnavailable {
                cause: EngineCause::NotRunning,
            },
            Status::Ready { .. } => Refusal::EngineUnavailable {
                cause: EngineCause::Unhealthy,
            },
            Status::PrivacyStopped => Refusal::PrivacyStopped { since_ms: None },
            Status::HelperNotConnected => Refusal::HelperNotRunning,
            Status::PermissionMissing { permissions } => missing_system_permission_refusal(
                permissions,
                self.state.capabilities().accessibility_state(),
                self.state.capabilities().screen_state(),
            ),
            Status::PermissionUnknown { permissions } => Refusal::PermissionUnknown {
                permission: missing_permission_name(permissions),
            },
        }
    }

    pub(super) fn bridge_computer_start_failure(
        &self,
        error: crate::daemon::computer_control::ComputerStartError,
    ) -> Refusal {
        use crate::daemon::computer_control::ComputerStartError;
        match error {
            ComputerStartError::Authority { refusal, .. } => refusal,
            ComputerStartError::NotPaired => Refusal::NotPaired,
            ComputerStartError::Busy(_) => Refusal::Busy {
                operation: Operation::ComputerAction,
                retry_after_ms: None,
            },
            ComputerStartError::InvalidTool => Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::ValidSelection,
            },
            ComputerStartError::UndeclaredTool(tool) => Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::DeclaredTool { received: tool },
            },
            ComputerStartError::InvalidHistory => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::ControlHistory,
            },
            ComputerStartError::Storage(_) => Refusal::StorageFailed {
                operation: Operation::ComputerAction,
                cause: None,
            },
            ComputerStartError::EngineUnavailable => self.bridge_engine_refusal(),
            ComputerStartError::CaptureFile(_) => Refusal::StorageFailed {
                operation: Operation::Capture,
                cause: None,
            },
            ComputerStartError::DesktopCoordinates(cause) => cause,
            ComputerStartError::Internal => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::ComputerStart,
            },
        }
    }

    pub(in crate::daemon) fn finish_remote_computer_action(
        &mut self,
        operation_id: &str,
        receipt: &Value,
    ) {
        let Some(ask) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::PerformAction
                    && ask.tool_call.is_some()
            })
            .cloned()
        else {
            return;
        };
        let tool = ask
            .tool_call
            .as_ref()
            .map_or("tool_call", |call| call.tool.as_str());
        // The action was attempted once and is never repeated when its receipt
        // cannot enter the queue. Queue admission is not a Deimos receipt.
        let result = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(
                &ask.id,
                BridgeResult::PerformAction(PerformActionResult {
                    receipt: ActionReceipt {
                        operation_id: operation_id.to_owned(),
                        tool: tool.to_owned(),
                        state: "performed".into(),
                        attempted: true,
                        driver_receipt: receipt.clone(),
                        retry: "no_repetir".into(),
                    },
                }),
            ),
        ));
        self.note_bridge_send(&result, "the receipt of the engine's single attempt");
        self.conversation.bridge_request = None;
    }
}
