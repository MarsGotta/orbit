//! «Mantener»: the floating panel says when the pointer enters and leaves it
//! (`Command::MeetingQuestionHold`), and the question's deadline waits for it
//! (`orbit_core::meeting_question::hold`).
use super::super::Runtime;

impl Runtime {
    /// The pointer entered (`held`) or left the panel showing question `id`.
    /// A question already gone, or another one, is not an error.
    pub(in crate::daemon) fn hold_meeting_question(&mut self, id: &str, held: bool) {
        let now_ms = self.question_now();
        if let Some(question) = self
            .features
            .question
            .current
            .as_mut()
            .filter(|question| question.id == id)
        {
            question.hold(held, now_ms);
        }
    }
}

#[cfg(test)]
#[path = "meeting_question_hold_tests.rs"]
mod tests;
