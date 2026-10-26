//! New views that were left retain their own waiting notes, never the next
//! view's. Delivery can name that conversation without navigating to it.
use super::AssistantState;

pub(super) fn abandon_new_view(assistant: &mut AssistantState) {
    let draft = assistant
        .pending_voice
        .iter()
        .find(|note| {
            note.conversation.as_ref().is_some_and(|conversation| {
                conversation.session_id.is_none() && conversation.abandoned_draft.is_none()
            })
        })
        .map(|note| note.message.id.clone());
    let Some(draft) = draft else {
        return;
    };
    for conversation in assistant
        .pending_voice
        .iter_mut()
        .filter_map(|note| note.conversation.as_mut())
    {
        if conversation.session_id.is_none() && conversation.abandoned_draft.is_none() {
            conversation.abandoned_draft = Some(draft.clone());
        }
    }
}

pub(in crate::transition) fn voice_session(
    assistant: &mut AssistantState,
    message_id: &str,
    session_id: &str,
) {
    let Some(note) = assistant
        .pending_voice
        .iter()
        .find(|note| note.message.id == message_id && note.detached())
    else {
        return;
    };
    let draft = note
        .conversation
        .as_ref()
        .and_then(|conversation| conversation.abandoned_draft.clone());
    if let Some(draft) = draft {
        for conversation in assistant
            .pending_voice
            .iter_mut()
            .filter_map(|note| note.conversation.as_mut())
        {
            if conversation.abandoned_draft.as_ref() == Some(&draft) {
                conversation.session_id = Some(session_id.to_owned());
            }
        }
    }
    assistant
        .pending_voice
        .retain(|note| note.message.id != message_id);
}

pub(in crate::transition) fn resume_detached_voice(
    assistant: &mut AssistantState,
    id: &str,
    cx: &crate::transition::Step,
) -> Result<(), crate::refusal::Refusal> {
    super::chat::ensure_can_send(assistant, cx)?;
    let voice = assistant
        .pending_voice
        .iter_mut()
        .find(|note| note.message.id == id && note.detached())
        .and_then(|note| note.message.voice.as_mut())
        .filter(|voice| {
            voice.transcript.is_none()
                && voice
                    .failure
                    .as_ref()
                    .is_some_and(|failure| crate::voice_note_waits_for_hub(&failure.code))
        })
        .ok_or(crate::refusal::Refusal::NotFound {
            resource: crate::refusal::Resource::Message,
        })?;
    voice.failure = None;
    Ok(())
}
