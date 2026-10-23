//! The voice notes waiting on this Mac and the conversation each was spoken
//! in (`AssistantState::pending_voice`).
use crate::model::Message;
use crate::transition::AssistantState;
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
    /// Whether this note belongs to the conversation open in `assistant`.
    pub fn belongs_to(&self, assistant: &AssistantState) -> bool {
        match &self.conversation {
            Some(VoiceNoteConversation {
                session_id: Some(session),
                ..
            }) => assistant.session_id.as_deref() == Some(session.as_str()),
            Some(VoiceNoteConversation {
                session_id: None,
                agent_id,
            }) => assistant.session_id.is_none() && assistant.agent_id == *agent_id,
            None => assistant.session_id.is_none(),
        }
    }
}
