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
    /// The turn in progress, or the last one until another begins. Boxed:
    /// two messages would make every topic frame as large as this one.
    pub turn: Option<Box<Turn>>,
    pub pending: Option<Approval>,
    pub voice_active: bool,
    pub turn_in_flight: bool,
    pub activity: Option<Activity>,
    pub voice: VoiceAvailability,
    #[serde(default = "unknown_call")]
    pub call: Knowledge<Option<crate::voice::CallState>>,
}

fn unknown_call() -> Knowledge<Option<crate::voice::CallState>> {
    Knowledge::Unknown {
        why: Pending::NotObservedYet,
    }
}
