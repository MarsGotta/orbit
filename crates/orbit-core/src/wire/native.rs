//! Native helper v1 shapes. Parsing here is pure; the daemon owns the pipe and
//! correlates a response with the command it sent.
//!
//! This file holds the commands Orbit sends; `event` is what the helper sends,
//! `frame_views` reads a frame's envelope without decoding it whole, and
//! `results` the bodies of the helper's answers.
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

// After `result_struct!`, which the pieces use.
mod event;
mod frame_views;
mod results;

pub use event::{
    MeetingApplication, NativeEvent, WakePhraseAbandonment, WakePhraseAbandonmentCause,
};
pub use frame_views::{
    NativeAudioFrameRef, NativeAudioIdentityRef, NativeCommandHeader, NativeError, NativeErrorRef,
    NativeEventHeader, NativeResponseFields, NativeResponseId, NativeResponseIdRef,
    NativeResponseView, NativeVersionHeader,
};
pub use results::{
    NativeAudioProcessingState, NativeCalendarStatusBody, NativeStateResult, PermissionsResult,
    SelectedTextResult,
};

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
    #[serde(rename = "playback_pause")]
    PlaybackPause { v: u8, id: String },
    #[serde(rename = "playback_resume")]
    PlaybackResume { v: u8, id: String },
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
        /// A microphone that starts muted (a meeting resumed with «Silenciar»
        /// on): the helper sends silence from its first frame and says
        /// `muted:true` in its answer. Absent: not muted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },
    /// «Silenciar»: the helper replaces the microphone stream's samples with
    /// silence before they leave it (same frames, same timing), or stops
    /// doing so, and answers with the state it applied.
    #[serde(rename = "audio_mute")]
    AudioMute {
        v: u8,
        id: String,
        stream_id: String,
        muted: bool,
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
    /// «Adjuntar ventana» without the engine: the text Accessibility shows
    /// in the window just captured, read within `limits`. The answer is a
    /// `window_text::WindowTextReply`; its text is never logged.
    #[serde(rename = "window_text")]
    WindowText {
        v: u8,
        id: String,
        application_pid: i32,
        window_id: u32,
        limits: crate::window_text::WindowTextLimits,
    },
    /// A photo no agent reads (an iPhone's HEIC) written as JPEG at
    /// `destination_path`, a file of the daemon's, before it is uploaded.
    #[serde(rename = "photo_jpeg")]
    PhotoJpeg {
        v: u8,
        id: String,
        source_path: String,
        destination_path: String,
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
            return Err("The native command ID is empty".into());
        }
        let Value::Object(mut fields) = fields else {
            return Err("Native command fields must be a JSON object".into());
        };
        for reserved in ["v", "id", "command"] {
            if fields.contains_key(reserved) {
                return Err(format!("Reserved field in native command: {reserved}"));
            }
        }
        fields.insert("v".into(), Value::from(NATIVE_PROTOCOL));
        fields.insert("id".into(), Value::from(id));
        fields.insert("command".into(), Value::from(command));
        serde_json::from_value(Value::Object(fields))
            .map_err(|error| format!("Invalid native command {command}: {error}"))
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
            "playback_pause",
            "playback_resume",
            "rearm",
            "shutdown",
            "permissions",
            "audio_start",
            "audio_purpose",
            "audio_mute",
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
            "window_text",
            "photo_jpeg",
        ]
    }
}

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
