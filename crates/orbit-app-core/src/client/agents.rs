use super::*;
use orbit_core::agent_config::AgentCall;

impl DaemonClient {
    /// A vault call carries no credential back; the hub is the only store.
    pub async fn vault(
        &self,
        call: orbit_core::vault::VaultCall,
    ) -> Result<orbit_core::vault::VaultOutcome, AppError> {
        self.inner
            .daemon
            .vault(call)
            .await
            .map_err(|error| AppError::from_daemon(error.as_ref()))
    }

    /// Settings › Agents: one read or order of an agent's configuration,
    /// answered when the hub answers. Nothing is kept here: the hub is the
    /// single source, and the screen reads it again after an order.
    pub async fn agent(&self, call: AgentCall) -> Result<crate::AgentAnswer, AppError> {
        let outcome = self
            .inner
            .daemon
            .agent(call.clone())
            .await
            .map_err(|error| AppError::from_daemon(error.as_ref()))?;
        let mut call = call;
        if let AgentCall::TranscriptionSave { api_key, .. } = &mut call {
            *api_key = None;
        }
        if let AgentCall::AiProviderSave { provider, .. } = &mut call {
            provider.api_key = None;
        }
        Ok(crate::AgentAnswer { call, outcome })
    }
}
