// DurableState reader copied from v0.7.0, model.rs:515-560.
// Only import paths and the derive for use in this fixture are adapted.
use orbit_core::{Approval, Capabilities, Connection, Recording, Turn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DurableState {
    pub schema: u8,
    pub(crate) generation: u64,
    pub(crate) privacy_stopped: bool,
    /// Who asked for the stop in force (REF-12): only Mars's is carried
    /// across a restart as hers. `None` with a stop in force is a stop
    /// nobody can attribute, and is kept. Omitted rather than nulled so a
    /// state that is not stopped stays byte-identical on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) privacy_stop_by: Option<orbit_core::privacy_stop::StopRequester>,
    pub(crate) connection: Connection,
    pub(crate) session_id: Option<String>,
    pub active_request: Option<String>,
    #[serde(default)]
    pub source_inventory: Option<orbit_core::sources::WindowInventory>,
    #[serde(default)]
    pub source_request: Option<orbit_core::sources::SourceProgress>,
    #[serde(default)]
    pub source_receipt: Option<orbit_core::sources::SourceReceipt>,
    /// A state saved before SIM-30 held up to 200 messages here, under
    /// `messages`; it loads without a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
    pub recording: Option<Recording>,
    pub voice_active: bool,
    #[serde(default)]
    pub call: Option<orbit_core::voice::CallState>,
    pub pending_approval: Option<Approval>,
    pub(crate) capabilities: Capabilities,
    /// The last notice said, typed (SIM-21). A state saved before typed
    /// notices held a sentence here; it loads as no notice.
    #[serde(default, deserialize_with = "orbit_core::notice::lenient_notice")]
    pub notice: Option<orbit_core::notice::Notice>,
    /// The last notices in the order they happened, added by
    /// [`orbit_core::State::set_notice`] instead of replacing `notice`.
    #[serde(default)]
    pub notice_log: orbit_core::notice::NoticeLog,
    pub pending_jobs: u64,
}
