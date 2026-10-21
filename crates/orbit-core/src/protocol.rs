use crate::{
    reads::ReadRequest,
    topics::{TopicFrame, TopicId},
};
use serde::{Deserialize, Serialize};

pub use crate::features::FeatureId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MeetingAudio {
    #[default]
    System,
    Application {
        application_pid: i32,
    },
    MicrophoneOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    #[cfg_attr(feature = "ts", ts(skip))]
    Status,
    #[cfg_attr(feature = "ts", ts(skip))]
    SubscribeTopics {
        topics: Vec<TopicId>,
    },
    #[cfg_attr(feature = "ts", ts(skip))]
    Read {
        read: ReadRequest,
    },
    Pair {
        hub_url: String,
        code: String,
        device_name: String,
    },
    SessionOpen {
        id: String,
    },
    TranscriptRetranscribe {
        recording_id: String,
    },
    TranscriptRetry {
        recording_id: String,
    },
    /// Mars's private notes of one recording: kept in the daemon's outbox
    /// until the hub confirms them (SIM-41). `base_revision` is the hub's
    /// revision the app was showing. Allowed while recording.
    #[cfg_attr(feature = "ts", ts(skip))]
    NotePrivateSave {
        recording_id: String,
        markdown: String,
        #[serde(default)]
        base_revision: u64,
    },
    /// Mars changed the note's sections or a task, on the hub's revision
    /// `base_revision`. Absent fields stay as they are; `sections` replaces
    /// the whole list.
    #[cfg_attr(feature = "ts", ts(skip))]
    NoteEdit {
        recording_id: String,
        #[serde(default)]
        sections: Option<Vec<crate::notes::Section>>,
        #[serde(default)]
        done: std::collections::BTreeMap<String, bool>,
        base_revision: u64,
    },
    /// Write the note again with this template.
    NoteRegenerate {
        recording_id: String,
        template_id: String,
    },
    /// Bring back the text edits a regeneration set aside.
    NoteRestoreSuperseded {
        recording_id: String,
    },
    /// The name of one speaker of a recording, by the hub's key; `None`
    /// gives it back its default name.
    #[cfg_attr(feature = "ts", ts(skip))]
    SpeakerRename {
        recording_id: String,
        key: String,
        name: Option<String>,
        #[serde(default)]
        scope: crate::notes::SpeakerRenameScope,
    },
    /// SIM-52: transcribe the meeting in this language (ISO 639-1, «en»),
    /// and again what is already transcribed; the hub writes the note again.
    MeetingLanguage {
        recording_id: String,
        language: String,
    },
    /// SIM-52: Mars's name for the meeting, everywhere it is named; an empty
    /// one gives it back its own.
    MeetingRename {
        recording_id: String,
        title: String,
    },
    /// P9-05: Mars's templates and their assignment to a series. Allowed
    /// while recording.
    NoteLibrary {
        request: crate::notes::LibraryRequest,
    },
    Chat {
        text: String,
    },
    AudioSourcesRefresh,
    /// P9-11 «Grabar» on the call the floating panel offers, naming the
    /// offer it answers. The press is the decision.
    CallRecord {
        call_id: String,
    },
    /// «Ahora no»: this call is not offered again until its application
    /// leaves the microphone. An offer already gone is not an error.
    CallDismiss {
        call_id: String,
    },
    /// «Seguir grabando»: a recording the call started no longer ends with
    /// the call; it is a manual recording from now on.
    RecordingKeep {
        recording_id: String,
    },
    /// P9-12 «Preguntar a Deimos» during a recording, typed in the pill or
    /// the floating panel: a turn to Deimos with the meeting and its last
    /// `minutes` of transcript (2, 5 or 10; 2 by default). The receipt says
    /// the question was taken; whether it reached Deimos, and its answer or
    /// its cause, is in `recording.question`.
    MeetingAsk {
        question: String,
        #[serde(default = "default_meeting_minutes")]
        minutes: u16,
    },
    /// P9-12: Mars closed the question in the floating panel. A question
    /// already gone is not an error.
    MeetingQuestionClose {
        id: String,
    },
    /// «Al detectar una llamada»: Mars's preference, kept by the daemon.
    CallPreference {
        preference: crate::call_detection::CallPreference,
    },
    /// SIM-23 «Enviar informes de errores anónimos»: her choice, kept by the
    /// daemon with its settings. Off until she turns it on.
    ErrorReports {
        enabled: bool,
    },
    /// SIM-23: a line the interface wrote to the app's report log, handed to
    /// the daemon so its one error-report client decides whether it leaves.
    /// The type admits only codes, numbers and closed tokens.
    #[cfg_attr(feature = "ts", ts(skip))]
    Report {
        report: crate::notice::Report,
    },
    Calendar {
        request: crate::calendar_control::CalendarCommand,
    },
    NewSession,
    Interrupt,
    PrivacyStop,
    Rearm,
    VoiceStart,
    VoiceEnd,
    #[cfg_attr(feature = "ts", ts(skip))]
    VoiceNoteAudio {
        message_id: String,
    },
    CallStart {
        duration_minutes: u16,
    },
    CallStop,
    #[cfg_attr(feature = "ts", ts(skip))]
    MeetingStart {
        title: String,
        #[serde(default)]
        audio: MeetingAudio,
        duration_minutes: u16,
        /// P9-11: the calendar event the recording belongs to, by its stable
        /// reference («Grabar» on an event). Absent: not linked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<crate::call_detection::LinkedEvent>,
    },
    MeetingPause,
    MeetingResume,
    MeetingFinish,
    Approval {
        approved: bool,
    },
    Clarify {
        answer: String,
    },
    Permissions {
        request: Vec<String>,
    },
    #[cfg_attr(feature = "ts", ts(skip))]
    Shutdown,
}

