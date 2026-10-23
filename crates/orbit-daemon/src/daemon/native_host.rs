//! Observed helper identity, shared by native consumers but never a grant.
//!
//! Only the daemon's native event path supplies these facts. IPC input, stored
//! history and Cua page contents cannot establish a native process identity.
use orbit_core::wire::native::NativeEventHeader;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct HostResponseFields {
    ok: bool,
    result: Option<Value>,
}

#[derive(Deserialize)]
struct HostStateFields<'a> {
    native_instance_id: &'a str,
    privacy_generation: u64,
    privacy_stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostIdentity {
    pub id: String,
    pub generation: u64,
    pub stopped: bool,
}

/// A revocation signal from one event, not permission to resume on `false`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct HostUpdate {
    pub authority_changed: bool,
    /// The same helper advanced exactly one epoch after its capture cut.
    pub capture_cut_rebound: bool,
    /// The helper is privacy-stopped (the user's latch, as `privacy_stop` or its
    /// `state` says) or went away.
    pub privacy_stopped: bool,
    /// The helper cut its capture without the user's latch
    /// (`capture_stopped`): the old epoch's bindings are gone like after the
    /// user's stop, but
    /// nothing is privacy-stopped, and consumers say what happened.
    pub capture_cut: bool,
}

impl HostUpdate {
    /// Whether the event retired what depended on the helper's epoch.
    pub fn stopped(self) -> bool {
        self.privacy_stopped || self.capture_cut
    }
}

/// RAM-only observations. A new daemon starts without any helper identity.
#[derive(Debug, Default)]
pub(super) struct NativeHost {
    identity: Option<HostIdentity>,
    cut_from: Option<HostIdentity>,
    calendar_backend_available: bool,
}

impl NativeHost {
    pub(super) fn identity(&self) -> Option<&HostIdentity> {
        self.identity.as_ref()
    }

    pub(super) fn calendar_backend_available(&self) -> bool {
        self.calendar_backend_available
    }

    pub(super) fn observe(&mut self, event: &Value) -> HostUpdate {
        let Ok(header) = NativeEventHeader::from_value(event) else {
            return HostUpdate::default();
        };
        match Some(header.event) {
            Some("ready") => {
                // Even a repeated ready invalidates the previous observations.
                // Wait for a separate state/readback before exposing identity.
                self.identity = None;
                self.cut_from = None;
                // The daemon launches the helper bundled with this version.
                self.calendar_backend_available = true;
                HostUpdate {
                    authority_changed: true,
                    ..HostUpdate::default()
                }
            }
            // A capture stop tears the helper down exactly like the user's stop
            // does (new privacy epoch, bindings invalidated) and only differs in
            // the durable latch, so the identity is stopped until the next
            // `state` says otherwise; it is reported as the cut it is, never
            // as a privacy stop.
            Some(name @ ("helper_disconnected" | "privacy_stop" | "capture_stopped")) => {
                self.cut_from = if name == "capture_stopped" {
                    self.identity.clone()
                } else {
                    None
                };
                if name == "helper_disconnected" {
                    self.calendar_backend_available = false;
                }
                if let Some(identity) = &mut self.identity {
                    identity.stopped = true;
                }
                let cut = name == "capture_stopped";
                HostUpdate {
                    authority_changed: false,
                    capture_cut_rebound: false,
                    privacy_stopped: !cut,
                    capture_cut: cut,
                }
            }
            Some("state") => self.observe_state(event),
            Some("response") => match HostResponseFields::deserialize(event) {
                Ok(response) if response.ok => response
                    .result
                    .as_ref()
                    .map_or_else(HostUpdate::default, |result| self.observe_state(result)),
                _ => HostUpdate::default(),
            },
            _ => HostUpdate::default(),
        }
    }

    fn observe_state(&mut self, state: &Value) -> HostUpdate {
        let Ok(fields) = HostStateFields::deserialize(state) else {
            return HostUpdate::default();
        };
        let id = fields.native_instance_id;
        let generation = fields.privacy_generation;
        let stopped = fields.privacy_stopped;
        if id.len() != 36 {
            return HostUpdate::default();
        }
        let Ok(uuid) = uuid::Uuid::parse_str(id) else {
            return HostUpdate::default();
        };
        if uuid.hyphenated().to_string() != id.to_lowercase() {
            return HostUpdate::default();
        }
        let authority_changed = self
            .identity
            .as_ref()
            .is_some_and(|previous| previous.id != id || previous.generation != generation);
        let capture_cut_rebound = self.cut_from.take().is_some_and(|previous| {
            previous.id == id && previous.generation.checked_add(1) == Some(generation) && !stopped
        });
        self.identity = Some(HostIdentity {
            id: id.into(),
            generation,
            stopped,
        });
        HostUpdate {
            authority_changed,
            capture_cut_rebound,
            privacy_stopped: stopped,
            capture_cut: false,
        }
    }
}

#[cfg(test)]
mod tests;
