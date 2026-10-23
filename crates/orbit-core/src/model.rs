use crate::notice::{Notice, NoticeCode, NoticeKey};
use serde::{Deserialize, Serialize};
use std::{fmt, ops::Deref};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;

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
/// Legacy spelling kept for tests and external clients; production uses resource names.
pub const MAX_FRAME_BYTES: usize = crate::limits::MAX_IPC_LINE_BYTES;
pub const MAX_VOICE_NOTE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    Offline,
    Connecting,
    Online,
    Revoked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePhase {
    #[default]
    Idle,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Interrupted,
}

impl CapturePhase {
    /// A restart must interrupt only a capture that was still active in the saved state.
    pub fn interrupted_on_recovery(&self) -> bool {
        matches!(self, Self::Starting | Self::Recording | Self::Finalizing)
    }
}

impl fmt::Display for CapturePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idle => "cerrada",
            Self::Starting => "iniciando",
            Self::Recording => "grabando",
            Self::Paused => "en pausa",
            Self::Finalizing => "finalizando",
            Self::Interrupted => "interrumpida",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub title: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub ended_at_ms: Option<i64>,
    pub expires_at_ms: i64,
    pub application_pid: Option<i32>,
    #[serde(default)]
    pub audio: crate::protocol::MeetingAudio,
    pub phase: CapturePhase,
    pub sealed_chunks: u64,
    pub gaps: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<TrackProgress>,
    /// Wall-clock instant when the recording entered `Finalizing`. The core
    /// uses this to bound the wait for the Mac's drain acknowledgement; the
    /// field is serialized with `default` so a snapshot written by an older
    /// version that never tracked the wait keeps loading.
    #[serde(default)]
    pub finalizing_since_ms: Option<i64>,
    /// Whether the person was told the recording is about to reach its
    /// ceiling: said once. Absent in a snapshot from an older version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ceiling_notice_given: bool,
}

/// How long any recording may run, whatever started it: by hand, the menu
/// bar, ⌃⌥M, an event in Hoy or the calendar, or a detected call.
///
/// Protects: a recording somebody forgot, or a call whose end the Mac never
/// saw, which would otherwise keep recording and uploading for ever. It is
/// not meant to cut meetings and nothing about a meeting sets it, not even
/// its calendar event: an event's end once stopped a meeting that ran over.
/// Why four hours: meetings of two hours must be kept whole, and four leaves
/// as much again for one that runs over. The app says it before a recording
/// starts («Graba hasta 4 h») from the snapshot, never from a copy. It does
/// not bound a voice call with the assistant (`voice::CALL_MINUTES`), the
/// audio kept on the Mac or what the hub accepts.
pub const RECORDING_CEILING_MINUTES: u16 = 240;

/// How long before [`RECORDING_CEILING_MINUTES`] the person is told, once,
/// that the recording will stop: room to finish it and start another.
pub const RECORDING_CEILING_NOTICE_MINUTES: i64 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackProgress {
    pub name: String,
    pub sealed_chunks: u64,
    pub gaps: u64,
    pub last_chunk_at_ms: i64,
}