impl Command {
    /// Internal transport and lifecycle orders have no interface feature.
    pub fn feature(&self) -> Option<FeatureId> {
        match self {
            Self::Pair { .. } | Self::ErrorReports { .. } | Self::PrivacyStop | Self::Rearm => {
                Some(FeatureId::Status)
            }
            Self::TranscriptRetranscribe { .. }
            | Self::TranscriptRetry { .. }
            | Self::NotePrivateSave { .. }
            | Self::NoteEdit { .. }
            | Self::NoteRegenerate { .. }
            | Self::NoteRestoreSuperseded { .. }
            | Self::SpeakerRename { .. }
            | Self::MeetingLanguage { .. }
            | Self::MeetingRename { .. } => Some(FeatureId::Note),
            Self::NoteLibrary { .. } => Some(FeatureId::Notes),
            Self::AudioSourcesRefresh
            | Self::CallRecord { .. }
            | Self::CallDismiss { .. }
            | Self::RecordingKeep { .. }
            | Self::MeetingAsk { .. }
            | Self::MeetingQuestionClose { .. }
            | Self::CallPreference { .. }
            | Self::MeetingStart { .. }
            | Self::MeetingPause
            | Self::MeetingResume
            | Self::MeetingFinish => Some(FeatureId::Recording),
            Self::Calendar { .. } => Some(FeatureId::Agenda),
            Self::Permissions { .. } => Some(FeatureId::Permissions),
            Self::SessionOpen { .. }
            | Self::Chat { .. }
            | Self::NewSession
            | Self::Interrupt
            | Self::VoiceStart
            | Self::VoiceEnd
            | Self::VoiceNoteAudio { .. }
            | Self::CallStart { .. }
            | Self::CallStop
            | Self::Approval { .. }
            | Self::Clarify { .. } => Some(FeatureId::Assistant),
            Self::Status
            | Self::SubscribeTopics { .. }
            | Self::Read { .. }
            | Self::Report { .. }
            | Self::Shutdown => None,
        }
    }
}

/// Empty recording titles use the local offset observed at `now_ms`.
/// An unknown zone cannot supply a local hour.
pub fn recording_title(title: String, now_ms: i64, utc_offset_ms: Option<i64>) -> String {
    if !title.trim().is_empty() {
        return title;
    }
    let Some(offset) = utc_offset_ms else {
        return "Reunión".into();
    };
    let minute = now_ms.saturating_add(offset).rem_euclid(86_400_000) / 60_000;
    format!("Reunión {:02}:{:02}", minute / 60, minute % 60)
}

fn default_meeting_minutes() -> u16 {
    u16::from(crate::meeting_question::DEFAULT_WINDOW_MINUTES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireRefusal {
    #[error("Orden IPC de {bytes} bytes; máximo {max_bytes} bytes")]
    Budget { bytes: usize, max_bytes: usize },
}

impl Command {
    /// Applies the command-specific gates before actor admission. The byte count
    /// includes the request envelope and its trailing newline.
    pub fn wire_gate(&self, bytes: usize) -> Result<(), WireRefusal> {
        let budget = match self {
            Self::SubscribeTopics { .. } | Self::Read { .. } => {
                crate::topics::transport_max_bytes()
            }
            _ => crate::limits::MAX_DEFAULT_COMMAND_BYTES,
        };
        if bytes > budget {
            return Err(WireRefusal::Budget {
                bytes,
                max_bytes: budget,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub v: u8,
    pub id: String,
    /// The action this order belongs to, from end to end (SIM-21). The app
    /// always sends one; a request without it gets a new one in the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<crate::notice::Op>,
    pub body: Command,
}

/// Small startup/status facts; the app reads views through topics and keyed reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub recovery: crate::RecoveryStatus,
    pub privacy_stopped: bool,
    pub privacy_stop_by: Option<crate::privacy_stop::StopRequester>,
    pub generation: u64,
}

impl From<&crate::State> for DaemonStatus {
    fn from(state: &crate::State) -> Self {
        Self {
            recovery: state.views.recovery.clone(),
            privacy_stopped: state.privacy_stopped(),
            privacy_stop_by: state.privacy_stop_by(),
            generation: state.generation(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub v: u8,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<DaemonStatus>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<crate::refusal::Refusal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisions: Option<std::collections::BTreeMap<TopicId, u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceNoteAudio {
    pub message_id: String,
    pub format: String,
    pub bytes_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceNoteAudioResponse {
    pub v: u8,
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub audio: Option<VoiceNoteAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<crate::refusal::Refusal>,
}

/// A topic subscription receipt or a keyed read answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicResponse {
    pub v: u8,
    pub id: String,
    pub ok: bool,
    pub revisions: std::collections::BTreeMap<TopicId, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<crate::reads::ReadId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<crate::topics::Knowledge<crate::topics::Page<serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<crate::refusal::Refusal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum TopicEvent {
    #[serde(rename = "topic.changed")]
    Changed { frame: TopicFrame },
}

#[cfg(test)]
mod topic_contract_tests {
    use super::Command;

    #[test]
    fn topic_subscription_has_its_own_wire_command() {
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "command": "subscribe_topics",
            "topics": ["status", "permissions", "recording"]
        }));
        assert!(command.is_ok(), "topic subscription rejected: {command:?}");
    }

    #[test]
    fn keyed_read_has_its_own_wire_command() {
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "command": "read",
            "read": {"kind": "notes_page", "cursor": null}
        }));
        assert!(command.is_ok(), "keyed read rejected: {command:?}");
    }
}
