//! What the helper sends on its own or in answer: every event by its name.
use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum NativeEvent {
    #[serde(rename = "ready")]
    Ready {
        v: u8,
        protocol: u8,
        platform: String,
        selected_text_available: bool,
        supported_audio_processing: Vec<String>,
        voice_processing_available: bool,
        playback_reference_available: bool,
        launch_mode: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture: Option<bool>,
    },
    #[serde(rename = "state")]
    State {
        v: u8,
        #[serde(flatten)]
        state: NativeStateResult,
    },
    #[serde(rename = "response")]
    Response {
        v: u8,
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<NativeError>,
    },
    #[serde(rename = "error")]
    Error {
        v: u8,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        os_domain: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        os_status: Option<i64>,
    },
    #[serde(rename = "hotkey")]
    Hotkey {
        v: u8,
        action: String,
        phase: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        applications: Option<Vec<MeetingApplication>>,
    },
    #[serde(rename = "privacy_stop")]
    PrivacyStop {
        v: u8,
        reason: String,
        privacy_stopped: bool,
        drain_complete: bool,
    },
    #[serde(rename = "capture_stopped")]
    CaptureStopped {
        v: u8,
        reason: String,
        privacy_stopped: bool,
        drain_complete: bool,
    },
    #[serde(rename = "local_rearm")]
    LocalRearm { v: u8, privacy_stopped: bool },
    #[serde(rename = "calendar_invalidated")]
    CalendarInvalidated { v: u8, code: String },
    #[serde(rename = "gap")]
    Gap {
        v: u8,
        source: String,
        stream_id: String,
        reason: String,
        frames: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_stop_possible: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_unknown: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ns: Option<u64>,
    },
    #[serde(rename = "audio")]
    Audio {
        v: u8,
        source: String,
        stream_id: String,
        sequence: u64,
        sample_rate: u32,
        channels: u8,
        frames: u32,
        monotonic_ns: u64,
        clock_domain: String,
        pcm_s16le_base64: String,
        processing: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing_enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing_bypassed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agc_enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        playback_reference: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pipeline_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        playback_active: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        playback_id: Option<String>,
    },
    #[serde(rename = "playback_progress")]
    PlaybackProgress {
        v: u8,
        playback_id: String,
        state: String,
        playback_active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frames: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rendered_frames: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remaining_frames: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        played_frames: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
    /// The applications holding the microphone now (sorted by process id,
    /// empty when none). The helper sends it once after `ready` and then only
    /// when the set changes; which of them is a call is decided in orbit-core.
    #[serde(rename = "microphone_activity")]
    MicrophoneActivity {
        v: u8,
        applications: Vec<MeetingApplication>,
    },
    /// The user said «Oye Deimos, …» on their own microphone while a
    /// recording was on: the question that followed the phrase, until a
    /// 1.5 s pause or 30 s, as text. Nothing else heard leaves the Mac.
    /// `stream_id` is the microphone stream it was heard on; the times are
    /// Unix milliseconds of the phrase's start and the question's end.
    #[serde(rename = "assistant_question")]
    AssistantQuestion {
        v: u8,
        stream_id: String,
        text: String,
        started_ms: i64,
        ended_ms: i64,
        /// The name heard, exactly as `audio_wake_phrase` gave it; absent
        /// from a helper that knows one name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_name: Option<String>,
    },
    /// Where the «Oye Deimos» detector is: `unavailable` (with
    /// `cause` and, for the system version, `os_version`), `idle`,
    /// `downloading` (with `percent`), `listening` or `failed` (with `cause`
    /// and Apple's `detail`).
    #[serde(rename = "voice_trigger")]
    VoiceTrigger {
        v: u8,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        os_version: Option<String>,
    },
    /// A step the helper reports that answers no order, as a code
    /// the daemon turns into a notice: `wake_phrase_heard`,
    /// `wake_phrase_abandoned`, `wake_phrase_question_dropped`. Never words
    /// that were said.
    #[serde(rename = "notice")]
    Notice {
        v: u8,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        abandonment: Option<WakePhraseAbandonment>,
    },
    #[serde(rename = "helper_disconnected")]
    HelperDisconnected {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakePhraseAbandonmentCause {
    PhraseRevised,
    QuestionWaitExpired,
    QuestionTooLong,
}

impl WakePhraseAbandonmentCause {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PhraseRevised => "phrase_revised",
            Self::QuestionWaitExpired => "question_wait_expired",
            Self::QuestionTooLong => "question_too_long",
        }
    }
}

/// Audio timeline durations only; recognized words never belong in a notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakePhraseAbandonment {
    pub cause: WakePhraseAbandonmentCause,
    pub elapsed_ms: u64,
    pub silent_ms: u64,
}

result_struct!(MeetingApplication {
    application_pid: i32,
    bundle_id: String,
    name: String
});
impl NativeEvent {
    pub fn names() -> &'static [&'static str] {
        &[
            "ready",
            "state",
            "response",
            "error",
            "hotkey",
            "privacy_stop",
            "capture_stopped",
            "local_rearm",
            "calendar_invalidated",
            "gap",
            "audio",
            "playback_progress",
            "microphone_activity",
            "assistant_question",
            "voice_trigger",
            "notice",
            "helper_disconnected",
        ]
    }
}
