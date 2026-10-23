//! Voice notes in the conversation: a sealed note becomes the turn's
//! message or waits on this Mac in the conversation it was spoken in, and is
//! sent with its transcript, retried, discarded or dropped when nothing was
//! heard in it.
use super::AssistantState;
use super::chat::{ensure_can_send, observed_at_ms, text_length};
use crate::model::*;
use crate::notice::NoticeCode;
use crate::refusal::{Operation, Refusal, Resource};
use crate::transition::{Effect, Step};

/// The new conversation of the open agent has just been named `id`: the
/// notes spoken in it before it existed are now that conversation's.
pub(super) fn adopt_new_conversation_notes(assistant: &mut AssistantState, id: &str) {
    let agent = assistant.agent_id.clone();
    for conversation in assistant
        .pending_voice
        .iter_mut()
        .filter_map(|note| note.conversation.as_mut())
        .filter(|conversation| conversation.session_id.is_none() && conversation.agent_id == agent)
    {
        conversation.session_id = Some(id.to_owned());
    }
}

/// The transcript of a voice note is sent as the turn. The note stored
/// before its transcription becomes the turn's message; a dictation without
/// one starts its own turn.
pub(in crate::transition) fn submit_voice(
    assistant: &mut AssistantState,
    id: String,
    text: String,
    duration_ms: u64,
    cx: &mut Step,
) -> Result<(), Refusal> {
    ensure_can_send(assistant, cx)?;
    let received_bytes = text.len();
    let text = text.trim().to_owned();
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(text_length(received_bytes));
    }
    let voice = VoiceNote {
        duration_ms,
        transcript: Some(text.clone()),
        failure: None,
        audio_available: true,
    };
    match assistant
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == id && turn.user.voice.is_some())
    {
        Some(turn) => {
            turn.user.text.clone_from(&text);
            turn.user.voice = Some(voice);
        }
        None => {
            assistant.turn = Some(Turn {
                user: Message {
                    attachments: Vec::new(),
                    id: id.clone(),
                    role: "user".into(),
                    text: text.clone(),
                    complete: true,
                    at_ms: observed_at_ms(cx.now_ms),
                    voice: Some(voice),
                },
                answer: None,
            });
        }
    }
    assistant.active_request = Some(id.clone());
    cx.say(NoticeCode::AssistantThinking);
    cx.effect(Effect::SendText {
        attachments: Vec::new(),
        id,
        text,
        voice: Some(crate::wire::hub::ChatVoice { duration_ms }),
        meeting: None,
    });
    Ok(())
}

/// A voice note sealed on this Mac becomes a message, with the hub or
/// without it: the WAV is kept, and its transcription can wait. With a turn
/// still in flight it waits in `pending_voice` instead of replacing it.
pub(in crate::transition) fn voice_stored(
    assistant: &mut AssistantState,
    id: String,
    duration_ms: u64,
    cx: &mut Step,
) -> Result<(), Refusal> {
    cx.before.privacy.ensure_active()?;
    let known = assistant
        .turn
        .iter()
        .map(|turn| &turn.user)
        .chain(assistant.pending_voice.iter().map(|note| &note.message))
        .any(|message| message.id == id);
    if known {
        return Err(busy(Operation::Interaction));
    }
    let busy = assistant.active_request.is_some();
    let message = Message {
        attachments: Vec::new(),
        id,
        role: "user".into(),
        text: String::new(),
        complete: true,
        at_ms: observed_at_ms(cx.now_ms),
        voice: Some(VoiceNote {
            duration_ms,
            transcript: None,
            failure: busy.then(|| VoiceTranscriptionFailure::local(VOICE_WAITING_TURN)),
            audio_available: true,
        }),
    };
    if busy {
        keep_pending_voice(assistant, message);
    } else {
        set_turn(
            assistant,
            Turn {
                user: message,
                answer: None,
            },
        );
    }
    cx.say(NoticeCode::VoiceNoteSaved);
    Ok(())
}

/// The turn becomes `turn`. A voice note the hub never took that it
/// replaces stays on the Mac, in `pending_voice`, instead of being dropped.
pub(super) fn set_turn(assistant: &mut AssistantState, turn: Turn) {
    retire_turn(assistant);
    assistant.turn = Some(turn);
}