/// Bounds the human recording title copied into the final manifest, and
/// the name the user gives a meeting afterwards (`notes::meeting_title`):
/// the same title on the hub (`MAX_RECORDING_TITLE_CHARACTERS`). 256 Unicode
/// scalar values matches Pydantic's `str` length and the hub intake; this is
/// not a byte or transcript limit.
pub const MAX_MANIFEST_TITLE_CHARACTERS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingInterruption {
    pub cause: RecordingInterruptionCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub audio_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingInterruptionCause {
    PrivacyStop,
    CauseNotSaved,
    MicrophonePermissionRequired,
    ScreenPermissionRequired,
    CaptureStartRefused,
    RecordingExpired,
    CaptureFinalizeTimedOut,
    HelperCaptureStop,
    HelperIntegrityFailure,
    HelperLost,
    CaptureEventFailed,
    CaptureMaintenanceFailed,
    AudioStopUnconfirmed,
    CaptureStopUndelivered,
    Shutdown,
    ListenerStopped,
    Restart,
    Unreadable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    pub application_pid: i32,
    pub application_name: String,
    pub bundle_id: String,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_url: Option<SummaryText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<SummaryText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id_short: Option<SummaryText>,
}

/// Heap-backed text keeps these additive presentation summaries from inflating
/// every cloned actor frame while preserving their JSON string contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
#[allow(clippy::box_collection)] // Thin pointer keeps State below its enforced 4 KiB bound.
pub struct SummaryText(Box<String>);

impl From<&str> for SummaryText {
    fn from(value: &str) -> Self {
        Self(Box::new(value.to_owned()))
    }
}

impl From<String> for SummaryText {
    fn from(value: String) -> Self {
        Self(Box::new(value))
    }
}

impl Deref for SummaryText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    /// The helper checked a macOS preflight and got false; TCC did not say
    /// whether the person denied it or has not decided yet.
    NotGranted,
    NotDetermined,
    #[default]
    Unknown,
}

impl PermissionState {
    fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

/// Authority fields are read-only outside orbit-core; core transitions and recovery own writes.
/// `native_helper` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::Capabilities::default();
/// state.native_helper = true;
/// ```
/// `microphone_state` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::Capabilities::default();
/// state.microphone_state = orbit_core::PermissionState::Granted;
/// ```
/// `screen_state` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::Capabilities::default();
/// state.screen_state = orbit_core::PermissionState::Granted;
/// ```
/// `accessibility_state` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::Capabilities::default();
/// state.accessibility_state = orbit_core::PermissionState::Granted;
/// ```
/// `calendar` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::Capabilities::default();
/// state.calendar = None;
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "CapabilitiesWire", into = "CapabilitiesWire")]
pub struct Capabilities {
    pub(crate) native_helper: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) microphone_state: PermissionState,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) screen_state: PermissionState,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    pub(crate) accessibility_state: PermissionState,
    pub wake_word: bool,
    #[serde(default)]
    pub voice_processing_supported: bool,
    #[serde(default)]
    pub voice_processing_available: bool,
    #[serde(default)]
    pub playback_reference_available: bool,
    /// Last calendar authorization the helper reported. `None` means nothing has
    /// been observed yet, which is not the same as denied: the helper sends this
    /// with every status, so a missing value means the app is not running.
    #[serde(default)]
    pub(crate) calendar: Option<crate::calendar::CalendarAuthorization>,
    /// How many calendars the Mac has, from the last complete listing: what
    /// Ajustes shows beside the permission, taken from the data and
    /// kept here rather than in the calendar view so that opening another
    /// screen cannot turn it into «0». `None` means not known yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendars: Option<u32>,
}

impl Capabilities {
    pub fn native_helper(&self) -> bool {
        self.native_helper
    }
    pub fn microphone_state(&self) -> PermissionState {
        self.microphone_state
    }
    pub fn screen_state(&self) -> PermissionState {
        self.screen_state
    }
    pub fn accessibility_state(&self) -> PermissionState {
        self.accessibility_state
    }
    pub fn calendar(&self) -> Option<crate::calendar::CalendarAuthorization> {
        self.calendar
    }
}

/// Snapshots written before the in-process app retain boolean permissions and
/// sidecars. Keep reading their observed non-grants without inferring a denial
/// from a legacy false field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CapabilitiesWire {
    native_helper: bool,
    #[serde(default)]
    microphone: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    microphone_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    microphone_not_granted: bool,
    #[serde(default)]
    screen_audio: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    screen_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    screen_not_granted: bool,
    #[serde(default)]
    accessibility: bool,
    #[serde(default, skip_serializing_if = "PermissionState::is_unknown")]
    accessibility_state: PermissionState,
    #[serde(default, skip_serializing_if = "is_false")]
    accessibility_not_granted: bool,
    wake_word: bool,
    #[serde(default)]
    voice_processing_supported: bool,
    #[serde(default)]
    voice_processing_available: bool,
    #[serde(default)]
    playback_reference_available: bool,
    #[serde(default)]
    calendar: Option<crate::calendar::CalendarAuthorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calendars: Option<u32>,
}

