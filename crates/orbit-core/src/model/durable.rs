//! The saved state: one part per domain, each changed only by its own
//! decisions (`transition/`). It is saved and served flat, exactly as before
//! the parts existed (`durable_wire.rs`).
use super::Capabilities;
use crate::transition::{AssistantState, CaptureState, HubLinkState, PrivacyState, VoiceState};

/// Authority fields are read-only outside orbit-core; core transitions and recovery own writes.
/// The privacy stop and the generation are inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.privacy.generation = 1;
/// ```
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.privacy.stopped = true;
/// ```
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.privacy.stop_by = None;
/// ```
/// The hub link is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.link.connection = orbit_core::Connection::Online;
/// ```
/// The open conversation is inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.assistant.session_id = None;
/// ```
/// The helper's capabilities are inaccessible to an external writer.
/// ```compile_fail,E0616
/// let mut state = orbit_core::DurableState::default();
/// state.capabilities = orbit_core::Capabilities::default();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(from = "super::durable_wire::DurableWire")]
pub struct DurableState {
    pub schema: u8,
    /// The privacy stop and the generation of authority.
    pub(crate) privacy: PrivacyState,
    /// The link to the hub.
    pub(crate) link: HubLinkState,
    /// The conversation this Mac keeps: its turn, the agent's question and
    /// the voice notes waiting for the hub.
    pub assistant: AssistantState,
    /// The recording and its capture.
    pub capture: CaptureState,
    /// The microphone for a voice note or a call with the assistant.
    pub voice: VoiceState,
    /// What the native helper observed: itself, the permissions, the
    /// calendar's authorization.
    pub(crate) capabilities: Capabilities,
    /// The last notice said, typed. A state saved before typed
    /// notices held a sentence here; it loads as no notice.
    pub notice: Option<crate::notice::Notice>,
    /// The last notices in the order they happened, added by
    /// [`crate::State::set_notice`] instead of replacing `notice`.
    pub notice_log: crate::notice::NoticeLog,
    pub pending_jobs: u64,
}

impl Default for DurableState {
    fn default() -> Self {
        Self {
            schema: 1,
            privacy: PrivacyState::default(),
            link: HubLinkState::default(),
            assistant: AssistantState::default(),
            capture: CaptureState::default(),
            voice: VoiceState::default(),
            capabilities: Capabilities::default(),
            notice: None,
            notice_log: crate::notice::NoticeLog::default(),
            pending_jobs: 0,
        }
    }
}

impl DurableState {
    pub fn generation(&self) -> u64 {
        self.privacy.generation()
    }
    pub fn privacy_stopped(&self) -> bool {
        self.privacy.stopped()
    }
    pub fn privacy_stop_by(&self) -> Option<crate::privacy_stop::StopRequester> {
        self.privacy.stop_by()
    }
    pub fn connection(&self) -> super::Connection {
        self.link.connection()
    }
    pub fn session_id(&self) -> Option<String> {
        self.assistant.session_id.clone()
    }
    pub fn agent_id(&self) -> Option<&str> {
        self.assistant.agent_id.as_deref()
    }
    /// The recording being thrown away while its capture drains.
    pub fn discarding(&self) -> Option<&str> {
        self.capture.discarding()
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
    /// The waiting voice notes of the open conversation, oldest first: what
    /// it shows after its messages. Another conversation's are not here.
    pub fn open_conversation_pending_voice(&self) -> impl Iterator<Item = &super::Message> {
        self.assistant.open_conversation_pending_voice()
    }
    /// A restart never restarts physical capture or uncertain prompts by
    /// itself, and never invents a stop: an update, a crash or the Mac
    /// booting comes back working unless the user had pressed their stop. A
    /// recording that was live is left interrupted here; the daemon resumes
    /// it, and says so, once its helper is ready (`cut_recovery`). Each
    /// domain recovers its own part; what they say is said in this order.
    pub fn recovered(mut self) -> Self {
        let privacy = self.privacy.recovered();
        self.link = HubLinkState::default();
        self.capabilities = Capabilities::default();
        self.voice.recovered();
        let assistant = self.assistant.recovered();
        let capture = self.capture.recovered();
        for notice in [privacy, assistant, capture].into_iter().flatten() {
            self.set_notice(notice);
        }
        self
    }
}
