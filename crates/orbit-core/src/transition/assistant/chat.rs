//! A written turn: sent, answered as it arrives, completed, or asked a
//! question by the agent.
use super::{AssistantState, voice_notes};
use crate::attachments::Attachment;
use crate::model::{Approval, MAX_TEXT_BYTES, Message, Turn};
use crate::notice::NoticeCode;
use crate::refusal::{Expected, Operation, Parameter, Refusal};
use crate::transition::{Effect, Step};

pub(in crate::transition) fn submit(
    assistant: &mut AssistantState,
    id: String,
    text: String,
    attachments: Vec<Attachment>,
    cx: &mut Step,
) -> Result<(), Refusal> {
    ensure_can_send(assistant, cx)?;
    let received_bytes = text.len();
    let text = text.trim().to_owned();
    // A file alone is a question too: only a turn without files needs text.
    if (text.is_empty() && attachments.is_empty()) || text.len() > MAX_TEXT_BYTES {
        return Err(text_length(received_bytes));
    }
    voice_notes::set_turn(
        assistant,
        Turn {
            user: Message {
                id: id.clone(),
                role: "user".into(),
                text: text.clone(),
                complete: true,
                at_ms: observed_at_ms(cx.now_ms),
                voice: None,
                attachments: attachments.clone(),
            },
            answer: None,
        },
    );
    assistant.active_request = Some(id.clone());
    cx.say(NoticeCode::AssistantThinking);
    cx.effect(Effect::SendText {
        id,
        text,
        voice: None,
        meeting: None,
        attachments: attachments
            .into_iter()
            .map(|attachment| attachment.id)
            .collect(),
    });
    Ok(())
}

/// A turn is sent only by a working Orbit, with the hub online, and one at
/// a time.
pub(super) fn ensure_can_send(assistant: &AssistantState, cx: &Step) -> Result<(), Refusal> {
    cx.before.privacy.ensure_active()?;
    cx.before.link.ensure_online()?;
    if assistant.active_request.is_some() {
        return Err(Refusal::Busy {
            operation: Operation::Interaction,
            retry_after_ms: None,
        });
    }
    Ok(())
}

pub(super) fn text_length(received_bytes: usize) -> Refusal {
    Refusal::InvalidParameter {
        parameter: Parameter::Text,
        expected: Expected::TextLength {
            bytes: received_bytes as u64,
            max_bytes: MAX_TEXT_BYTES as u64,
        },
    }
}

pub(in crate::transition) fn delta(
    assistant: &mut AssistantState,
    request_id: String,
    text: &str,
    cx: &mut Step,
) {
    if assistant.active_request.as_deref() != Some(&request_id) {
        return;
    }
    let Some(turn) = assistant
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == request_id)
    else {
        return;
    };
    if let Some(answer) = turn.answer.as_mut() {
        if answer.text.len().saturating_add(text.len()) <= MAX_TEXT_BYTES * 3 {
            answer.text.push_str(text);
        }
        return;
    }
    let end = clamp_utf8(text, MAX_TEXT_BYTES);
    turn.answer = Some(Message {
        attachments: Vec::new(),
        id: format!("{request_id}:assistant"),
        role: "assistant".into(),
        text: text[..end].to_owned(),
        complete: false,
        at_ms: answer_at_ms(&turn.user, cx.now_ms),
        voice: None,
    });
    cx.say(NoticeCode::AssistantResponding);
}

pub(in crate::transition) fn completed(
    assistant: &mut AssistantState,
    request_id: String,
    text: Option<String>,
    cx: &mut Step,
) {
    if assistant.active_request.as_deref() != Some(&request_id) {
        return;
    }
    assistant.retire_request();
    if let Some(turn) = assistant
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == request_id)
    {
        if let Some(text) = text {
            let end = clamp_utf8(&text, MAX_TEXT_BYTES * 3);
            let text = text[..end].to_owned();
            match turn.answer.as_mut() {
                Some(answer) => answer.text = text,
                None => {
                    turn.answer = Some(Message {
                        attachments: Vec::new(),
                        id: format!("{request_id}:assistant"),
                        role: "assistant".into(),
                        text,
                        complete: true,
                        at_ms: answer_at_ms(&turn.user, cx.now_ms),
                        voice: None,
                    });
                }
            }
        }
        if let Some(answer) = turn.answer.as_mut() {
            answer.complete = true;
        }
    }
    cx.say(NoticeCode::AssistantAnswered);
}

/// A question the agent asks in the turn `turn_id`; one for a turn that is
/// no longer in flight is stale.
pub(in crate::transition) fn approval(
    assistant: &mut AssistantState,
    turn_id: &str,
    approval: Approval,
    cx: &mut Step,
) {
    if assistant.active_request.as_deref() == Some(turn_id) {
        assistant.pending_approval = Some(approval);
    } else {
        cx.say(NoticeCode::AssistantStaleQuestion);
    }
}

pub(super) fn observed_at_ms(now_ms: i64) -> u64 {
    u64::try_from(now_ms).unwrap_or_default()
}

/// The answer is never before the question, whatever the wall clock did.
fn answer_at_ms(user: &Message, now_ms: i64) -> u64 {
    observed_at_ms(now_ms).max(user.at_ms.saturating_add(1))
}

/// Longest byte offset of `text` not exceeding `limit` that lies on a UTF-8
/// character boundary. Treating the byte budget as raw bytes (instead of
/// characters) preserves the `MAX_TEXT_BYTES` cap: cutting
/// `é` (two bytes) by characters would double the budget and a blind cut
/// could split a multi-byte character.
fn clamp_utf8(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
