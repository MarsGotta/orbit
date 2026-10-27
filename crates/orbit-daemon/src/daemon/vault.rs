//! Read-only vault calls run off the actor loop; the hub keeps all settings.
use super::*;
use orbit_core::vault::{VaultCall, VaultOutcome, VaultResponse};

impl Runtime {
    pub(super) fn serve_vault(&mut self, call: VaultCall, id: String, reply: ipc::ReplySender) {
        let hub = self.hub_access().and_then(|_| {
            self.note_hub()
                .ok_or(orbit_core::refusal::Refusal::NotPaired)
        });
        self.reads.tasks.push(tokio::spawn(async move {
            let outcome = match hub {
                Ok(hub) => crate::hub_vault::call(&hub, &call).await,
                Err(refusal) => VaultOutcome::Failed { refusal },
            };
            let _ = reply.send(VaultResponse { v: 1, id, outcome });
        }));
    }
}
