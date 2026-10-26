//! Waiting notes from a new view that was left are delivered to their own
//! conversation. They remain hidden locally; the hub's history shows them
//! after delivery. No navigation or current-turn ownership changes here.
use super::{Runtime, audio::VoiceSource};
use anyhow::{Context, Result};
use orbit_core::{Event, PendingVoiceNote, VoiceTranscriptionFailure};

impl Runtime {
    /// A lost delivery receipt cannot split one draft into two hub sessions.
    /// Other drafts and known conversations remain eligible for delivery.
    pub(super) fn detached_voice_can_resume(&self, note: &PendingVoiceNote) -> bool {
        if !note.detached() {
            return false;
        }
        let draft = note
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.abandoned_draft.as_ref());
        draft.is_none_or(|draft| {
            !self
                .state
                .durable
                .assistant
                .pending_voice
                .iter()
                .any(|other| {
                    other
                        .conversation
                        .as_ref()
                        .and_then(|conversation| conversation.abandoned_draft.as_ref())
                        == Some(draft)
                        && other
                            .message
                            .voice
                            .as_ref()
                            .and_then(|voice| voice.failure.as_ref())
                            .is_some_and(|failure| failure.code == "voice_delivery_unconfirmed")
                })
        })
    }

    /// Completed answers release the queue without replacing the open view.
    /// Errors continue through the usual cause-reporting path.
    pub(super) fn settle_detached_voice_frame(
        &mut self,
        frame: &orbit_core::wire::hub::HubEvent,
    ) -> bool {
        use orbit_core::wire::hub::HubEventKind;
        if self.assistant.background_voice_request.is_none()
            || frame.id.as_deref() != self.assistant.background_voice_request.as_deref()
        {
            return false;
        }
        if matches!(
            frame.kind,
            HubEventKind::ChatCompleted(_) | HubEventKind::Error(_)
        ) {
            self.assistant.background_voice_request = None;
            return matches!(frame.kind, HubEventKind::ChatCompleted(_));
        }
        false
    }

    pub(super) fn detached_voice_note(&self, id: &str) -> Option<&PendingVoiceNote> {
        self.state
            .durable
            .assistant
            .pending_voice
            .iter()
            .find(|note| note.message.id == id && note.detached())
    }

    pub(super) async fn resume_detached_voice_note(&mut self, id: &str) -> Result<()> {
        let note = self
            .detached_voice_note(id)
            .context("waiting voice note absent")?;
        let source = VoiceSource {
            recording_id: id.to_owned(),
            message_id: id.to_owned(),
            chunk_id: String::new(),
            session_id: note
                .conversation
                .as_ref()
                .and_then(|conversation| conversation.session_id.clone()),
            duration_ms: note
                .message
                .voice
                .as_ref()
                .map_or(0, |voice| voice.duration_ms),
        };
        let (chunk_id, wav) = self
            .stored_voice_note(id)?
            .context("waiting voice audio absent")?;
        let token = self
            .link
            .token
            .clone()
            .context("waiting voice pairing absent")?;
        // Persist that this attempt began. A restart must not resend work
        // whose receipt may have been lost. The original WAV remains local.
        self.apply(Event::VoiceResume { id: id.to_owned() }).await?;
        self.store_actor.snapshot(&self.state).await?;
        self.spawn_voice_transcription(VoiceSource { chunk_id, ..source }, wav, token, false)
            .await
    }

    pub(super) async fn send_detached_voice_text(
        &mut self,
        source: &VoiceSource,
        text: String,
    ) -> Result<bool> {
        let Some(note) = self.detached_voice_note(&source.message_id) else {
            return Ok(false);
        };
        let conversation = note.conversation.as_ref();
        let agent_id = self.settings.hub_agents.as_ref().map(|roster| {
            conversation
                .and_then(|conversation| conversation.agent_id.clone())
                .unwrap_or_else(|| roster.default_agent_id().to_owned())
        });
        let params = orbit_core::wire::hub::ChatSendParams {
            text,
            session_id: source.session_id.clone(),
            call_active: Some(false),
            client_id: Some(source.message_id.clone()),
            voice: Some(orbit_core::wire::hub::ChatVoice {
                duration_ms: source.duration_ms,
            }),
            meeting: None,
            agent_id,
            attachments: Vec::new(),
            handover_from: None,
        };
        self.apply(Event::VoiceFailed {
            id: source.message_id.clone(),
            failure: VoiceTranscriptionFailure::local("voice_delivery_unconfirmed"),
        })
        .await?;
        self.store_actor.snapshot(&self.state).await?;
        self.send(crate::hub::command(
            &source.message_id,
            "chat.send",
            serde_json::to_value(params)?,
        )?)
        .await?;
        self.assistant.background_voice_request = Some(source.message_id.clone());
        Ok(true)
    }

    pub(super) async fn detached_voice_session(
        &mut self,
        id: Option<&str>,
        session: &str,
    ) -> Result<bool> {
        let Some(id) = id.filter(|id| {
            self.detached_voice_note(id).is_some_and(|note| {
                note.message
                    .voice
                    .as_ref()
                    .and_then(|voice| voice.failure.as_ref())
                    .is_some_and(|failure| failure.code == "voice_delivery_unconfirmed")
            })
        }) else {
            return Ok(false);
        };
        self.apply(Event::VoiceSession {
            message_id: id.to_owned(),
            session_id: session.to_owned(),
        })
        .await?;
        self.store_actor.snapshot(&self.state).await?;
        Ok(true)
    }
}
