use super::*;
use orbit_core::agent_config::AgentCall;

impl DaemonClient {
    /// Ajustes › Agentes: one read or order of an agent's configuration,
    /// answered when the hub answers. Nothing is kept here: the hub is the
    /// single source, and the screen reads it again after an order.
    pub async fn agent(&self, call: AgentCall) -> Result<crate::AgentAnswer, AppError> {
        let outcome = self
            .inner
            .daemon
            .agent(call.clone())
            .await
            .map_err(|error| AppError::from_daemon(error.as_ref()))?;
        Ok(crate::AgentAnswer { call, outcome })
    }
}
