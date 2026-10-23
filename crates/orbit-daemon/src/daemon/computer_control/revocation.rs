//! Control withdrawn: the worker's ticket goes before any fallible write or
//! await, and the view says why, with an action in flight still uncertain.
use super::*;

impl Runtime {
    /// Invalidate before any fallible write or await. A queued late result cannot
    /// restore the client: its ticket is removed before the worker is aborted.
    pub(in crate::daemon) async fn revoke_computer(
        &mut self,
        refusal: &orbit_core::refusal::Refusal,
    ) {
        self.revoke_computer_with_cause(
            &refusal.to_string(),
            ComputerRevocation::Refusal(refusal.clone()),
        )
        .await;
    }

    pub(in crate::daemon) async fn revoke_computer_with_cause(
        &mut self,
        detail: &str,
        cause: ComputerRevocation,
    ) {
        let retiring_remote_control = self.assistant.bridge_request.as_ref().is_some_and(|ask| {
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
                            super::super::BridgeReleaseReason::PrivacyStopped
                        } else {
                            super::super::BridgeReleaseReason::Authority(change)
                        };
                    self.release_bridge_request(reason);
                }
                ComputerRevocation::Refusal(refusal) => {
                    self.release_bridge_request(
                        super::super::BridgeReleaseReason::ComputerRevoked(refusal),
                    );
                }
            }
        }
        let had_active = self.computer.work.active().is_some()
            || self.computer.authority.is_some()
            || self.computer.service.is_some()
            || self.computer.active_general.is_some();
        let in_flight = self.computer.active_general.is_some();
        let mut history = self.computer.active_general.take();
        if let Some(entry) = history.as_mut() {
            entry.outcome = if in_flight {
                ControlOutcome::Unknown
            } else {
                ControlOutcome::Cancelled
            };
        }
        self.computer.pending_cancel_uncertain |= in_flight;
        self.revoke_computer_state(detail, had_active, cause);
        if let Some(entry) = history {
            let _ = self.upsert_control_history(entry);
        }
    }

    pub(super) fn revoke_computer_state(
        &mut self,
        detail: &str,
        had_active: bool,
        cause: ComputerRevocation,
    ) {
        if let Some(authority) = self.computer.authority.take() {
            authority.send_replace(WorkerAuthority {
                grant: self.computer_authority(),
                revoked: Some(cause),
            });
        }
        self.computer.work.cancel();
        if let Some(mut service) = self.computer.service.take() {
            service.invalidate();
        }
        self.computer.declared_tools = None;
        if had_active && let Some(view) = self.computer.view.as_mut() {
            view.connected = false;
            view.connection_epoch = None;
            view.observation = None;
            view.stage = if self.computer.pending_cancel_uncertain {
                ComputerStage::Unknown
            } else {
                ComputerStage::Cancelled
            };
            view.detail = detail.into();
        }
    }
}
