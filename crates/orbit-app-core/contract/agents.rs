use super::*;
use orbit_core::agent_config::{AgentCall, AgentOutcome};

app_type! {
    /// Ajustes › Agentes: one call to the hub and what came of it. The app
    /// keeps nothing of it; the screen holds what it shows.
    pub struct AgentAnswer {
        pub call: AgentCall,
        pub outcome: AgentOutcome,
    }
}
