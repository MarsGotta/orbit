//! The live turn: the open conversation, the turn in progress and
//! what is pending. The conversation's messages are read from the hub.
use crate::{
    Approval, Turn,
    topics::{Knowledge, Pending},
};
use serde::{Deserialize, Serialize};

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// One turn, never the conversation (the app reads that from the
/// hub), and the agent's work in it. A turn at its ceilings — a 32 KiB
/// question, a 96 KiB answer (`MAX_TEXT_BYTES` and three times it) and a
/// 16 KB pending description — in characters that escape to two bytes
/// (quotes, line breaks) measured 294,695 bytes; with its work at its
/// ceilings (`agent_work`: every step kept, in four-byte characters, and
/// the one plan) 407,761, under 416 KiB (425,984; the daemon's
/// `a_turn_and_its_work_at_their_ceilings_fit_the_topic`). Control
/// characters escape to six bytes; such a turn is refused with its size.
pub const MAX_BYTES: usize = 416 * 1024;
/// 100 ms bounds streamed-text redraws, not hub message throughput.
pub const MIN_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Transcribing,
    Thinking,
    Responding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceAbsence {
    SpeechUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VoiceAvailability {
    Unknown { why: Pending },
    Absent { code: VoiceAbsence },
    Present,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    /// The open conversation, `None` before its first turn.
    pub session_id: Option<String>,
    /// The agent the open conversation is with; `None` is the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The turn in progress, or the last one until another begins. Boxed:
    /// two messages would make every topic frame as large as this one.
    pub turn: Option<Box<Turn>>,
    /// Voice notes the hub has not taken yet that a later turn replaced,
    /// kept on this Mac (`DurableState::pending_voice`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_voice: Vec<crate::Message>,
    pub pending: Option<Approval>,
    pub voice_active: bool,
    pub turn_in_flight: bool,
    pub activity: Option<Activity>,
    pub voice: VoiceAvailability,
    /// The answer being read aloud at the user's request («Escuchar»), by
    /// its message id, from the order until the helper says it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking: Option<String>,
    #[serde(default = "unknown_call")]
    pub call: Knowledge<Option<crate::voice::CallState>>,
    /// The last voice note not sent for being too short, with its numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_voice_note: Option<crate::voice::ShortVoiceNote>,
    /// What the agent did in the turn in progress, or the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<Box<crate::agent_work::AgentWork>>,
    /// The call's microphone is muted («Silenciar»), as the Mac confirmed
    /// it: the assistant does not hear the user, who still hears it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub microphone_muted: bool,
}

fn unknown_call() -> Knowledge<Option<crate::voice::CallState>> {
    Knowledge::Unknown {
        why: Pending::NotObservedYet,
    }
}
