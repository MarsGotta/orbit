// main's `transition/voice_notes.rs`, copied verbatim; only imports differ. Test-only.
//! Voice notes in the core: a sealed note becomes the turn's message or
//! waits on this Mac in the conversation it was spoken in, and is retried,
//! discarded or dropped when nothing was heard in it.
use super::{ensure_active, observed_at_ms};
use crate::model::*;
use crate::notice::NoticeCode;
use crate::refusal::Refusal;
use crate::transition::equivalence::legacy::model::State;

/// The new conversation of the open agent has just been named `id`: the
/// notes spoken in it before it existed are now that conversation's.
pub(super) fn adopt_new_conversation_notes(next: &mut State, id: &str) {
    let agent = next.durable.agent_id.clone();
    for conversation in next
        .durable
        .pending_voice
        .iter_mut()
        .filter_map(|note| note.conversation.as_mut())
        .filter(|conversation| conversation.session_id.is_none() && conversation.agent_id == agent)
    {
        conversation.session_id = Some(id.to_owned());
    }
}

/// A voice note sealed on this Mac becomes a message, with the hub or
/// without it: the WAV is kept, and its transcription can wait. With a turn
/// still in flight it waits in `pending_voice` instead of replacing it.
pub(super) fn decide_voice_stored(
    state: &State,
    next: &mut State,
    id: String,
    duration_ms: u64,
    now_ms: i64,
) -> Result<(), Refusal> {
    ensure_active(state)?;
    let known = next
        .durable
        .turn
        .iter()
        .map(|turn| &turn.user)
        .chain(next.durable.pending_voice.iter().map(|note| &note.message))
        .any(|message| message.id == id);
    if known {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    let busy = state.durable.active_request.is_some();
    let message = Message {
        attachments: Vec::new(),
        id,
        role: "user".into(),
        text: String::new(),
        complete: true,
        at_ms: observed_at_ms(now_ms),
        voice: Some(VoiceNote {
            duration_ms,
            transcript: None,
            failure: busy.then(|| VoiceTranscriptionFailure::local(VOICE_WAITING_TURN)),
            audio_available: true,
        }),
    };
    if busy {
        keep_pending_voice(next, message);
    } else {
        set_turn(
            next,
            Turn {
                user: message,
                answer: None,
            },
        );
    }
    next.set_notice(NoticeCode::VoiceNoteSaved);
    Ok(())
}

/// The turn becomes `turn`. A voice note the hub never took that it
/// replaces stays on the Mac, in `pending_voice`, instead of being dropped.
pub(super) fn set_turn(next: &mut State, turn: Turn) {
    retire_turn(next);
    next.durable.turn = Some(turn);
}

/// The turn ends without another (a new or another conversation). Its
/// voice note the hub never took is kept, as when a turn replaces it.
pub(super) fn retire_turn(next: &mut State) {
    if let Some(message) = next
        .durable
        .turn
        .take()
        .and_then(|turn| turn.untranscribed_voice_message().cloned())
    {
        keep_pending_voice(next, message);
    }
}

/// Keeps `message` waiting on this Mac as a note of the open conversation,
/// where it was spoken.
pub(super) fn keep_pending_voice(next: &mut State, message: Message) {
    let conversation = crate::VoiceNoteConversation {
        session_id: next.durable.session_id.clone(),
        agent_id: next.durable.agent_id.clone(),
    };
    let pending = &mut next.durable.pending_voice;
    if pending.iter().all(|kept| kept.message.id != message.id) {
        pending.push(crate::PendingVoiceNote {
            message,
            conversation: Some(conversation),
        });
    }
}

/// «Descartar»: a note whose transcription failed leaves the open turn or
/// the notes waiting on this Mac. A note still being transcribed has no
/// failure yet and is refused as busy; a note with words is a message, and
/// is not a voice note to discard.
pub(super) fn decide_voice_discard(next: &mut State, id: &str) -> Result<(), Refusal> {
    let failed = |message: &Message| {
        message
            .voice
            .as_ref()
            .filter(|voice| voice.transcript.is_none())
            .map(|voice| voice.failure.is_some())
    };
    let found = next
        .durable
        .turn
        .as_ref()
        .map(|turn| &turn.user)
        .filter(|message| message.id == id)
        .or_else(|| {
            next.durable
                .pending_voice
                .iter()
                .map(|note| &note.message)
                .find(|message| message.id == id)
        })
        .and_then(failed);
    match found {
        None => Err(Refusal::NotFound {
            resource: crate::refusal::Resource::Message,
        }),
        Some(false) => Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        }),
        Some(true) => {
            forget_voice_note(next, id);
            Ok(())
        }
    }
}

