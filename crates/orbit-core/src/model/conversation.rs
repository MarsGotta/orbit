//! The conversation as this Mac keeps it: its turn, its voice notes and
//! the agent's questions, and the hub's pages of it.
use super::{is_false, is_zero_u64};
use serde::{Deserialize, Serialize};

/// Voice notes this Mac keeps waiting for the hub (spoken offline, or whose
/// transcription failed) once a later turn took their place. Each one is a
/// message in the saved state, a few hundred bytes, beside its WAV (a minute
/// at most) that the Mac keeps anyway for playback; fifty is an afternoon of
/// dictation with the hub away. Past it a new note is refused with the
/// count (`too_many`), never stored without a name. Not a limit on how many
/// notes the hub keeps.
pub const MAX_PENDING_VOICE_NOTES: usize = 50;

/// The failure of a voice note stored while another turn was in flight.
pub const VOICE_WAITING_TURN: &str = "voice_waiting_turn";
/// The failure of a voice note spoken with the hub away.
pub const VOICE_HUB_UNREACHABLE: &str = "voice_hub_unreachable";
/// The failure of a voice note spoken with this Mac not paired.
pub const VOICE_PAIRING_REQUIRED: &str = "voice_hub_pairing_required";

/// Whether a voice note that failed with `code` never reached the hub, so
/// sending it once the hub is back cannot repeat anything: the daemon sends
/// those by itself. A note the hub did try is sent again only when asked.
pub fn voice_note_waits_for_hub(code: &str) -> bool {
    matches!(
        code,
        VOICE_WAITING_TURN | VOICE_HUB_UNREACHABLE | VOICE_PAIRING_REQUIRED
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
    pub complete: bool,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceNote>,
    /// The files the user's message carried.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::attachments::Attachment>,
}

/// The turn in progress, or the last one until another begins: what
/// the user said and Deimos's answer as it arrives. It is all the Mac keeps of a
/// conversation; the conversation itself is the hub's, read a page at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub user: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Message>,
}

impl Turn {
    /// The message with this id, the user's or the answer.
    fn message_mut(&mut self, id: &str) -> Option<&mut Message> {
        if self.user.id == id {
            Some(&mut self.user)
        } else {
            self.answer.as_mut().filter(|answer| answer.id == id)
        }
    }

    /// The user's message, while it is a voice note with no transcript:
    /// what a later turn must not drop.
    pub fn untranscribed_voice_message(&self) -> Option<&Message> {
        Some(&self.user).filter(|message| {
            message
                .voice
                .as_ref()
                .is_some_and(|voice| voice.transcript.is_none())
        })
    }

    /// The user's voice note `id`, while it has no transcript yet.
    pub fn untranscribed_voice_mut(&mut self, id: &str) -> Option<&mut VoiceNote> {
        self.message_mut(id)
            .filter(|message| message.role == "user")
            .and_then(|message| message.voice.as_mut())
            .filter(|voice| voice.transcript.is_none())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceNote {
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<VoiceTranscriptionFailure>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub audio_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceTranscriptionFailure {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
}

impl VoiceTranscriptionFailure {
    pub fn local(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            provider_status: None,
            hub_status: None,
            timeout_ms: None,
            bytes: None,
            max_bytes: None,
            attempts: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub request_id: String,
    pub description: String,
    pub clarification: bool,
    /// The answers the agent offers, as the hub names them (`once`,
    /// `session`, `always`, `deny`; a clarification's suggestions). Empty:
    /// it named none, so `once` and `deny`, or free text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// A clarification of several questions, each answered by its id; the
    /// ones already answered have left.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    /// What runs deletes files: the hub then offers it one at a time
    /// (hub-v1 «Choices and questions»). `None`: the hub did not say so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletes: Option<Deletion>,
}

/// A request whose command deletes files; `paths`, how many it names, when
/// the hub could count them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deletion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<u32>,
}

impl Deletion {
    /// The hub's `deletes`, read leniently: an object is a deletion, and a
    /// count it cannot read is not known. Anything else says nothing.
    pub fn from_wire(value: Option<&serde_json::Value>) -> Option<Self> {
        let value = value?.as_object()?;
        Some(Self {
            paths: value
                .get("paths")
                .and_then(serde_json::Value::as_u64)
                .and_then(|paths| u32::try_from(paths).ok()),
        })
    }
}

/// One of several questions the agent asks at once (hub-v1 `questions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub qid: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_select: bool,
}

/// One conversation as the hub lists it. Its title is its date,
/// written by the app from `created_at_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    /// The agent it is with; absent from a hub without agents, and then
    /// the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub created_at_ms: i64,
    /// When its last message was written; its start while it has none.
    pub last_at_ms: i64,
    pub message_count: u64,
}

/// One message of a conversation as the hub keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessage {
    /// The Mac's id for it when the Mac sent it (the user's messages), so
    /// their turn and their voice note's audio are found by it; else the hub's.
    pub id: String,
    pub role: String,
    pub text: String,
    /// When it was written; `None` for messages stored before the hub kept
    /// the conversations.
    pub at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<SessionVoice>,
    /// The files the message carried, as the hub lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::attachments::Attachment>,
    /// An answer's summary of what its turn did; none from a hub up to 0.11.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<crate::agent_work::WorkSummary>,
}

/// A voice note in a conversation: its length, and whether this Mac still
/// holds its audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionVoice {
    pub duration_ms: u64,
    pub audio_available: bool,
}
