//! main's flat saved state, copied verbatim (only `pub(crate)` made `pub`
//! and the paths of shared types qualified): what `legacy::decide` decides
//! on. Test-only.
#![allow(dead_code)]
use crate::model::{
    Approval, Capabilities, CapturePhase, Connection, Recording, Turn, Views,
    VoiceTranscriptionFailure,
};
use crate::notice::{Notice, NoticeCode, NoticeKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableState {
    pub schema: u8,
    pub generation: u64,
    pub privacy_stopped: bool,
    /// Who asked for the stop in force: only the user's is carried
    /// across a restart as theirs. `None` with a stop in force is a stop
    /// nobody can attribute, and is kept. Omitted rather than nulled so a
    /// state that is not stopped stays byte-identical on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_stop_by: Option<crate::privacy_stop::StopRequester>,
    pub connection: Connection,
    pub session_id: Option<String>,
    /// The agent the open conversation is with (hub-v1 «Agents»), as the
    /// app chose it or the hub listed it; `None` is the hub's default agent,
    /// which every conversation from before agents is with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
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
    /// «Terminar sin guardar»: the recording whose capture is draining to
    /// be thrown away, never closed, uploaded or written up
    /// (`transition/discard.rs`). Absent in every other state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarding: Option<String>,
    pub voice_active: bool,
    #[serde(default)]
    pub call: Option<crate::voice::CallState>,
    pub pending_approval: Option<Approval>,
    pub capabilities: Capabilities,
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
            discarding: None,
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
    /// The recording being thrown away while its capture drains.
    pub fn discarding(&self) -> Option<&str> {
        self.discarding.as_deref()
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
        // A recording being thrown away is gone: it never resumes or closes.
        if let Some(discarded) = self.discarding.take()
            && self
                .recording
                .as_ref()
                .is_some_and(|recording| recording.id == discarded)
        {
            self.recording = None;
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

/// main's served state: the flat saved state and the views beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(flatten)]
    pub durable: DurableState,
    #[serde(flatten)]
    pub views: Views,
}

impl State {
    pub fn set_notice(&mut self, notice: impl Into<crate::notice::Notice>) {
        self.durable.set_notice(notice);
    }
    pub fn recovered(self) -> Self {
        Self {
            durable: self.durable.recovered(),
            views: Views::default(),
        }
    }
}

/// main's `PendingVoiceNote::belongs_to`, on the flat state.
pub fn belongs_to(note: &crate::PendingVoiceNote, durable: &DurableState) -> bool {
    use crate::VoiceNoteConversation;
    match &note.conversation {
        Some(VoiceNoteConversation {
            session_id: Some(session),
            ..
        }) => durable.session_id.as_deref() == Some(session.as_str()),
        Some(VoiceNoteConversation {
            session_id: None,
            agent_id,
        }) => durable.session_id.is_none() && durable.agent_id == *agent_id,
        None => durable.session_id.is_none(),
    }
}