impl From<Capabilities> for CapabilitiesWire {
    fn from(value: Capabilities) -> Self {
        let old_state = |state| {
            if state == PermissionState::NotGranted {
                PermissionState::Unknown
            } else {
                state
            }
        };
        Self {
            native_helper: value.native_helper,
            microphone: value.microphone_state == PermissionState::Granted,
            microphone_state: old_state(value.microphone_state),
            microphone_not_granted: value.microphone_state == PermissionState::NotGranted,
            screen_audio: value.screen_state == PermissionState::Granted,
            screen_state: old_state(value.screen_state),
            screen_not_granted: value.screen_state == PermissionState::NotGranted,
            accessibility: value.accessibility_state == PermissionState::Granted,
            accessibility_state: old_state(value.accessibility_state),
            accessibility_not_granted: value.accessibility_state == PermissionState::NotGranted,
            wake_word: value.wake_word,
            voice_processing_supported: value.voice_processing_supported,
            voice_processing_available: value.voice_processing_available,
            playback_reference_available: value.playback_reference_available,
            calendar: value.calendar,
            calendars: value.calendars,
        }
    }
}

impl From<CapabilitiesWire> for Capabilities {
    fn from(value: CapabilitiesWire) -> Self {
        // The legacy bools exist for old readers only. A missing typed state
        // stays Unknown for new readers, even when an old bool is true.
        let observed = |state, not_granted| {
            if not_granted {
                PermissionState::NotGranted
            } else {
                state
            }
        };
        Self {
            native_helper: value.native_helper,
            microphone_state: observed(value.microphone_state, value.microphone_not_granted),
            screen_state: observed(value.screen_state, value.screen_not_granted),
            accessibility_state: observed(
                value.accessibility_state,
                value.accessibility_not_granted,
            ),
            wake_word: value.wake_word,
            voice_processing_supported: value.voice_processing_supported,
            voice_processing_available: value.voice_processing_available,
            playback_reference_available: value.playback_reference_available,
            calendar: value.calendar,
            calendars: value.calendars,
        }
    }
}

/// Authority fields are read-only outside orbit-core; core transitions and recovery own writes.
/// `generation` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.generation = 1;
/// ```
/// `privacy_stopped` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.privacy_stopped = true;
/// ```
/// `privacy_stop_by` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.privacy_stop_by = None;
/// ```
/// `connection` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.connection = orbit_core::Connection::Online;
/// ```
/// `session_id` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.session_id = None;
/// ```
/// `capabilities` is inaccessible to an external writer.
/// ```compile_fail
/// let mut state = orbit_core::DurableState::default();
/// state.capabilities = orbit_core::Capabilities::default();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableState {
    pub schema: u8,
    pub(crate) generation: u64,
    pub(crate) privacy_stopped: bool,
    /// Who asked for the stop in force: only the user's is carried
    /// across a restart as theirs. `None` with a stop in force is a stop
    /// nobody can attribute, and is kept. Omitted rather than nulled so a
    /// state that is not stopped stays byte-identical on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) privacy_stop_by: Option<crate::privacy_stop::StopRequester>,
    pub(crate) connection: Connection,
    pub(crate) session_id: Option<String>,
    /// The agent the open conversation is with (hub-v1 «Agents»), as the
    /// app chose it or the hub listed it; `None` is the hub's default agent,
    /// which every conversation from before agents is with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent_id: Option<String>,
    pub active_request: Option<String>,
    /// A state saved before the hub kept the conversations held up to 200
    /// messages here, under
    /// `messages`; it loads without a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
    /// Voice notes the hub has not taken yet that a later turn replaced,
    /// oldest first, at most [`MAX_PENDING_VOICE_NOTES`] across every
    /// conversation: each shown in its own conversation, and sent there
    /// again from the WAV this Mac keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_voice: Vec<crate::PendingVoiceNote>,
    pub recording: Option<Recording>,
    pub voice_active: bool,
    #[serde(default)]
    pub call: Option<crate::voice::CallState>,
    pub pending_approval: Option<Approval>,
    pub(crate) capabilities: Capabilities,
    /// The last notice said, typed. A state saved before typed
    /// notices held a sentence here; it loads as no notice.
    #[serde(default, deserialize_with = "crate::notice::lenient_notice")]
    pub notice: Option<crate::notice::Notice>,
    /// The last notices in the order they happened, added by
    /// [`State::set_notice`] instead of replacing `notice`.
    #[serde(default)]
    pub notice_log: crate::notice::NoticeLog,
    pub pending_jobs: u64,
}