/// The voice note `id` leaves the turn, which it was the start of, and the
/// notes waiting on this Mac.
pub(super) fn forget_voice_note(next: &mut State, id: &str) {
    if next
        .durable
        .turn
        .as_ref()
        .is_some_and(|turn| turn.user.id == id && turn.untranscribed_voice_message().is_some())
    {
        next.durable.turn = None;
    }
    next.durable
        .pending_voice
        .retain(|note| note.message.id != id);
}

/// A voice note's transcription asked for again: the open turn's note `id`
/// that failed, while nothing else uses the microphone or the turn. Its
/// failure is cleared so the note reads as being transcribed; the daemon has
/// already checked that this Mac still holds its audio.
pub(super) fn decide_voice_retry(state: &State, next: &mut State, id: &str) -> Result<(), Refusal> {
    ensure_active(state)?;
    if state.durable.connection != Connection::Online {
        return Err(Refusal::HubUnreachable { since_ms: None });
    }
    if state.durable.call.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::AudioSession,
            retry_after_ms: None,
        });
    }
    if state.durable.voice_active
        || state.durable.active_request.is_some()
        || state.durable.pending_approval.is_some()
    {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    if let Some(at) = next.durable.pending_voice.iter().position(|note| {
        note.message.id == id
            && crate::transition::equivalence::legacy::model::belongs_to(note, &next.durable)
            && note
                .message
                .voice
                .as_ref()
                .is_some_and(|voice| voice.transcript.is_none())
    }) {
        // A note a later turn replaced becomes the turn again, so its
        // transcript is sent as it would have been. Only in the
        // conversation it was spoken in: sent from another, it would land
        // in the wrong one.
        let mut message = next.durable.pending_voice.remove(at).message;
        if let Some(voice) = message.voice.as_mut() {
            voice.failure = None;
        }
        set_turn(
            next,
            Turn {
                user: message,
                answer: None,
            },
        );
        next.set_notice(NoticeCode::VoiceTranscribing);
        return Ok(());
    }
    let voice = next
        .durable
        .turn
        .as_mut()
        .and_then(|turn| turn.untranscribed_voice_mut(id))
        .filter(|voice| voice.failure.is_some())
        .ok_or(Refusal::NotFound {
            resource: crate::refusal::Resource::Message,
        })?;
    voice.failure = None;
    next.set_notice(NoticeCode::VoiceTranscribing);
    Ok(())
}

/// A voice note's transcription failed: the open turn's note, or one
/// waiting on this Mac, says why. A note with words is never failed.
pub(super) fn decide_voice_failed(next: &mut State, id: &str, failure: VoiceTranscriptionFailure) {
    if let Some(voice) = next
        .durable
        .turn
        .as_mut()
        .and_then(|turn| turn.untranscribed_voice_mut(id))
        .or_else(|| {
            next.durable
                .pending_voice
                .iter_mut()
                .find(|note| note.message.id == id)
                .and_then(|note| note.message.voice.as_mut())
                .filter(|voice| voice.transcript.is_none())
        })
    {
        voice.failure = Some(failure);
    }
}
