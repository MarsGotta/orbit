use super::*;

app_type! {
    /// A calendar occurrence across reads, including a moved occurrence.
    #[serde(deny_unknown_fields)]
    pub struct MeetingEventReference {
        pub calendar_item_id: String,
        #[ts(type = "number")]
        pub occurrence_start_ms: i64,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum MeetingPhase {
        Idle,
        Starting,
        Recording,
        Paused,
        Finishing,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum MeetingAudioKind {
        System,
        Application,
        Microphone,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum MeetingTrackName {
        MicLocal,
        RemoteApp,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum MeetingTrackLevel {
        Silent,
        Low,
        Ok,
    }
}

app_type! {
    pub struct MeetingTrackSnapshot {
        pub name: MeetingTrackName,
        pub level: MeetingTrackLevel,
        #[ts(type = "number")]
        pub chunks_uploaded: u64,
        #[ts(type = "number")]
        pub gaps: u64,
    }
}

app_type! {
    pub struct MeetingUploadSnapshot {
        #[ts(type = "number")]
        pub pending_chunks: u64,
        pub offline: bool,
    }
}

app_type! {
    /// What the floating panel has to say. Each state names what
    /// its button answers: the offer's `call_id` or the recording's id.
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum CallPanelSnapshot {
        /// «Llamada en {application}»: Grabar / Ahora no.
        Offer {
            call_id: String,
            application: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            event_title: Option<String>,
        },
        /// A recording that started without asking: Parar.
        Recording {
            recording_id: String,
            application: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            event_title: Option<String>,
        },
        /// «La llamada terminó · termino de grabar en N s»: Seguir grabando.
        /// `ends_at_ms` is Unix milliseconds; the app counts down to it.
        Ending {
            recording_id: String,
            application: String,
            #[ts(type = "number")]
            ends_at_ms: i64,
        },
    }
}

app_type! {
    /// How the user asked Deimos during the meeting.
    #[serde(rename_all = "snake_case")]
    pub enum MeetingQuestionOrigin {
        /// «Oye Deimos, …» on the user's microphone.
        Voice,
        /// «Preguntar a Deimos» in the pill or the panel.
        Typed,
        /// The dictation shortcut during the recording.
        Hotkey,
    }
}

app_type! {
    /// Where the question's turn is. Thinking (no text yet) and
    /// responding (text arriving) are two states, never one word.
    #[serde(rename_all = "snake_case")]
    pub enum MeetingQuestionStage {
        Thinking,
        Responding,
        Answered,
        Failed,
    }
}

app_type! {
    /// What was in progress when the user asked.
    #[serde(rename_all = "snake_case")]
    pub enum MeetingQuestionBusy {
        AssistantTurn,
        PendingApproval,
        VoiceNote,
        VoiceTranscription,
        Call,
    }
}

app_type! {
    /// The Mac's link to the hub when a question could not leave.
    #[serde(rename_all = "snake_case")]
    pub enum MeetingQuestionLink {
        Offline,
        Connecting,
        Online,
        Revoked,
    }
}

app_type! {
    /// Why a question did not get its answer, with its numbers.
    #[serde(tag = "cause", rename_all = "snake_case")]
    pub enum MeetingQuestionCause {
        PrivacyStopped,
        NoMeeting,
        HubUnreachable { connection: MeetingQuestionLink },
        DeimosUnavailable { code: String },
        TurnInFlight { turn: MeetingQuestionBusy },
        QuestionEmpty,
        QuestionTooLong {
            #[ts(type = "number")]
            bytes: u64,
            #[ts(type = "number")]
            max_bytes: u64,
        },
        WindowNotOffered { minutes: u16, offered: Vec<u8> },
        TurnFailed,
        Interrupted,
        HubRefused { code: String, message: String },
        NoAnswer,
        Revoked,
    }
}

app_type! {
    /// The user's question to Deimos during the meeting, as the floating
    /// panel shows it: the question, «Deimos está pensando…», the answer as
    /// it arrives, or the cause.
    pub struct MeetingQuestionSnapshot {
        /// The turn; «Cerrar» names it.
        pub id: String,
        pub question: String,
        pub minutes: u8,
        pub origin: MeetingQuestionOrigin,
        pub stage: MeetingQuestionStage,
        /// The answer's text, also while it arrives; it stays in the
        /// conversation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub answer: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub cause: Option<MeetingQuestionCause>,
        /// Until when the panel shows it (Unix ms); absent while Deimos has
        /// not finished.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "number | undefined", optional)]
        pub shown_until_ms: Option<i64>,
        /// The question reached Deimos as a turn; a refusal before sending
        /// never did.
        pub sent: bool,
    }
}

app_type! {
    /// One window of transcript «Preguntar a Deimos» offers. An
    /// object, not a bare number, so the generated goldens keep one shape.
    pub struct MeetingWindowSnapshot {
        pub minutes: u8,
    }
}

app_type! {
    /// What «Preguntar a Deimos» offers during a recording. The
    /// windows and the ceiling come from orbit-core; the interface never
    /// copies them.
    pub struct MeetingAskSnapshot {
        /// Minutes of transcript the button offers (2, 5, 10).
        pub windows: Vec<MeetingWindowSnapshot>,
        pub default_minutes: u8,
        /// The longest question, in UTF-8 bytes.
        pub max_question_bytes: u32,
    }
}

app_type! {
    /// Why «Oye Deimos» does not listen during the recording, with
    /// its numbers.
    #[serde(tag = "cause", rename_all = "snake_case")]
    pub enum VoiceOffCause {
        PrivacyStopped,
        Paused,
        MacosVersion {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            os_version: Option<String>,
        },
        LocaleUnsupported,
        Unavailable { code: String },
        Failed {
            code: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            detail: Option<String>,
        },
        HelperDisconnected,
        HelperWithoutPhrase,
        NoMicrophone,
    }
}

app_type! {
    /// Whether «Oye Deimos» listens during the recording, as the pill
    /// and the panel say it; absent outside a recording.
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum VoiceStatusSnapshot {
        Listening,
        Starting,
        Downloading {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            percent: Option<u8>,
        },
        NotListening { why: VoiceOffCause },
        /// It listens, and the phrase heard last was abandoned without a
        /// question: the panel says why and what to do, for a few seconds.
        Abandoned { cause: PhraseAbandonedCause },
    }
}

app_type! {
    /// Why «Oye Deimos» was heard and no question followed it.
    #[serde(rename_all = "snake_case")]
    pub enum PhraseAbandonedCause {
        /// The words heard changed and no longer opened with the phrase.
        PhraseRevised,
        /// No word of a question came within four pauses.
        QuestionWaitExpired,
        /// The sound went on for 30 s without a word of a question.
        QuestionTooLong,
    }
}

app_type! {
    pub struct MeetingApplicationSnapshot {
        pub name: String,
        pub pid: i32,
    }
}

app_type! {
    /// Why `applications` is empty, so the start dialog explains itself instead
    /// of offering a source that can never be chosen.
    #[serde(rename_all = "snake_case")]
    pub enum MeetingApplicationsState {
        /// At least one application with audio is listed.
        Ready,
        /// The Mac helper is not connected, so no inventory can be read.
        HelperMissing,
        /// Privacy is latched locally: no inventory is read until it is rearmed.
        PrivacyStopped,
        /// The helper answered and no running application has audio.
        Empty,
        /// The helper answered with an inventory that could not be verified;
        /// `applications_detail` says why, with its numbers. Not
        /// the same as `Empty`: nobody knows what is running.
        Unavailable,
    }
}

app_type! {
    pub struct MeetingTranscriptionSnapshot {
        pub pending: u32,
        pub running: u32,
        pub completed: u32,
        pub failed: u32,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum MeetingSummaryState {
        None,
        Pending,
        Ready,
    }
}

app_type! {
    pub struct RecentMeetingSnapshot {
        pub recording_id: String,
        pub title: String,
        #[ts(type = "number")]
        pub started_at_ms: i64,
        #[ts(type = "number")]
        pub duration_ms: i64,
        pub calendar_event_id: Option<String>,
        pub transcription: MeetingTranscriptionSnapshot,
        pub summary: MeetingSummaryState,
    }
}

app_type! {
    pub struct MeetingSnapshot {
        pub phase: MeetingPhase,
        pub recording_id: Option<String>,
        pub title: Option<String>,
        #[ts(type = "number | null")]
        pub started_at_ms: Option<i64>,
        /// The latest the recording in progress records until (Unix ms),
        /// the daemon's deadline («hasta las 15:40»): its start plus the
        /// safety ceiling, whatever started it. No calendar event moves it.
        #[ts(type = "number | null")]
        pub ends_at_ms: Option<i64>,
        /// The longest any recording may run, in minutes, said before one
        /// starts («Graba hasta 4 h»): orbit-core's
        /// `RECORDING_CEILING_MINUTES`, which the interface never copies.
        pub limit_minutes: u16,
        pub audio: MeetingAudioKind,
        pub tracks: Vec<MeetingTrackSnapshot>,
        pub upload: MeetingUploadSnapshot,
        /// What the floating panel says, if anything.
        pub call_panel: Option<CallPanelSnapshot>,
        /// «Al detectar una llamada»; `None` while not known.
        pub call_preference: Option<CallPreference>,
        /// Why the preference could not be read, when it could not.
        /// Absent with no preference: not published yet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub call_preference_unreadable: Option<String>,
        /// The last question to Deimos during the meeting, while the
        /// panel shows it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub question: Option<MeetingQuestionSnapshot>,
        /// «Preguntar a Deimos», while a recording is in progress.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub ask: Option<MeetingAskSnapshot>,
        /// Whether «Oye Deimos» listens during the recording, and
        /// why not; absent outside a recording.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub voice_trigger: Option<VoiceStatusSnapshot>,
        pub recent: Vec<RecentMeetingSnapshot>,
        pub applications: Vec<MeetingApplicationSnapshot>,
        pub applications_state: MeetingApplicationsState,
        /// Why the list is unavailable, with its numbers, when the state is
        /// `Unavailable`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub applications_detail: Option<String>,
    }
}

pub use orbit_core::{call_detection::CallPreference, protocol::MeetingAudio};
