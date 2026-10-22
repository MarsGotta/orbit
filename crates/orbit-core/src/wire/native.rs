//! Native helper v1 shapes. Parsing here is pure; the daemon owns the pipe and
//! correlates a response with the command it sent.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    calendar::{
        CalendarAuthorization, CalendarDelete, CalendarFailure, CalendarReadQuery, CalendarSave,
        CalendarStatus,
    },
    sources::{SelectedTextRange, WindowBinding},
};

pub const NATIVE_PROTOCOL: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDecodeProblem {
    pub event: &'static str,
    pub command: String,
    pub field: String,
    pub observed_type: &'static str,
}

impl std::fmt::Display for NativeDecodeProblem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} {} field={} type={}",
            self.event, self.command, self.field, self.observed_type
        )
    }
}

impl std::error::Error for NativeDecodeProblem {}

pub fn decode_result_body<T: DeserializeOwned>(
    command: &str,
    body: &Value,
) -> Result<T, NativeDecodeProblem> {
    let bytes = serde_json::to_vec(body).map_err(|_| NativeDecodeProblem {
        event: "response",
        command: command.to_owned(),
        field: ".".into(),
        observed_type: "unserializable",
    })?;
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
        use serde_path_to_error::Segment;
        let detail = error.inner().to_string();
        let missing = detail
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split_once('`'))
            .map(|(field, _)| field.to_owned());
        let mut field_value = Some(body);
        for segment in error.path() {
            field_value = match segment {
                Segment::Map { key } => field_value.and_then(|value| value.get(key)),
                Segment::Seq { index } => field_value.and_then(|value| value.get(*index)),
                Segment::Enum { .. } => field_value,
                Segment::Unknown => None,
            };
        }
        let observed_type = if missing.is_some() {
            "absent"
        } else {
            match field_value {
                Some(Value::Null) => "null",
                Some(Value::Bool(_)) => "boolean",
                Some(Value::Number(_)) => "number",
                Some(Value::String(_)) => "string",
                Some(Value::Array(_)) => "array",
                Some(Value::Object(_)) => "object",
                None => "absent",
            }
        };
        NativeDecodeProblem {
            event: "response",
            command: command.to_owned(),
            field: missing.unwrap_or_else(|| error.path().to_string()),
            observed_type,
        }
    })
}

macro_rules! result_struct {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        pub struct $name { $(pub $field: $ty,)* }
    };
}

/// The current consumer of a raw microphone stream, including a shared stream
/// whose meeting or voice note has just ended. Missing is unknown, never a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeMicrophonePurpose {
    Meeting,
    VoiceNote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", deny_unknown_fields)]
