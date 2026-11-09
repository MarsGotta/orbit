//! Plugin calls run off the actor loop; the daemon keeps nothing of them.
use super::*;
use crate::hub_notes::NoteHub;
use orbit_core::plugin::{PluginCall, PluginOutcome, PluginResponse};
use orbit_core::refusal::Refusal;

/// Answers a plugin call from a task on `tasks`, with the hub the caller
/// resolved or the refusal it got before reaching the network.
pub(super) fn serve_plugin(
    hub: Result<NoteHub, Refusal>,
    tasks: &mut Vec<JoinHandle<()>>,
    plugin: String,
    call: PluginCall,
    id: String,
    reply: ipc::ReplySender,
) {
    tasks.push(tokio::spawn(async move {
        let outcome = match hub {
            Ok(hub) => crate::hub_plugin::call(&hub, &plugin, &call).await,
            Err(refusal) => PluginOutcome::Failed { refusal },
        };
        let _ = reply.send(PluginResponse { v: 1, id, outcome });
    }));
}
