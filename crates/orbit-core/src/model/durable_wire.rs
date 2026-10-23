//! The saved state as it is written and read: flat, its keys in the order
//! Orbit has always written them, whichever domain each belongs to. A
//! snapshot written before the state had one part per domain loads, and
//! writes back the same bytes (`tests/persisted_shape.rs`).
use super::{Approval, Capabilities, Connection, DurableState, Recording, Turn};
use crate::transition::{AssistantState, CaptureState, HubLinkState, PrivacyState, VoiceState};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};

/// The saved state's reader. Unknown keys are ignored, as they always
/// were: a snapshot that still names the daemon's version loads.
#[derive(Deserialize)]
pub(super) struct DurableWire {
    schema: u8,
    generation: u64,
    privacy_stopped: bool,
    /// Omitted rather than nulled so a state that is not stopped stays
    /// byte-identical on the wire.
    #[serde(default)]
    privacy_stop_by: Option<crate::privacy_stop::StopRequester>,
    connection: Connection,
    session_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    active_request: Option<String>,
    /// A state saved before the hub kept the conversations held up to 200
    /// messages here, under `messages`; it loads without a turn.
    #[serde(default)]
    turn: Option<Turn>,
    #[serde(default)]
    pending_voice: Vec<crate::PendingVoiceNote>,
    recording: Option<Recording>,
    #[serde(default)]
    discarding: Option<String>,
    voice_active: bool,
    #[serde(default)]
    call: Option<crate::voice::CallState>,
    pending_approval: Option<Approval>,
    capabilities: Capabilities,
    #[serde(default, deserialize_with = "crate::notice::lenient_notice")]
    notice: Option<crate::notice::Notice>,
    #[serde(default)]
    notice_log: crate::notice::NoticeLog,
    pending_jobs: u64,
}

impl From<DurableWire> for DurableState {
    fn from(wire: DurableWire) -> Self {
        Self {
            schema: wire.schema,
            privacy: PrivacyState {
                generation: wire.generation,
                stopped: wire.privacy_stopped,
                stop_by: wire.privacy_stop_by,
            },
            link: HubLinkState {
                connection: wire.connection,
            },
            assistant: AssistantState {
                session_id: wire.session_id,
                agent_id: wire.agent_id,
                active_request: wire.active_request,
                turn: wire.turn,
                pending_voice: wire.pending_voice,
                pending_approval: wire.pending_approval,
            },
            capture: CaptureState {
                recording: wire.recording,
                discarding: wire.discarding,
            },
            voice: VoiceState {
                voice_active: wire.voice_active,
                call: wire.call,
            },
            capabilities: wire.capabilities,
            notice: wire.notice,
            notice_log: wire.notice_log,
            pending_jobs: wire.pending_jobs,
        }
    }
}

/// Written field by field, in the saved order, borrowing every part: a
/// snapshot or a served state never clones the state to write it.
impl Serialize for DurableState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (privacy, assistant) = (&self.privacy, &self.assistant);
        let mut wire = serializer.serialize_struct("DurableState", 19)?;
        wire.serialize_field("schema", &self.schema)?;
        wire.serialize_field("generation", &privacy.generation)?;
        wire.serialize_field("privacy_stopped", &privacy.stopped)?;
        optional(&mut wire, "privacy_stop_by", &privacy.stop_by)?;
        wire.serialize_field("connection", &self.link.connection)?;
        wire.serialize_field("session_id", &assistant.session_id)?;
        optional(&mut wire, "agent_id", &assistant.agent_id)?;
        wire.serialize_field("active_request", &assistant.active_request)?;
        optional(&mut wire, "turn", &assistant.turn)?;
        if assistant.pending_voice.is_empty() {
            wire.skip_field("pending_voice")?;
        } else {
            wire.serialize_field("pending_voice", &assistant.pending_voice)?;
        }
        wire.serialize_field("recording", &self.capture.recording)?;
        optional(&mut wire, "discarding", &self.capture.discarding)?;
        wire.serialize_field("voice_active", &self.voice.voice_active)?;
        wire.serialize_field("call", &self.voice.call)?;
        wire.serialize_field("pending_approval", &assistant.pending_approval)?;
        wire.serialize_field("capabilities", &self.capabilities)?;
        wire.serialize_field("notice", &self.notice)?;
        wire.serialize_field("notice_log", &self.notice_log)?;
        wire.serialize_field("pending_jobs", &self.pending_jobs)?;
        wire.end()
    }
}

/// A key that is omitted rather than nulled while it has no value.
fn optional<W: SerializeStruct, T: Serialize>(
    wire: &mut W,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), W::Error> {
    match value {
        Some(_) => wire.serialize_field(key, value),
        None => wire.skip_field(key),
    }
}
