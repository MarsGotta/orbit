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
    /// An earlier conversation, with the agent the hub listed it with
    /// (absent: the hub's default agent, as every conversation before agents).
    SessionOpen {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_id: Option<String>,
    },
    TranscriptRetranscribe {
        recording_id: String,
    },
    TranscriptRetry {
        recording_id: String,
    },
    /// The user's private notes of one recording: kept in the daemon's outbox
    /// until the hub confirms them. `base_revision` is the hub's
    /// revision the app was showing. Allowed while recording.
    #[cfg_attr(feature = "ts", ts(skip))]
    NotePrivateSave {
        recording_id: String,
        markdown: String,
        #[serde(default)]
        base_revision: u64,
    },
    /// The user changed the note's sections or a task, on the hub's revision
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
    /// Transcribe the meeting in this language (ISO 639-1, «en»),
    /// and again what is already transcribed; the hub writes the note again.
    MeetingLanguage {
        recording_id: String,
        language: String,
    },
    /// The user's name for the meeting, everywhere it is named; an empty
    /// one gives it back its own.
    MeetingRename {
        recording_id: String,
        title: String,
    },
    /// «Borrar reunión», confirmed: the meeting and all made from it leave
    /// this Mac and the hub; its series' dictionaries stay.
    MeetingDelete {
        recording_id: String,
    },
    /// The user's templates and their assignment to a series. Allowed
    /// while recording.
    NoteLibrary {
        request: crate::notes::LibraryRequest,
    },
    /// A turn to the open conversation's agent. `attachments`: the files
    /// the hub already holds for it (`AttachmentAdd`), sent by their ids;
    /// with them the text may be empty. `primary`: asked where nobody is
    /// named («Preguntar» in a note), so it goes to the primary agent: in
    /// the open conversation when it is the primary's, else in a new one.
    Chat {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            feature = "ts",
            ts(as = "Option<Vec<crate::attachments::Attachment>>", optional)
        )]
        attachments: Vec<crate::attachments::Attachment>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        primary: bool,
    },
    /// «Adjuntar», a file dropped on the window or a pasted image or text:
    /// the daemon reads it (by path) and uploads it to the hub, off its loop,
    /// and answers with the attachment (`AttachmentResponse`). Nothing is
    /// sent to the agent until a `Chat` names it.
    #[cfg_attr(feature = "ts", ts(skip))]
    AttachmentAdd {
        source: crate::attachments::AttachmentSource,
    },
    /// One attachment's bytes, for the chat's viewer.
    #[cfg_attr(feature = "ts", ts(skip))]
    AttachmentFetch {
        attachment_id: String,
    },
    /// «Abrir» and «Mostrar en Finder»: the attachment written to this Mac's
    /// attachment folder; the answer carries the file's path.
    #[cfg_attr(feature = "ts", ts(skip))]
    AttachmentSave {
        attachment_id: String,
    },
    AudioSourcesRefresh,
    /// «Grabar» on the call the floating panel offers, naming the
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
    /// «Preguntar a Deimos» during a recording, typed in the pill or
    /// the floating panel: a turn to Deimos with the meeting and its last
    /// `minutes` of transcript (2, 5 or 10; 2 by default). The receipt says
    /// the question was taken; whether it reached Deimos, and its answer or
    /// its cause, is in `recording.question`.
    MeetingAsk {
        question: String,
        #[serde(default = "default_meeting_minutes")]
        minutes: u16,
    },
    /// The user closed the question in the floating panel. A question
    /// already gone is not an error.
    MeetingQuestionClose {
        id: String,
    },
    /// The pointer entered (`held`) or left the floating panel showing
    /// question `id`: its deadline waits while it is held. A question already
    /// gone is not an error.
    MeetingQuestionHold {
        id: String,
        held: bool,
    },
    /// «Recordatorio personal»: an edit of meeting `key`'s reminder (its
    /// title's digest), written at `edited_at_ms`. Answered once the hub kept
    /// it, or a later one: the latest edit wins.
    ReminderWrite {
        key: String,
        text: String,
        edited_at_ms: i64,
    },
    /// «Al detectar una llamada»: the user's preference, kept by the daemon.
    CallPreference {
        preference: crate::call_detection::CallPreference,
    },
    /// «Añadir carpeta»: the helper opens the macOS panel, folders only. The
    /// folder the person chooses there is the assistant's to read; choosing
    /// none changes nothing.
    AssistantFolderAdd,
    /// «Quitar»: the assistant no longer reads this folder. One already gone
    /// is not an error.
    AssistantFolderRemove {
        id: String,
    },
    /// Ajustes › Agentes: one read or order of an agent's configuration on
    /// the hub, answered with its outcome (`AgentResponse`), never queued.
    #[cfg_attr(feature = "ts", ts(skip))]
    Agent {
        call: crate::agent_config::AgentCall,
    },
    /// «Enviar informes de errores anónimos»: the user's choice, kept by the
    /// daemon with its settings. Off until they turn it on.
    ErrorReports {
        enabled: bool,
    },
    /// A line the interface wrote to the app's report log, handed to
    /// the daemon so its one error-report client decides whether it leaves.
    /// The type admits only codes, numbers and closed tokens.
    #[cfg_attr(feature = "ts", ts(skip))]
    Report {
        report: crate::notice::Report,
    },
    Calendar {
        request: crate::calendar_control::CalendarCommand,
    },
    /// A new conversation with one of the hub's agents (absent: its default).
    NewSession {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_id: Option<String>,
    },
    /// «Seguir con {nombre}»: a new conversation with `agent_id` that takes
    /// over `session_id`, the hub's conversation whose agent could not
    /// answer, with `text`, the question left unanswered (hub-v1 «Handing a
    /// conversation over»). Only to a hub that lists `addable_agents`.
    Handover {
        session_id: String,
        agent_id: String,
        text: String,
    },
    Interrupt,
    PrivacyStop,
    Rearm,
    VoiceStart,
    VoiceEnd,
    /// Esc in the composer: the voice note in progress ends without being
    /// sent.
    VoiceCancel,
    /// «Reintentar transcripción»: the open turn's voice note whose
    /// transcription failed is transcribed again from the audio this Mac
    /// keeps.
    VoiceRetry {
        message_id: String,
    },
    /// «Descartar», confirmed: a voice note whose transcription failed
    /// leaves the conversation, and its audio leaves this Mac.
    VoiceDiscard {
        message_id: String,
    },
    /// «Escuchar» on an answer: the hub's voice reads `text`, the answer
    /// `message_id` as shown, through the Mac's speaker. One at a time: a new
    /// one replaces what is being read.
    Speak {
        message_id: String,
        text: String,
    },
    /// «Parar»: the answer being read aloud stops. Nothing being read is
    /// not an error.
    SpeakStop,
    #[cfg_attr(feature = "ts", ts(skip))]
    VoiceNoteAudio {
        message_id: String,
    },
    /// A call from the app. The retired duration argument is ignored.
    CallStart {
        #[serde(default = "default_call_minutes")]
        #[cfg_attr(feature = "ts", ts(as = "Option<u16>", optional))]
        duration_minutes: u16,
    },
    CallStop,
    CallKeep,
    MeetingKeep,
    #[cfg_attr(feature = "ts", ts(skip))]
    MeetingStart {
        title: String,
        #[serde(default)]
        audio: MeetingAudio,
        /// Retired duration argument, accepted and ignored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_minutes: Option<u16>,
        /// The calendar event the recording belongs to, by its stable
        /// reference («Grabar» on an event). Absent: not linked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<crate::call_detection::LinkedEvent>,
    },
    MeetingPause,
    MeetingResume,
    MeetingFinish,
    /// «Terminar sin guardar», confirmed: the recording ends, recording or
    /// paused, and nothing of it stays here or on the hub.
    MeetingDiscard {
        recording_id: String,
    },
    /// «Silenciar» (`muted`) or «Reactivar micrófono»: the microphone of the
    /// recording or of the call with the assistant. The other side of a
    /// meeting keeps recording; the app shows it muted once the Mac says so.
    MicrophoneMute {
        muted: bool,
    },
    Approval {
        approved: bool,
        /// Which of the request's choices; absent, `once` or `deny`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        choice: Option<String>,
    },
    Clarify {
        answer: String,
        /// The question answered, when the agent asked several.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        question_id: Option<String>,
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
            | Self::MeetingRename { .. }
            | Self::MeetingDelete { .. } => Some(FeatureId::Note),
            Self::NoteLibrary { .. } | Self::ReminderWrite { .. } => Some(FeatureId::Notes),
            Self::AudioSourcesRefresh
            | Self::CallRecord { .. }
            | Self::CallDismiss { .. }
            | Self::RecordingKeep { .. }
            | Self::MeetingAsk { .. }
            | Self::MeetingQuestionClose { .. }
            | Self::MeetingQuestionHold { .. }
            | Self::CallPreference { .. }
            | Self::MeetingStart { .. }
            | Self::MeetingPause
            | Self::MeetingResume
            | Self::MeetingKeep
            | Self::MeetingFinish
            | Self::MeetingDiscard { .. }
            | Self::MicrophoneMute { .. } => Some(FeatureId::Recording),
            Self::Calendar { .. } => Some(FeatureId::Agenda),
            Self::AssistantFolderAdd | Self::AssistantFolderRemove { .. } => {
                Some(FeatureId::Control)
            }
            Self::Permissions { .. } => Some(FeatureId::Permissions),
            Self::SessionOpen { .. }
            | Self::Chat { .. }
            | Self::AttachmentAdd { .. }
            | Self::AttachmentFetch { .. }
            | Self::AttachmentSave { .. }
            | Self::NewSession { .. }
            | Self::Handover { .. }
            | Self::Interrupt
            | Self::VoiceStart
            | Self::VoiceEnd
            | Self::VoiceCancel
            | Self::VoiceRetry { .. }
            | Self::VoiceDiscard { .. }
            | Self::Speak { .. }
            | Self::SpeakStop
            | Self::VoiceNoteAudio { .. }
            | Self::CallStart { .. }
            | Self::CallKeep
            | Self::CallStop
            | Self::Approval { .. }
            | Self::Clarify { .. } => Some(FeatureId::Assistant),
            // Reaches the daemon only through its own call, which waits
            // for the hub's answer (`LocalClient::agent`).
            Self::Status
            | Self::SubscribeTopics { .. }
            | Self::Read { .. }
            | Self::Agent { .. }
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

fn default_call_minutes() -> u16 {
    0
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
    /// The action this order belongs to, from end to end. The app
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

/// The answer to `Command::Agent`: what came of the call on the hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub v: u8,
    pub id: String,
    pub outcome: crate::agent_config::AgentOutcome,
}

/// The answer to an attachment order, outside `State`: the attachment the
/// hub keeps (`AttachmentAdd`), its bytes (`AttachmentFetch`) or the path it
/// was written to (`AttachmentSave`).
#[derive(Debug, Serialize, Deserialize)]
pub struct AttachmentResponse {
    pub v: u8,
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<crate::refusal::Refusal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<crate::attachments::Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<AttachmentContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// «Adjuntar ventana»: what became of the window's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_text: Option<crate::attachments::WindowText>,
}

/// An attachment's bytes as the hub returned them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentContent {
    /// The file's name, as the hub keeps it.
    pub name: String,
    pub mime: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

mod base64_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
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

    /// The interface starts a call without saying its length: the app's
    /// own comes from orbit-core. A length given (the real-test harness's
    /// short calls) is kept.
    #[test]
    fn a_call_from_the_app_takes_its_length_from_orbit_core() {
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "command": "call_start"
        }));
        assert!(
            matches!(command, Ok(Command::CallStart { duration_minutes })
                if duration_minutes == 0),
            "{command:?}"
        );
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "command": "call_start", "duration_minutes": 2
        }));
        assert!(
            matches!(
                command,
                Ok(Command::CallStart {
                    duration_minutes: 2
                })
            ),
            "{command:?}"
        );
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
