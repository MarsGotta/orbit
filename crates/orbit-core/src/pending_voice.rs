//! The voice notes waiting on this Mac and the conversation each was spoken
//! in (`AssistantState::pending_voice`).
use crate::model::{Message, VoiceTranscriptionFailure};
use crate::transition::AssistantState;
use serde::{Deserialize, Serialize};

/// A voice note the hub has not taken yet, with the conversation it was
/// spoken in: it is shown there and nowhere else, and sent there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingVoiceNote {
    #[serde(flatten)]
    pub message: Message,
    /// Where it was spoken. Absent on a note kept by Orbit 0.11.0, which did
    /// not record it: unknown, so it stays hidden locally and no new
    /// view adopts it. An unsent note can still reach its own hub conversation.
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
    /// A new view that was left before the hub named it. Its first note's
    /// id groups its notes durably; it is never the next new view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abandoned_draft: Option<String>,
}

impl PendingVoiceNote {
    /// A note in a view that was left, including notes whose old format
    /// did not record a view. They stay hidden until delivered to the hub.
    pub fn detached(&self) -> bool {
        self.conversation
            .as_ref()
            .is_none_or(|conversation| conversation.abandoned_draft.is_some())
    }

    /// Old noise is removed only when no hub conversation owns it.
    pub fn obsolete_sessionless(&self) -> bool {
        self.conversation
            .as_ref()
            .is_none_or(|conversation| conversation.session_id.is_none())
            && obsolete_voice_message(&self.message)
    }

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
                abandoned_draft,
            }) => {
                abandoned_draft.is_none()
                    && assistant.session_id.is_none()
                    && assistant.agent_id == *agent_id
            }
            None => false,
        }
    }
}

/// Three seconds bounds only refused/empty noise retained as an error, not
/// accepted short words or the detector's 160 ms speech submission floor.
/// Matches the hub's `NO_SPEECH_REFUSAL_MAX_MS`: preroll and closing silence
/// leave under two seconds of possible speech in these refused fragments.
/// The Mac also applies it to old-hub failures and stored notes on load.
/// Longer notes remain available for playback and discard.
pub const SHORT_REFUSED_VOICE_MS: u64 = 3_000;

pub fn omit_voice_failure(duration_ms: u64, failure: &VoiceTranscriptionFailure) -> bool {
    duration_ms < SHORT_REFUSED_VOICE_MS
        && (failure.code == "transcription_provider_refused"
            || (failure.code == "transcription_http_error" && failure.provider_status == Some(400))
            || failure.code == "voice_transcription_empty")
}

pub(crate) fn obsolete_voice_message(message: &Message) -> bool {
    message.voice.as_ref().is_some_and(|voice| {
        voice.transcript.is_none()
            && voice.failure.as_ref().is_some_and(|failure| {
                failure.code == "voice_transcription_joined_next_turn"
                    || omit_voice_failure(voice.duration_ms, failure)
            })
    })
}
