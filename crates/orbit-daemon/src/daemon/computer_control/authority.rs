//! Who may control the Mac now: the authority a worker starts with and keeps
//! watching, and why it is refused or revoked.
use super::*;

impl Runtime {
    pub(super) fn computer_start_authority(
        &self,
    ) -> std::result::Result<ActionAuthority, ComputerStartError> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(ComputerStartError::Authority { refusal });
        }
        let authority = self.computer_authority();
        if authority.device_id.is_empty() {
            return Err(ComputerStartError::NotPaired);
        }
        Ok(authority)
    }

    pub(super) fn computer_authority(&self) -> ActionAuthority {
        ActionAuthority {
            device_id: self.settings.device_id.clone().unwrap_or_default(),
            generation: self.state.generation(),
            now_ms: now(),
            online: self.state.connection() == Connection::Online,
            privacy_stopped: self.state.privacy_stopped(),
        }
    }

    /// Why the engine may not start work now, if it may not: the privacy
    /// stop, or a permission macOS does not grant or has not reported yet.
    /// Nothing else. No switch, and neither the hub nor the
    /// conversation, which only matter to a request that came through them.
    pub(in crate::daemon) fn computer_refusal(&self) -> Option<orbit_core::refusal::Refusal> {
        engine::admits(engine::SystemPermissions::ENGINE, self.engine_observed()).err()
    }

    /// What the Mac has told the daemon about what macOS grants, and whether
    /// the helper that tells it is there at all.
    pub(in crate::daemon) fn engine_observed(&self) -> engine::Observed {
        engine::Observed {
            privacy_stopped: self.state.privacy_stopped(),
            helper_connected: self.state.capabilities().native_helper(),
            accessibility: self.state.capabilities().accessibility_state(),
            screen_recording: self.state.capabilities().screen_state(),
        }
    }

    /// Why work already running on the engine must stop, if it must: only the
    /// known facts, the privacy stop and a permission macOS withdrew. A
    /// permission that is merely not known right now leaves it running.
    pub(super) fn computer_revocation(&self) -> Option<orbit_core::refusal::Refusal> {
        engine::revokes(self.engine_observed())
    }

    pub(super) fn require_computer_authority(&self) -> Result<ActionAuthority> {
        if let Some(refusal) = self.computer_refusal() {
            return Err(refusal.into());
        }
        let authority = self.computer_authority();
        // Each receipt is filed under the paired device; without one there is
        // nothing to file it under. This is the pairing, not a second consent.
        if authority.device_id.is_empty() {
            bail!(
                "El control guarda cada recibo a nombre del Mac emparejado, y este Mac no está emparejado con el hub"
            );
        }
        Ok(authority)
    }
}
