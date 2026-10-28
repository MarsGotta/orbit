use super::*;
use orbit_core::agent_config::{AgentCall, AgentOutcome};

#[test]
fn export_vault_contract() -> Result<(), Box<dyn std::error::Error>> {
    orbit_core::vault::VaultCall::export_all()?;
    orbit_core::vault::VaultOutcome::export_all()?;
    Ok(())
}

app_type! {
    /// Settings › Agents: one call to the hub and what came of it. The app
    /// keeps nothing of it; the screen holds what it shows.
    pub struct AgentAnswer {
        pub call: AgentCall,
        pub outcome: AgentOutcome,
    }
}
