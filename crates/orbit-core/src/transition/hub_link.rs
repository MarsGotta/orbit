//! The link to the hub: whether it is offline, connecting, online, or the
//! hub revoked this device.
//!
//! Reads of other domains: the privacy stop (a stopped Orbit does not
//! connect).
use super::Step;
use crate::authority::AuthorityChange;
use crate::model::Connection;
use crate::refusal::Refusal;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HubLinkState {
    pub(crate) connection: Connection,
}

impl HubLinkState {
    pub fn connection(&self) -> Connection {
        self.connection.clone()
    }
    pub(super) fn revoked(&self) -> bool {
        self.connection == Connection::Revoked
    }
    /// Refuses what needs the hub while it is not online.
    pub(super) fn ensure_online(&self) -> Result<(), Refusal> {
        if self.connection == Connection::Online {
            Ok(())
        } else {
            Err(Refusal::HubUnreachable { since_ms: None })
        }
    }
    /// The authority a new `connection` changes: a change of link, or a
    /// revocation said again.
    pub(super) fn authority_for(&self, connection: &Connection) -> Option<AuthorityChange> {
        if *connection == self.connection && *connection != Connection::Revoked {
            return None;
        }
        Some(match connection {
            Connection::Revoked => AuthorityChange::Revoked,
            Connection::Offline => AuthorityChange::HubLinkOffline,
            Connection::Connecting | Connection::Online => AuthorityChange::HubLink,
        })
    }
    pub(super) fn set(&mut self, connection: Connection) {
        self.connection = connection;
    }
    /// Offline, unless the hub revoked this device: that stays said.
    pub(super) fn go_offline(&mut self) {
        if self.connection != Connection::Revoked {
            self.connection = Connection::Offline;
        }
    }
}

/// The local link is starting; no authority has changed yet.
pub(super) fn connecting(link: &mut HubLinkState, cx: &mut Step) -> Result<(), Refusal> {
    if cx.before.privacy.stopped() || cx.before.link.revoked() {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    link.connection = Connection::Connecting;
    Ok(())
}
