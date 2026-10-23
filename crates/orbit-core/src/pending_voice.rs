//! The voice notes waiting on this Mac and the conversation each was spoken
//! in (`DurableState::pending_voice`).
use crate::model::{DurableState, Message};
use serde::{Deserialize, Serialize};

/// A voice note the hub has not taken yet, with the conversation it was
/// spoken in: it is shown there and nowhere else, and sent there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingVoiceNote {
    #[serde(flatten)]
    pub message: Message,
    /// Where it was spoken. Absent on a note kept by Orbit 0.11.0, which did
    /// not record it: unknown, so it is shown only in a new conversation,
    /// never in one that exists, and no conversation adopts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<VoiceNoteConversation>,
}

/// The conversation a voice note was spoken in: its id, or `None` for a new
/// conversation of `agent_id` that has not started yet. The first message
/// that starts it, this note or another, makes the note that conversation's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceNoteConversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl PendingVoiceNote {
    /// Whether this note belongs to the conversation open in `durable`.
    pub fn belongs_to(&self, durable: &DurableState) -> bool {
        match &self.conversation {
            Some(VoiceNoteConversation {
                session_id: Some(session),
                ..
            }) => durable.session_id.as_deref() == Some(session.as_str()),
            Some(VoiceNoteConversation {
                session_id: None,
                agent_id,
            }) => durable.session_id.is_none() && durable.agent_id == *agent_id,
            None => durable.session_id.is_none(),
        }
    }
}

impl DurableState {
    /// The waiting voice notes of the open conversation, oldest first: what
    /// it shows after its messages. Another conversation's are not here.
    pub fn open_conversation_pending_voice(&self) -> impl Iterator<Item = &Message> {
        self.pending_voice
            .iter()
            .filter(|note| note.belongs_to(self))
            .map(|note| &note.message)
    }
}
