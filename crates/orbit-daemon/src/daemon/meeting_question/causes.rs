//! Why a question to Deimos never reached it: the panel's cause, and the
//! labels the log names it with. Never the question's words in the log.
use super::super::*;
use super::Meeting;
use orbit_core::meeting_question::{
    AskFacts, BusyTurn, MeetingQuestion, QuestionCause, QuestionOrigin, QuestionStage,
    same_question,
};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};

impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn refuse_question(
        &mut self,
        request_id: String,
        meeting: Option<&Meeting>,
        question: String,
        window: u8,
        origin: QuestionOrigin,
        cause: QuestionCause,
        now_ms: i64,
    ) {
        let thinking = self
            .question
            .current
            .as_ref()
            .filter(|current| current.stage == QuestionStage::Thinking);
        // The helper heard the same words again: that question is the one
        // on its way, and nothing was refused that the user asked.
        let repeated = origin == QuestionOrigin::Voice
            && thinking.is_some_and(|current| same_question(&current.question, &question));
        let thinking = thinking.is_some();
        self.log(
            "meeting_question_refused",
            &[
                ("origin", origin_label(origin)),
                ("cause", cause_label(&cause)),
                ("repeated", if repeated { "yes" } else { "no" }),
            ],
        );
        if repeated {
            return;
        }
        // A question still waiting for Deimos keeps the panel: its answer is
        // on its way. The refused one is said in the notices instead.
        if thinking {
            let notice = if matches!(cause, QuestionCause::TurnInFlight { .. }) {
                Notice::new(NoticeCode::MeetingQuestionStillAnswering)
            } else {
                let notice = Notice::new(NoticeCode::MeetingQuestionNotSent)
                    .with_token(NoticeKey::Cause, cause_label(&cause));
                if let QuestionCause::QuestionTooLong { bytes, max_bytes } = cause {
                    notice
                        .with(NoticeKey::Bytes, bytes)
                        .with(NoticeKey::MaxBytes, max_bytes)
                } else {
                    notice
                }
            };
            self.say(notice);
            return;
        }
        self.question.failure = None;
        self.question.current = Some(MeetingQuestion::refused(
            request_id,
            meeting.map(|meeting| meeting.recording_id.clone()),
            question,
            window,
            origin,
            cause,
            now_ms,
        ));
    }
}

/// The core refused the turn after the admission let it through (the state
/// changed in between): name the same causes.
pub(super) fn submit_refusal_cause(
    refusal: &orbit_core::refusal::Refusal,
    facts: &AskFacts,
) -> QuestionCause {
    use orbit_core::refusal::Refusal as Core;
    // Only these three repeat an admission cause; any other refusal of the
    // core travels with its own sentence.
    if matches!(
        refusal,
        Core::Busy {
            operation: orbit_core::refusal::Operation::Interaction,
            retry_after_ms: None
        }
    ) {
        QuestionCause::TurnInFlight {
            turn: facts.busy.unwrap_or(BusyTurn::AssistantTurn),
        }
    } else if matches!(refusal, Core::HubUnreachable { since_ms: None }) {
        QuestionCause::HubUnreachable {
            connection: facts.connection.clone(),
        }
    } else if matches!(refusal, Core::PrivacyStopped { since_ms: None }) {
        QuestionCause::PrivacyStopped
    } else {
        QuestionCause::HubRefused {
            code: "core_refused".into(),
            message: refusal.to_string(),
        }
    }
}

pub(super) fn origin_label(origin: QuestionOrigin) -> &'static str {
    match origin {
        QuestionOrigin::Voice => "voice",
        QuestionOrigin::Typed => "typed",
        QuestionOrigin::Hotkey => "hotkey",
    }
}

pub(super) fn cause_label(cause: &QuestionCause) -> &'static str {
    match cause {
        QuestionCause::PrivacyStopped => "privacy_stopped",
        QuestionCause::NoMeeting => "no_meeting",
        QuestionCause::HubUnreachable { .. } => "hub_unreachable",
        QuestionCause::DeimosUnavailable { .. } => "deimos_unavailable",
        QuestionCause::TurnInFlight { .. } => "turn_in_flight",
        QuestionCause::QuestionEmpty => "question_empty",
        QuestionCause::QuestionTooLong { .. } => "question_too_long",
        QuestionCause::WindowNotOffered { .. } => "window_not_offered",
        QuestionCause::TurnFailed => "turn_failed",
        QuestionCause::Interrupted => "interrupted",
        QuestionCause::HubRefused { .. } => "hub_refused",
        QuestionCause::NoAnswer => "no_answer",
        QuestionCause::Revoked => "revoked",
    }
}

/// `text` cut at a character boundary to `max` bytes: a diagnostic the
/// helper or the hub sent, never conversation content.
pub(super) fn bounded(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
