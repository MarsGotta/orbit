//! The assistant's handle: the conversation in progress, its voice turns,
//! what is read aloud and what the hub said about attachments.
use super::*;

/// The conversation's runtime state, kept in memory only.
pub(super) struct AssistantHandle {
    pub(super) bridge_request: Option<BridgeAsk>,
    pub(super) interaction_response: Option<interactions::PendingResponse>,
    pub(super) interrupted_turns: interactions::InterruptedTurns,
    /// One voice turn worker; a second turn cannot start while one runs.
    pub(super) voice_task: ticketed::Ticketed<()>,
    pub(super) voice_operation: Option<String>,
    /// Voice turns left without a trace since the daemon started (nothing
    /// heard, or carried by the next turn): the count the log carries.
    pub(super) dropped_voice_turns: u64,
    /// One playback at a time; a replay replaces the current one.
    pub(super) speech_task: ticketed::Ticketed<()>,
    /// What the last reading in each voice said, by the agent whose voice
    /// it is (`AgentRoster::reading_voice`): `true` it played, `false` the
    /// hub's voice did not arrive. A voice not read yet is not here, which
    /// is unknown, not «no». One entry per voice that has read, so at most
    /// one per agent a hub has listed while the daemon runs.
    pub(super) speech_voices: BTreeMap<String, bool>,
    pub(super) speak_next: bool,
    pub(super) speech_notice_call_id: Option<String>,
    /// The answer the user asked to hear («Escuchar»), while it plays.
    pub(super) read_aloud: Option<speech::ReadAloud>,
    /// What the agent did in the turn in progress, or the last one
    /// (`chat.item`). Live only: a restart starts without it.
    pub(super) work: Option<orbit_core::agent_work::AgentWork>,
    /// The connected hub's attachment ceilings, from its last `ready`;
    /// `None` before one arrives and for a 0.9.0 hub, which has none.
    pub(super) hub_attachments: Option<orbit_core::attachments::AttachmentLimits>,
    /// The conversation the next new one takes over (`Command::Handover`),
    /// sent with its first `chat.send` and then forgotten.
    pub(super) handover_from: Option<String>,
    /// «Adjuntar ventana» while its capture runs (`window_attachment`).
    pub(super) window_attachment: Option<window_attachment::PendingWindow>,
    /// The revision of the last chat snapshot the hub sent.
    pub(super) snapshot_revision: Option<u64>,
}

impl AssistantHandle {
    pub(super) fn new() -> Self {
        Self {
            bridge_request: None,
            interaction_response: None,
            interrupted_turns: Default::default(),
            voice_task: ticketed::Ticketed::logical(),
            voice_operation: None,
            dropped_voice_turns: 0,
            speech_task: ticketed::Ticketed::logical(),
            speech_voices: BTreeMap::new(),
            speak_next: false,
            read_aloud: None,
            hub_attachments: None,
            handover_from: None,
            window_attachment: None,
            snapshot_revision: None,
            speech_notice_call_id: None,
            work: None,
        }
    }
}

impl Runtime {
    /// The agent a turn is for, and whose voice reads its answer: the open
    /// conversation's, or the default one. Only a hub that listed its agents
    /// is told: a 0.9.0 hub refuses the field in `chat.send` and has one
    /// agent anyway.
    pub(super) fn turn_agent_id(&self) -> Option<String> {
        let roster = self.settings.hub_agents.as_ref()?;
        Some(
            self.state
                .agent_id()
                .unwrap_or(roster.default_agent_id())
                .to_owned(),
        )
    }

    /// The agent whose voice reads the open conversation's answers aloud:
    /// its own, or the default agent's when it has none (as the hub falls back).
    pub(super) fn reading_voice(&self) -> String {
        let roster = self.settings.roster();
        let agent_id = self.state.agent_id().unwrap_or(roster.default_agent_id());
        roster.reading_voice(agent_id).to_owned()
    }

    /// The user's turn goes to the hub as `chat.send`.
    pub(super) async fn send_turn_text(
        &mut self,
        id: String,
        text: String,
        voice: Option<orbit_core::wire::hub::ChatVoice>,
        meeting: Option<orbit_core::wire::hub::ChatMeeting>,
        attachments: Vec<String>,
    ) -> Result<()> {
        // The work shown is this turn's from now on: the last
        // turn's never stands for it.
        self.assistant.work.take_if(|work| work.turn_id != id);
        // The turn's id is the Mac's id for the user's message,
        // kept in the hub's row so the app finds their turn by it.
        let params = serde_json::to_value(orbit_core::wire::hub::ChatSendParams {
            text,
            session_id: self.state.session_id().clone(),
            call_active: Some(self.state.durable.voice.call.is_some()),
            client_id: Some(id.clone()),
            voice,
            meeting,
            agent_id: self.turn_agent_id(),
            attachments,
            // The conversation this new one takes over; only its first turn.
            handover_from: if self.state.session_id().is_none() {
                self.assistant.handover_from.take()
            } else {
                None
            },
        })?;
        self.send(hub::command(&id, "chat.send", params)?).await
    }
}
