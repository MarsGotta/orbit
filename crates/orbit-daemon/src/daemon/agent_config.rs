//! Ajustes › Agentes: each call is prepared on the actor's loop,
//! with the device's credential, and answered off it when the hub answers,
//! so a slow hub never holds audio or other daemon events. Nothing of an
//! agent's configuration is kept here: the hub is its single source.
use super::*;
use orbit_core::agent_config::{AgentCall, AgentOutcome};
use orbit_core::protocol::AgentResponse;

impl Runtime {
    /// The request of one call, ready to run off the loop; the call ends in
    /// its refusal when the hub cannot be asked now (not paired, privacy
    /// stop, no link), without a request.
    fn agent_task(
        &self,
        call: AgentCall,
    ) -> impl std::future::Future<Output = AgentOutcome> + Send + 'static {
        let hub = self.hub_access().and_then(|_| {
            self.note_hub()
                .map(|hub| crate::hub_agents::AgentHub {
                    base: hub.base,
                    token: hub.token,
                    op: hub.op,
                })
                .ok_or(orbit_core::refusal::Refusal::NotPaired)
        });
        async move {
            match hub {
                Ok(hub) => hub.call(&call).await,
                Err(refusal) => AgentOutcome::Failed { refusal },
            }
        }
    }

    #[cfg(test)]
    pub(super) async fn agent_call(&self, call: AgentCall) -> AgentOutcome {
        self.agent_task(call).await
    }

    /// `Command::Agent`: answered with its outcome once the hub answers.
    pub(super) fn serve_agent(
        &mut self,
        call: AgentCall,
        request_id: String,
        reply: ipc::ReplySender,
    ) {
        let task = self.agent_task(call);
        let log_root = self.paths.root.clone();
        self.reads.tasks.push(tokio::spawn(async move {
            let outcome = task.await;
            let response = AgentResponse {
                v: 1,
                id: request_id,
                outcome,
            };
            if reply.send(response).is_err() {
                crate::diagnostics::DiagnosticLog::in_root(&log_root)
                    .record("ipc_reply_receiver_gone", &[("command", "agent")]);
            }
        }));
    }
}
