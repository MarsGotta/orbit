use super::*;

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum ChatRole {
        User,
        Assistant,
        System,
    }
}

app_type! {
    /// One conversation as the hub lists it. Its title is its
    /// date, which the interface writes from `created_at_ms`.
    pub struct ChatSessionSummary {
        pub id: String,
        /// The agent it is with; absent: the hub's default agent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub agent_id: Option<String>,
        #[ts(type = "number")]
        pub created_at_ms: i64,
        #[ts(type = "number")]
        pub last_at_ms: i64,
        #[ts(type = "number")]
        pub message_count: u64,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum AssistantActivity {
        Transcribing,
        Thinking,
        Responding,
    }
}

app_type! {
    pub struct VoiceNoteAudioSnapshot {
        pub message_id: String,
        pub format: String,
        pub bytes_base64: String,
    }
}

app_type! {
    pub struct VoiceTranscriptionFailure {
        pub code: String,
        pub provider_status: Option<u16>,
        pub hub_status: Option<u16>,
        #[ts(type = "number | null")]
        pub timeout_ms: Option<u64>,
        pub bytes: Option<String>,
        pub max_bytes: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub attempts: Option<u32>,
    }
}

app_type! {
    pub struct VoiceNote {
        #[ts(type = "number")]
        pub duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub transcript: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub failure: Option<VoiceTranscriptionFailure>,
        #[serde(default, skip_serializing_if = "is_false")]
        pub audio_available: bool,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum ReceiptKind {
        Read,
        Written,
        Updated,
        Attached,
        Reminder,
    }
}

app_type! {
    pub struct ToolReceipt {
        pub kind: ReceiptKind,
        pub label: String,
        pub path: String,
    }
}

app_type! {
    pub struct ChatMessage {
        pub id: String,
        pub role: ChatRole,
        pub text: String,
        #[ts(type = "number")]
        pub at_ms: i64,
        pub streaming: bool,
        pub voice: Option<VoiceNote>,
        pub receipts: Vec<ToolReceipt>,
        /// The files the user's message carried.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<orbit_core::attachments::Attachment>>", optional)]
        pub attachments: Vec<orbit_core::attachments::Attachment>,
    }
}

app_type! {
    /// What the agent waits for, with the answers it offers: an
    /// approval's `choices` (`once`, `session`, `always`, `deny`; empty is
    /// `once` and `deny`), a clarification's suggestions and, when it asks
    /// several things at once, each question with its id.
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum ChatPending {
        Approval {
            id: String,
            text: String,
            choices: Vec<String>,
        },
        Clarify {
            id: String,
            text: String,
            choices: Vec<String>,
            questions: Vec<ChatQuestion>,
        },
    }
}

app_type! {
    /// One of several questions; it is answered by `qid`.
    pub struct ChatQuestion {
        pub qid: String,
        pub question: String,
        pub choices: Vec<String>,
        pub multi_select: bool,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum VoiceState {
        Unknown,
        Idle,
        Listening,
        Transcribing,
    }
}

app_type! {
    pub struct VoiceSnapshot {
        pub state: VoiceState,
        pub level: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub availability: Option<crate::features::FeatureAvailability>,
        /// The last voice note the daemon did not send for being shorter
        /// than its speech minimum: the composer says both numbers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub too_short: Option<ShortVoiceNote>,
    }
}

app_type! {
    pub struct ShortVoiceNote {
        #[ts(type = "number")]
        pub duration_ms: u64,
        #[ts(type = "number")]
        pub minimum_ms: u64,
    }
}

app_type! {
    pub struct ChatSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub availability: Option<crate::features::FeatureAvailability>,
        /// The open conversation; its messages are read from the hub a page
        /// at a time.
        pub session_id: Option<String>,
        /// The agent the open conversation is with; absent: the default one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub agent_id: Option<String>,
        /// The turn in progress, or the last one until another begins:
        /// the user's message and the answer as it arrives.
        pub turn: Vec<ChatMessage>,
        /// The last voice note the user asked to hear, once its audio arrived.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub voice_audio: Option<VoiceNoteAudioSnapshot>,
        #[serde(rename = "pending")]
        pub pending_contract: Option<ChatPending>,
        pub voice: VoiceSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub activity: Option<AssistantActivity>,
        /// The answer being read aloud («Escuchar»), by its message id,
        /// until the Mac's speaker has played it or the user stopped it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub speaking: Option<String>,
        /// What the agent did in the turn in progress, or the last one:
        /// its steps, plan and reasoning, and when it started and ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub work: Option<orbit_core::agent_work::AgentWork>,
    }
}

app_type! {
    pub struct CallSnapshot {
        #[ts(type = "number")]
        pub started_at_ms: i64,
        pub seeing_screen: bool,
        pub last_line: Option<String>,
        pub speaking: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub voice_unavailable: bool,
    }
}
