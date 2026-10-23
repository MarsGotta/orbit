//! Connection, pairing and privacy facts for the desktop status view.
use crate::{AccountSummary, Connection};
use serde::{Deserialize, Serialize};

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// 102-byte measured head; 4 KiB catches its accidental growth, not status
/// data. The hub's agents ride it too: `agents::MAX_AGENTS` at the hub's
/// bounds, each with its profile, account and usage (a 90-character
/// description, four bytes a character, a 128-byte address), measured
/// 34,329 bytes for the whole topic on 2026-10-22, so 32 KiB more holds
/// them with 6 KiB to spare (`agents_at_their_ceiling_keep_the_status_topic`).
pub const MAX_BYTES: usize = 36 * 1024;
/// Published as soon as it changes.
pub const MIN_INTERVAL_MS: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    /// The agents a conversation can be with, in the hub's order: the
    /// last list the hub gave, or the one agent of a hub that names none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<crate::agents::Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    /// The services an agent can be added with from Orbit; absent for a hub
    /// without «Assistants from Orbit», which adds none and hands no
    /// conversation over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addable_agents: Option<Vec<String>>,
    /// The connected hub's attachment ceilings, from its last `ready`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<crate::attachments::AttachmentLimits>,
    pub hub: Connection,
    pub account: Option<AccountSummary>,
    pub privacy_stopped: bool,
    pub helper_connected: bool,
    /// «Enviar informes de errores anónimos»: the user's choice. `None` when
    /// this build carries no Sentry address, so the switch is not offered;
    /// a daemon from before the switch says nothing, which reads the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reports: Option<bool>,
}