pub enum NativeCommand {
    #[serde(rename = "hello")]
    Hello { v: u8, id: String },
    #[serde(rename = "heartbeat")]
    Heartbeat { v: u8, id: String },
    #[serde(rename = "status")]
    Status { v: u8, id: String },
    #[serde(rename = "calendar_disconnect")]
    CalendarDisconnect { v: u8, id: String },
    #[serde(rename = "calendar_sources")]
    CalendarSources {
        v: u8,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        calendar_page: Option<CalendarPage>,
    },
    #[serde(rename = "calendar_snapshot")]
    CalendarSnapshot {
        v: u8,
        id: String,
        calendar_query: CalendarReadQuery,
    },
    /// Create an event or change an occurrence.
    #[serde(rename = "calendar_save")]
    CalendarSave {
        v: u8,
        id: String,
        calendar_save: CalendarSave,
    },
    /// Delete an occurrence, or it and the ones after it.
    #[serde(rename = "calendar_delete")]
    CalendarDelete {
        v: u8,
        id: String,
        calendar_delete: CalendarDelete,
    },
    #[serde(rename = "privacy_stop")]
    PrivacyStop { v: u8, id: String },
    #[serde(rename = "capture_stop")]
    CaptureStop { v: u8, id: String },
    #[serde(rename = "playback_stop")]
    PlaybackStop { v: u8, id: String },
    #[serde(rename = "rearm")]
    Rearm { v: u8, id: String },
    #[serde(rename = "shutdown")]
    Shutdown { v: u8, id: String },
    #[serde(rename = "permissions")]
    Permissions {
        v: u8,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<Vec<NativePermission>>,
    },
    #[serde(rename = "audio_start")]
    AudioStart {
        v: u8,
        id: String,
        stream_id: String,
        source: NativeAudioSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        purpose: Option<NativeMicrophonePurpose>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing: Option<NativeAudioProcessing>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        application_pid: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_scope: Option<NativeCaptureScope>,
    },
    /// Updates the consumer when a voice note shares a meeting's microphone.
    #[serde(rename = "audio_purpose")]
    AudioPurpose {
        v: u8,
        id: String,
        stream_id: String,
        purpose: NativeMicrophonePurpose,
    },
    /// Listen (or stop listening) for «Oye Deimos» on the live
    /// microphone stream `stream_id`. The meeting recording turns it on when
    /// it starts or resumes, even on a stream a voice note opened; pausing,
    /// finishing and the privacy stop turn it off, even while the stream
    /// stays open for the note.
    #[serde(rename = "audio_wake_phrase")]
    AudioWakePhrase {
        /// The default agent's name, which a helper reading only this hears.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_name: Option<String>,
        /// Every agent's name, each heard as «Oye <nombre>», «Hola <nombre>»
        /// and «<nombre>,»; it includes `assistant_name`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_names: Option<Vec<String>>,
        v: u8,
        id: String,
        stream_id: String,
        enabled: bool,
    },
    #[serde(rename = "audio_stop")]
    AudioStop {
        v: u8,
        id: String,
        stream_id: String,
    },
    #[serde(rename = "playback")]
    Playback {
        v: u8,
        id: String,
        playback_id: String,
        sequence: u64,
        sample_rate: u32,
        channels: u8,
        pcm_s16le_base64: String,
    },
    #[serde(rename = "playback_clip")]
    PlaybackClip {
        v: u8,
        id: String,
        playback_id: String,
        mime_type: String,
        provider: String,
        audio_base64: String,
    },
    #[serde(rename = "application_sources")]
    ApplicationSources {
        v: u8,
        id: String,
        bundle_ids: Vec<String>,
    },
    #[serde(rename = "screen_sources")]
    ScreenSources { v: u8, id: String },
    #[serde(rename = "screenshot")]
    Screenshot {
        v: u8,
        id: String,
        window_binding: WindowBinding,
    },
    #[serde(rename = "selected_text")]
    SelectedText {
        v: u8,
        id: String,
        window_binding: WindowBinding,
    },
    /// «Añadir carpeta»: the macOS panel, folders only. The person's choice
    /// there is the permission; the answer is the folder and its bookmark.
    #[serde(rename = "assistant_folder_pick")]
    AssistantFolderPick { v: u8, id: String },
    /// Opens each folder's bookmark and says whether it still opens; with
    /// `path`, also where that path leads (links and `..` followed).
    #[serde(rename = "assistant_folders")]
    AssistantFolders {
        v: u8,
        id: String,
        folders: Vec<crate::assistant_files::FolderBookmark>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// One folder's entries, by name, from `after_index`. `path` is the
    /// canonical path the folders admitted.
    #[serde(rename = "assistant_files_list")]
    AssistantFilesList {
        v: u8,
        id: String,
        folder: crate::assistant_files::FolderBookmark,
        path: String,
        after_index: u64,
        max_entries: u32,
    },
    /// Names, and with `content` the text of plain-text files, under each
    /// root, never following a link.
    #[serde(rename = "assistant_files_search")]
    AssistantFilesSearch {
        v: u8,
        id: String,
        roots: Vec<crate::assistant_files::SearchRoot>,
        query: String,
        content: bool,
        limits: crate::assistant_files::SearchLimits,
    },
    /// One file the folders admitted, read as `read` says.
    #[serde(rename = "assistant_files_read")]
    AssistantFilesRead {
        v: u8,
        id: String,
        folder: crate::assistant_files::FolderBookmark,
        path: String,
        read: crate::assistant_files::ReadAs,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarPage {
    pub after_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermission {
    Microphone,
    Screen,
    Accessibility,
    Calendars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAudioSource {
    MicLocal,
    RemoteApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAudioProcessing {
    Unprocessed,
    VoiceProcessing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCaptureScope {
    System,
    Application,
}

impl NativeCommand {
    /// Envelopes a typed order once. Reserved envelope keys in the supplied
    /// fields are errors, so no caller can silently replace its name or ID.
    pub fn from_parts(id: String, command: &str, fields: Value) -> Result<Self, String> {
        if id.is_empty() {
            return Err("El ID de la orden nativa está vacío".into());
        }
        let Value::Object(mut fields) = fields else {
            return Err("Los campos de la orden nativa deben ser un objeto JSON".into());
        };
        for reserved in ["v", "id", "command"] {
            if fields.contains_key(reserved) {
                return Err(format!("Campo reservado en la orden nativa: {reserved}"));
            }
        }
        fields.insert("v".into(), Value::from(NATIVE_PROTOCOL));
        fields.insert("id".into(), Value::from(id));
        fields.insert("command".into(), Value::from(command));
        serde_json::from_value(Value::Object(fields))
            .map_err(|error| format!("Orden nativa {command} inválida: {error}"))
    }

    pub fn names() -> &'static [&'static str] {
        &[
            "hello",
            "heartbeat",
            "status",
            "calendar_disconnect",
            "calendar_sources",
            "calendar_snapshot",
            "calendar_save",
            "calendar_delete",
            "privacy_stop",
            "capture_stop",
            "playback_stop",
            "rearm",
            "shutdown",
            "permissions",
            "audio_start",
            "audio_purpose",
            "audio_wake_phrase",
            "audio_stop",
            "playback",
            "playback_clip",
            "application_sources",
            "screen_sources",
            "screenshot",
            "selected_text",
            "assistant_folder_pick",
            "assistant_folders",
            "assistant_files_list",
            "assistant_files_search",
            "assistant_files_read",
        ]
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeError {
    pub code: String,
    pub message: String,
    /// The refusal's numbers and names, by code (the calendar write
    /// refusals). Absent from every other error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// The fields shared by every helper response. The bridge validates the
/// native-v1 event; the owner supplies the command before decoding its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeResponseFields {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<NativeError>,
}

impl NativeResponseFields {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NativeResponseId {
    pub id: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeEventHeader<'a> {
    pub event: &'a str,
}

impl<'a> NativeEventHeader<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeVersionHeader {
    pub v: u8,
}

impl NativeVersionHeader {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeResponseIdRef<'a> {
    pub id: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeResponseView<'a> {
    pub id: &'a str,
    pub ok: bool,
    pub result: Option<&'a Value>,
    pub error: Option<NativeErrorRef<'a>>,
}

impl<'a> NativeResponseView<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, NativeDecodeProblem> {
        fn problem(field: &str, value: Option<&Value>) -> NativeDecodeProblem {
            NativeDecodeProblem {
                event: "response",
                command: "unknown".into(),
                field: field.into(),
                observed_type: match value {
                    None => "absent",
                    Some(Value::Null) => "null",
                    Some(Value::Bool(_)) => "boolean",
                    Some(Value::Number(_)) => "number",
                    Some(Value::String(_)) => "string",
                    Some(Value::Array(_)) => "array",
                    Some(Value::Object(_)) => "object",
                },
            }
        }
        let map = value.as_object().ok_or_else(|| problem(".", Some(value)))?;
        let id_value = map.get("id");
        let id = id_value
            .and_then(Value::as_str)
            .ok_or_else(|| problem("id", id_value))?;
        let ok_value = map.get("ok");
        let ok = ok_value
            .and_then(Value::as_bool)
            .ok_or_else(|| problem("ok", ok_value))?;
        let error = match map.get("error") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let fields = value
                    .as_object()
                    .ok_or_else(|| problem("error", Some(value)))?;
                let code = fields.get("code");
                let message = fields.get("message");
                Some(NativeErrorRef {
                    code: code
                        .and_then(Value::as_str)
                        .ok_or_else(|| problem("error.code", code))?,
                    message: message
                        .and_then(Value::as_str)
                        .ok_or_else(|| problem("error.message", message))?,
                })
            }
        };
        Ok(Self {
            id,
            ok,
            result: map.get("result").filter(|result| !result.is_null()),
            error,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NativeErrorRef<'a> {
    pub code: &'a str,
    pub message: &'a str,
}

impl<'a> NativeResponseIdRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeCommandHeader<'a> {
    pub id: &'a str,
    pub command: &'a str,
}

impl<'a> NativeCommandHeader<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

/// Borrow the PCM text and frame identity while validating its numeric fields.
/// The daemon can inspect every audio frame without copying its base64 body.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeAudioFrameRef<'a> {
    pub source: &'a str,
    pub stream_id: &'a str,
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: u32,
    pub sequence: u64,
    pub monotonic_ns: u64,
    pub pcm_s16le_base64: &'a str,
    #[serde(default)]
    pub capture_epoch_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeAudioIdentityRef<'a> {
    pub source: &'a str,
    pub stream_id: &'a str,
}

impl<'a> NativeAudioIdentityRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

impl<'a> NativeAudioFrameRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeStateResult {
    pub privacy_stopped: bool,
    pub connected: bool,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub selected_text_available: bool,
    pub microphone: bool,
    pub system_audio: bool,
    pub playing: bool,
    pub supported_audio_processing: Vec<String>,
    pub voice_processing_available: bool,
    pub playback_reference_available: bool,
    pub audio_processing: NativeAudioProcessingState,
    #[serde(deserialize_with = "read_native_calendar_status")]
    pub calendar: CalendarStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain_complete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain: Option<String>,
}

#[derive(Deserialize)]
struct ReceivedCalendarStatus {
    authorization: CalendarAuthorization,
    physical_read_pending: bool,
    #[serde(default, deserialize_with = "native_present")]
    failure: Option<CalendarFailure>,
}

fn native_present<'de, D, T>(decoder: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(decoder).map(Some)
}

fn read_native_calendar_status<'de, D>(decoder: D) -> Result<CalendarStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let received = ReceivedCalendarStatus::deserialize(decoder)?;
    Ok(CalendarStatus {
        authorization: received.authorization,
        physical_read_pending: received.physical_read_pending,
        failure: received.failure,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct NativeCalendarStatusBody(
    #[serde(deserialize_with = "read_native_calendar_status")] pub CalendarStatus,
);

result_struct!(NativeAudioProcessingState {
    mode: String,
    enabled: bool,
    bypassed: bool,
    agc_enabled: bool,
    playback_reference: String,
    pipeline_id: String,
    engine_running: bool,
    render_sample_rate: u32,
});
result_struct!(PermissionsResult {
    microphone: String,
    screen: bool,
    accessibility: bool,
    calendars: String
});
result_struct!(SelectedTextResult {
    binding: WindowBinding,
    clock_domain: String,
    capture_started_monotonic_ns: u64,
    capture_completed_monotonic_ns: u64,
    mime_type: String,
    source: String,
    text: String,
    range: SelectedTextRange,
    ax_binding_quality: String,
    selection_scope: String,
    sensitivity: String
});

#[cfg(test)]
mod borrowed_header_tests {
    use super::*;

    #[test]
    fn event_and_response_headers_borrow_a_large_frame_without_copying_its_pcm()
    -> Result<(), Box<dyn std::error::Error>> {
        let frame = serde_json::json!({
            "event": "response", "id": "fixture", "ok": true,
            "result": {"pcm_s16le_base64": "A".repeat(128_000)}
        });
        let event = NativeEventHeader::from_value(&frame)?;
        let response = NativeResponseIdRef::from_value(&frame)?;
        let event_value = frame["event"].as_str().ok_or("event value absent")?;
        let id_value = frame["id"].as_str().ok_or("ID value absent")?;
        assert!(std::ptr::eq(event.event.as_ptr(), event_value.as_ptr()));
        assert!(std::ptr::eq(response.id.as_ptr(), id_value.as_ptr()));
        let view = NativeResponseView::from_value(&frame)?;
        assert!(std::ptr::eq(
            view.result.ok_or("result absent")?,
            &frame["result"]
        ));
        Ok(())
    }

    #[test]
    fn audio_projection_borrows_maximal_pcm_and_response_error_names_field_and_type()
    -> Result<(), Box<dyn std::error::Error>> {
        let frame = serde_json::json!({
            "source":"mic_local","stream_id":"track", "sample_rate":48_000,
            "channels":1,"frames":48_000,"sequence":1,"monotonic_ns":1,
            "pcm_s16le_base64":"A".repeat(128_000)
        });
        let audio = NativeAudioFrameRef::from_value(&frame)?;
        let pcm = frame["pcm_s16le_base64"].as_str().ok_or("PCM absent")?;
        assert!(std::ptr::eq(audio.pcm_s16le_base64.as_ptr(), pcm.as_ptr()));
        let malformed = serde_json::json!({"id":"r", "ok":"yes"});
        let problem = match NativeResponseView::from_value(&malformed) {
            Err(problem) => problem,
            Ok(_) => return Err("wrong ok type accepted".into()),
        };
        assert_eq!(
            (problem.event, problem.field.as_str(), problem.observed_type),
            ("response", "ok", "string")
        );
        Ok(())
    }
}
