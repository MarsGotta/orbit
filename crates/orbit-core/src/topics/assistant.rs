//! The live turn: the open conversation, the turn in progress and
//! what is pending. The conversation's messages are read from the hub.
use crate::{
    Approval, Turn,
    topics::{Knowledge, Pending},
};
use serde::{Deserialize, Serialize};

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
}

fn unknown_call() -> Knowledge<Option<crate::voice::CallState>> {
    Knowledge::Unknown {
        why: Pending::NotObservedYet,
    }
}
