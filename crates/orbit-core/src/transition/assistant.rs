//! The assistant and the chat: the open conversation and its agent, the
//! turn in flight, the agent's question and the voice notes this Mac keeps
//! for the hub. The conversation itself is the hub's.
//!
//! Reads of other domains: the privacy stop and the hub link before a
//! turn is sent, and the microphone (`voice_call`) before a voice note is
//! transcribed again.
use super::{Effect, Step};
use crate::authority::AuthorityChange;
use crate::model::{Approval, Message, Turn, VoiceTranscriptionFailure};
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::pending_voice::PendingVoiceNote;

mod chat;
mod voice_drafts;
mod voice_notes;

pub(super) use chat::{approval, completed, delta, submit};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssistantState {
    pub(crate) session_id: Option<String>,
    /// The agent the open conversation is with (hub-v1 «Agents»), as the
    /// app chose it or the hub listed it; `None` is the hub's default agent,
    /// which every conversation from before agents is with.
    pub(crate) agent_id: Option<String>,
    pub active_request: Option<String>,
    /// The turn in progress, or the last one until another begins.
    pub turn: Option<Turn>,
    /// Voice notes the hub has not taken yet that a later turn replaced,
    /// oldest first, at most [`crate::MAX_PENDING_VOICE_NOTES`] across every
    /// conversation: each shown in its own conversation, and sent there
    /// again from the WAV this Mac keeps.
    pub pending_voice: Vec<PendingVoiceNote>,
    pub pending_approval: Option<Approval>,
}

impl AssistantState {
    /// Old session-less fragments to remove on load, including their audio.
    /// Notes already owned by a hub conversation are preserved verbatim.
    pub fn obsolete_voice_note_ids(&self) -> Vec<String> {
        self.pending_voice
            .iter()
            .filter(|note| note.obsolete_sessionless())
            .map(|note| note.message.id.clone())
            .chain(
                self.turn
                    .iter()
                    .filter(|turn| {
                        self.session_id.is_none()
                            && crate::pending_voice::obsolete_voice_message(&turn.user)
                    })
                    .map(|turn| turn.user.id.clone()),
            )
            .collect()
    }

    pub(super) fn turn_in_flight(&self) -> bool {
        self.active_request.is_some()
    }
    pub(super) fn question_pending(&self) -> bool {
        self.pending_approval.is_some()
    }
    pub(super) fn waiting_voice_notes(&self) -> usize {
        self.pending_voice.len()
    }

    /// The waiting voice notes of the open conversation, oldest first.
    pub(crate) fn open_conversation_pending_voice(&self) -> impl Iterator<Item = &Message> {
        self.pending_voice
            .iter()
            .filter(|note| note.belongs_to(self))
            .map(|note| &note.message)
    }

    /// The turn in flight and its question end, without an answer.
    pub(super) fn retire_request(&mut self) {
        self.active_request = None;
        self.pending_approval = None;
    }

    /// A restart forgets the turn in flight and its question; a voice note
    /// that was still being transcribed says the restart interrupted it.
    pub(crate) fn recovered(&mut self) -> Option<Notice> {
        let obsolete = self.obsolete_voice_note_ids();
        self.pending_voice
            .retain(|note| !obsolete.contains(&note.message.id));
        if self
            .turn
            .as_ref()
            .is_some_and(|turn| obsolete.contains(&turn.user.id))
        {
            self.turn = None;
        }
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
        self.pending_approval = None;
        self.active_request
            .take()
            .map(|_| NoticeCode::AssistantTurnLostByRestart.into())
    }
}

/// The hub named the conversation (`chat.session`). Only a conversation
/// the hub names for the first time takes its agent: one opened without an
/// agent is from before agents, and the hub keeps it with whichever agent is
/// its default.
pub(super) fn session(
    assistant: &mut AssistantState,
    id: String,
    agent_id: Option<String>,
    cx: &mut Step,
) {
    let first = assistant.session_id.is_none();
    if assistant
        .session_id
        .as_ref()
        .is_some_and(|previous| previous != &id)
    {
        cx.authority(AuthorityChange::Conversation);
    }
    if first {
        // Match the view's agent before the hub resolves its default agent.
        voice_notes::adopt_new_conversation_notes(assistant, &id);
    }
    if first && assistant.agent_id.is_none() {
        assistant.agent_id = agent_id;
    }
    assistant.session_id = Some(id);
}