impl Default for DurableState {
    fn default() -> Self {
        Self {
            schema: 1,
            generation: 0,
            privacy_stopped: false,
            privacy_stop_by: None,
            connection: Connection::Offline,
            session_id: None,
            agent_id: None,
            active_request: None,
            turn: None,
            pending_voice: Vec::new(),
            recording: None,
            voice_active: false,
            call: None,
            pending_approval: None,
            capabilities: Capabilities::default(),
            notice: None,
            notice_log: crate::notice::NoticeLog::default(),
            pending_jobs: 0,
        }
    }
}

impl DurableState {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn privacy_stopped(&self) -> bool {
        self.privacy_stopped
    }
    pub fn privacy_stop_by(&self) -> Option<crate::privacy_stop::StopRequester> {
        self.privacy_stop_by
    }
    pub fn connection(&self) -> Connection {
        self.connection.clone()
    }
    pub fn session_id(&self) -> Option<String> {
        self.session_id.clone()
    }
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    /// Says one notice: sets the current one and appends to the bounded
    /// sequence, so what was said before stays readable.
    pub fn set_notice(&mut self, notice: impl Into<crate::notice::Notice>) {
        self.notice = Some(self.notice_log.record(notice.into()));
    }
    /// The code of the current notice, if any.
    pub fn notice_code(&self) -> Option<crate::notice::NoticeCode> {
        self.notice.as_ref().map(|notice| notice.code)
    }
    /// A restart never restarts physical capture or uncertain prompts by
    /// itself, and never invents a stop: an update, a crash or the Mac
    /// booting comes back working unless the user had pressed their stop. A
    /// recording that was live is left interrupted here; the daemon resumes
    /// it, and says so, once its helper is ready (`cut_recovery`).
    pub fn recovered(mut self) -> Self {
        use crate::privacy_stop::StopRequester;
        match (self.privacy_stopped, self.privacy_stop_by) {
            // The hub decides a revocation again at the next connection; a
            // restart does not carry it as if the user had asked for it.
            (true, Some(StopRequester::Revocation)) | (false, _) => {
                self.privacy_stopped = false;
                self.privacy_stop_by = None;
            }
            (true, Some(StopRequester::User)) => self.set_notice(
                Notice::new(NoticeCode::PrivacyStopKept).with(NoticeKey::Attributed, true),
            ),
            // Saved by a version that did not record who asked. Absence is
            // not a "no": the stop is kept, and says why.
            (true, None) => self.set_notice(
                Notice::new(NoticeCode::PrivacyStopKept).with(NoticeKey::Attributed, false),
            ),
        }
        self.generation = self.generation.saturating_add(1);
        self.connection = Connection::Offline;
        self.capabilities = Capabilities::default();
        self.voice_active = false;
        for voice in self
            .turn
            .iter_mut()
            .map(|turn| &mut turn.user)
            .chain(self.pending_voice.iter_mut().map(|note| &mut note.message))
            .filter_map(|message| message.voice.as_mut())
            .filter(|voice| voice.transcript.is_none() && voice.failure.is_none())
        {
            voice.failure = Some(VoiceTranscriptionFailure::local(
                "voice_transcription_interrupted_restart",
            ));
        }
        // Revoke live call authority, not its durable original's lifecycle.
        // The daemon reconciles voice_call manifests separately before capture
        // or network work can resume; this pure model never performs recovery I/O.
        self.call = None;
        self.pending_approval = None;
        if self.active_request.take().is_some() {
            self.set_notice(NoticeCode::AssistantTurnLostByRestart);
        }
        if let Some(recording) = self.recording.as_mut()
            && recording.phase.interrupted_on_recovery()
        {
            recording.phase = CapturePhase::Interrupted;
            recording.gaps += 1;
            let saved = recording.sealed_chunks;
            self.set_notice(
                Notice::new(NoticeCode::RecordingInterruptedByRestart)
                    .with(NoticeKey::Fragments, saved),
            );
        }
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Views {
    #[serde(default)]
    pub source_inventory: Option<crate::sources::WindowInventory>,
    #[serde(default)]
    pub source_request: Option<crate::sources::SourceProgress>,
    #[serde(default)]
    pub source_receipt: Option<crate::sources::SourceReceipt>,

    /// Startup recovery is an observed phase, not an absent/default state.
    #[serde(default, skip_serializing_if = "RecoveryStatus::is_unknown")]
    pub recovery: RecoveryStatus,
    /// Wall-clock start of the native audio drain currently visible to IPC.
    /// This is presentation state only: a restarted daemon cannot resume an
    /// in-memory helper command from a persisted timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draining_since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_sources: Vec<AudioSource>,
    /// Why the helper's inventory of applications with audio is not shown,
    /// with its numbers. Absent while the list is fine or has
    /// not been read: an empty list with no reason is "none running", and
    /// one with a reason is "could not be verified", never the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_sources_unavailable: Option<crate::refusal::Refusal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_signal: Option<crate::audio_signal::RecordingSignalView>,
    /// The last voice note the daemon did not send for being too short,
    /// until the next note or message. Presentation only: a restart forgets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_voice_note: Option<crate::voice::ShortVoiceNote>,
}

/// Durable authority is read-only outside orbit-core; see [`DurableState`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(flatten)]
    pub durable: DurableState,
    #[serde(flatten)]
    pub views: Views,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStep {
    Chunks,
    UploadJobs,
    Snapshot,
    Environment,
    Operations,
    /// «Al detectar una llamada», read from the encrypted store.
    CallPreference,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecoveryStatus {
    #[default]
    Unknown,
    Recovering {
        step: RecoveryStep,
        elapsed_ms: u64,
    },
    Ready,
}

impl RecoveryStatus {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<DurableState> for State {
    fn from(durable: DurableState) -> Self {
        Self {
            durable,
            views: Views::default(),
        }
    }
}

impl State {
    pub fn generation(&self) -> u64 {
        self.durable.generation()
    }
    pub fn privacy_stopped(&self) -> bool {
        self.durable.privacy_stopped()
    }
    pub fn privacy_stop_by(&self) -> Option<crate::privacy_stop::StopRequester> {
        self.durable.privacy_stop_by()
    }
    pub fn connection(&self) -> Connection {
        self.durable.connection()
    }
    pub fn session_id(&self) -> Option<String> {
        self.durable.session_id()
    }
    /// The open conversation's agent; `None` is the hub's default.
    pub fn agent_id(&self) -> Option<&str> {
        self.durable.agent_id()
    }
    pub fn capabilities(&self) -> &Capabilities {
        self.durable.capabilities()
    }
    /// Voice processing and calendar count are observations, not grants of
    /// authority. The four permissions and helper presence remain decision-only.
    pub fn set_voice_processing_supported(&mut self, value: bool) {
        self.durable.capabilities.voice_processing_supported = value;
    }
    pub fn set_voice_processing_available(&mut self, value: bool) {
        self.durable.capabilities.voice_processing_available = value;
    }
    pub fn set_playback_reference_available(&mut self, value: bool) {
        self.durable.capabilities.playback_reference_available = value;
    }
    pub fn set_calendar_count(&mut self, count: Option<u32>) {
        self.durable.capabilities.calendars = count;
    }
    /// Whether a native helper is connected at all, so «nobody has asked
    /// yet» and «there is nobody to ask» stay distinguishable. The helper
    /// reports capabilities with every status, so a missing report means no
    /// helper; this reads that fact and never asks the system itself.
    pub fn helper_connected(&self) -> bool {
        self.durable.capabilities.native_helper
    }

    pub fn durable(&self) -> DurableState {
        self.durable.clone()
    }

    pub fn set_notice(&mut self, notice: impl Into<crate::notice::Notice>) {
        self.durable.set_notice(notice);
    }
    pub fn notice_code(&self) -> Option<crate::notice::NoticeCode> {
        self.durable.notice_code()
    }
    pub fn recovered(self) -> Self {
        self.durable.recovered().into()
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn recovery_absence_is_unknown_and_ready_is_explicit() -> Result<(), serde_json::Error> {
        let older = serde_json::to_value(State::default())?;
        assert!(older.get("recovery").is_none());
        let restored: State = serde_json::from_value(older)?;
        assert_eq!(restored.views.recovery, RecoveryStatus::Unknown);

        let ready = State {
            views: crate::Views {
                recovery: RecoveryStatus::Ready,
                ..crate::Views::default()
            },
            ..State::default()
        };
        let served = serde_json::to_value(ready)?;
        assert_eq!(served["recovery"]["state"], "ready");
        Ok(())
    }

    #[test]
    fn ephemeral_views_do_not_inflate_every_actor_frame() {
        // A resource budget, not a stable ABI. Large new presentation payloads
        // belong off-stack; raising the test-thread stack hides this regression.
        assert!(std::mem::size_of::<State>() <= 4096);
    }

    #[test]
    fn a_snapshot_that_still_names_the_daemon_version_loads() -> Result<(), serde_json::Error> {
        // Up to 0.5.x the daemon stamped its build version on the state it
        // served, and a snapshot could keep it. One executable has one
        // version since the daemon runs inside the app: the field is gone,
        // and a snapshot that still carries it loads as if it never had.
        let mut saved = serde_json::to_value(DurableState::default())?;
        if let Some(fields) = saved.as_object_mut() {
            fields.insert("daemon_version".into(), "0.5.1".into());
        }
        let restored: DurableState = serde_json::from_value(saved)?;
        assert_eq!(restored, DurableState::default());
        assert!(
            serde_json::to_value(&restored)?
                .get("daemon_version")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn the_ipc_state_says_whether_a_helper_is_connected() {
        // «nadie lo ha preguntado todavía» and «no hay quien lo
        // pregunte» are only distinguishable while the state says whether a
        // helper is connected at all.
        let connected = State {
            durable: crate::DurableState {
                capabilities: Capabilities {
                    native_helper: true,
                    ..Capabilities::default()
                },
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        assert!(connected.helper_connected());
        let absent = State::default();
        assert!(!absent.helper_connected());
    }

    #[test]
    fn snapshot_without_finalizing_since_ms_deserializes() -> Result<(), serde_json::Error> {
        let state = State {
            durable: crate::DurableState {
                recording: Some(Recording {
                    id: "r".into(),
                    title: "Sesión".into(),
                    started_at_ms: 1_000,
                    ended_at_ms: Some(2_000),
                    expires_at_ms: 1_000_000,
                    application_pid: None,
                    audio: crate::protocol::MeetingAudio::MicrophoneOnly,
                    phase: CapturePhase::Finalizing,
                    sealed_chunks: 1,
                    gaps: 0,
                    tracks: Vec::new(),
                    finalizing_since_ms: Some(2_000),
                    ceiling_notice_given: false,
                }),
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let mut value = serde_json::to_value(&state)?;
        // A snapshot written before the field existed must keep loading.
        let removed = value
            .pointer_mut("/recording")
            .and_then(serde_json::Value::as_object_mut)
            .map(|recording| recording.remove("finalizing_since_ms").is_some());
        assert_eq!(removed, Some(true));
        let restored: State = serde_json::from_value(value)?;
        assert_eq!(
            restored
                .durable
                .recording
                .and_then(|recording| recording.finalizing_since_ms),
            None
        );
        Ok(())
    }
}