/// The turn ends without another (a new or another conversation). Its
/// voice note the hub never took is kept, as when a turn replaces it.
pub(super) fn retire_turn(assistant: &mut AssistantState) {
    if let Some(message) = assistant
        .turn
        .take()
        .and_then(|turn| turn.untranscribed_voice_message().cloned())
    {
        keep_pending_voice(assistant, message);
    }
}

/// Keeps `message` waiting on this Mac as a note of the open conversation,
/// where it was spoken.
fn keep_pending_voice(assistant: &mut AssistantState, message: Message) {
    let conversation = crate::VoiceNoteConversation {
        session_id: assistant.session_id.clone(),
        agent_id: assistant.agent_id.clone(),
    };
    let pending = &mut assistant.pending_voice;
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
pub(in crate::transition) fn voice_discard(
    assistant: &mut AssistantState,
    id: &str,
) -> Result<(), Refusal> {
    let failed = |message: &Message| {
        message
            .voice
            .as_ref()
            .filter(|voice| voice.transcript.is_none())
            .map(|voice| voice.failure.is_some())
    };
    let found = assistant
        .turn
        .as_ref()
        .map(|turn| &turn.user)
        .filter(|message| message.id == id)
        .or_else(|| {
            assistant
                .pending_voice
                .iter()
                .map(|note| &note.message)
                .find(|message| message.id == id)
        })
        .and_then(failed);
    match found {
        None => Err(Refusal::NotFound {
            resource: Resource::Message,
        }),
        Some(false) => Err(busy(Operation::Interaction)),
        Some(true) => {
            forget_voice_note(assistant, id);
            Ok(())
        }
    }
}

/// The voice note `id` leaves the turn, which it was the start of, and the
/// notes waiting on this Mac. Nothing was heard in it: it was noise, or
/// part of the next turn.
pub(in crate::transition) fn forget_voice_note(assistant: &mut AssistantState, id: &str) {
    if assistant
        .turn
        .as_ref()
        .is_some_and(|turn| turn.user.id == id && turn.untranscribed_voice_message().is_some())
    {
        assistant.turn = None;
    }
    assistant.pending_voice.retain(|note| note.message.id != id);
}

/// A voice note's transcription asked for again: the open turn's note `id`
/// that failed, while nothing else uses the microphone or the turn. Its
/// failure is cleared so the note reads as being transcribed; the daemon has
/// already checked that this Mac still holds its audio.
pub(in crate::transition) fn voice_retry(
    assistant: &mut AssistantState,
    id: &str,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let before = cx.before;
    before.privacy.ensure_active()?;
    before.link.ensure_online()?;
    if before.voice.in_call() {
        return Err(busy(Operation::AudioSession));
    }
    if before.voice.recording_voice_note()
        || assistant.active_request.is_some()
        || assistant.pending_approval.is_some()
    {
        return Err(busy(Operation::Interaction));
    }
    if let Some(at) = assistant.pending_voice.iter().position(|note| {
        note.message.id == id
            && note.belongs_to(assistant)
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
        let mut message = assistant.pending_voice.remove(at).message;
        if let Some(voice) = message.voice.as_mut() {
            voice.failure = None;
        }
        set_turn(
            assistant,
            Turn {
                user: message,
                answer: None,
            },
        );
        cx.say(NoticeCode::VoiceTranscribing);
        return Ok(());
    }
    let voice = assistant
        .turn
        .as_mut()
        .and_then(|turn| turn.untranscribed_voice_mut(id))
        .filter(|voice| voice.failure.is_some())
        .ok_or(Refusal::NotFound {
            resource: Resource::Message,
        })?;
    voice.failure = None;
    cx.say(NoticeCode::VoiceTranscribing);
    Ok(())
}

/// A voice note's transcription failed: the open turn's note, or one
/// waiting on this Mac, says why. A note with words is never failed.
pub(in crate::transition) fn voice_failed(
    assistant: &mut AssistantState,
    id: &str,
    failure: VoiceTranscriptionFailure,
) {
    if let Some(voice) = assistant
        .turn
        .as_mut()
        .and_then(|turn| turn.untranscribed_voice_mut(id))
        .or_else(|| {
            assistant
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

fn busy(operation: Operation) -> Refusal {
    Refusal::Busy {
        operation,
        retry_after_ms: None,
    }
}