/// A new conversation with `agent_id`; `None` is the hub's default agent.
pub(super) fn new_session(assistant: &mut AssistantState, agent_id: Option<String>, cx: &mut Step) {
    cx.authority(AuthorityChange::Conversation);
    leave_conversation(assistant);
    assistant.agent_id = agent_id;
}

/// An earlier conversation, with the agent the hub listed it with. Its
/// messages are the hub's; the app reads them.
pub(super) fn open_session(
    assistant: &mut AssistantState,
    id: String,
    agent_id: Option<String>,
    cx: &mut Step,
) {
    cx.authority(AuthorityChange::Conversation);
    voice_notes::retire_turn(assistant);
    voice_drafts::abandon_new_view(assistant);
    assistant.pending_approval = None;
    assistant.session_id = Some(id);
    assistant.agent_id = agent_id;
}

/// The paired device changed: nothing of the conversation is its.
pub(super) fn device_changed(assistant: &mut AssistantState, cx: &mut Step) {
    cx.authority(AuthorityChange::Device);
    voice_drafts::abandon_new_view(assistant);
    assistant.session_id = None;
    assistant.agent_id = None;
    assistant.active_request = None;
    assistant.pending_approval = None;
}

pub(super) fn interrupt(assistant: &mut AssistantState, cx: &mut Step) {
    cx.authority(AuthorityChange::Interrupt);
    assistant.retire_request();
    cx.effects([Effect::Interrupt, Effect::StopPlayback]);
    cx.say(NoticeCode::AssistantInterruptRequested);
}

/// A failed chat turn, said with the hub's code and the numbers and closed
/// tokens of its refusal. A conversation the hub no longer has is left.
pub(super) fn failure(
    assistant: &mut AssistantState,
    code: &str,
    numbers: Vec<(NoticeKey, String)>,
    cx: &mut Step,
) {
    let notice = if code == "session_not_found" {
        leave_conversation(assistant);
        NoticeCode::AssistantSessionNotFound
    } else {
        NoticeCode::AssistantFailed
    };
    let notice = numbers.into_iter().fold(
        Notice::new(notice).with_token(NoticeKey::Code, code),
        |notice, (key, value)| notice.with_token(key, &value),
    );
    fail_turn(assistant, notice, cx);
}

/// The hub went away: a turn in flight is lost.
pub(super) fn hub_lost(assistant: &mut AssistantState, cx: &mut Step) {
    if assistant.active_request.is_some() {
        fail_turn(assistant, NoticeCode::AssistantTurnLostByDisconnect, cx);
    }
}

fn fail_turn(assistant: &mut AssistantState, notice: impl Into<Notice>, cx: &mut Step) {
    cx.say(notice);
    assistant.retire_request();
}

/// The open conversation is left for a new one. The turn is retired while
/// its conversation is still the open one, so a note it keeps stays that
/// conversation's.
fn leave_conversation(assistant: &mut AssistantState) {
    voice_notes::retire_turn(assistant);
    voice_drafts::abandon_new_view(assistant);
    assistant.session_id = None;
    assistant.pending_approval = None;
}

/// Voice-note events share the assistant's state and no other domain's.
pub(super) fn voice_event(
    assistant: &mut AssistantState,
    event: super::Event,
    cx: &mut Step,
) -> Result<(), crate::refusal::Refusal> {
    use super::Event;
    match event {
        Event::SubmitVoice {
            id,
            text,
            duration_ms,
        } => voice_notes::submit_voice(assistant, id, text, duration_ms, cx)?,
        Event::VoiceResume { id } => voice_drafts::resume_detached_voice(assistant, &id, cx)?,
        Event::VoiceSession {
            message_id,
            session_id,
        } => voice_drafts::voice_session(assistant, &message_id, &session_id),
        Event::VoiceStored { id, duration_ms } => {
            voice_notes::voice_stored(assistant, id, duration_ms, cx)?
        }
        Event::VoiceRetry { id } => voice_notes::voice_retry(assistant, &id, cx)?,
        Event::VoiceDiscard { id } => voice_notes::voice_discard(assistant, &id)?,
        Event::VoiceUnheard { id } => voice_notes::forget_voice_note(assistant, &id),
        Event::VoiceFailed { id, failure } => voice_notes::voice_failed(assistant, &id, failure),
        _ => unreachable!("voice event router"),
    }
    Ok(())
}
