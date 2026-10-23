//! The pointer over the floating panel holds its answer (proposal 19 of the
//! redesign). The daemon lets the question go at `shown_until_ms`, so the
//! hold is the daemon's: the panel says when the pointer enters and leaves,
//! and the deadline waits for it.
use super::MeetingQuestion;

/// How long the answer stays once the pointer leaves the panel, when its own
/// time is already over: enough to come back to it with the pointer, short
/// enough that it then goes without being closed. It never shortens the time
/// the answer had left (`ANSWER_SHOWN_MS`), and it is not a retry interval.
pub const HOLD_GRACE_MS: i64 = 5_000;

impl MeetingQuestion {
    /// The pointer entered (`held`) or left the panel at `now_ms`. A leave
    /// without an enter before it changes nothing.
    pub fn hold(&mut self, held: bool, now_ms: i64) {
        if !held && self.held {
            self.released_at_ms = Some(now_ms);
        }
        self.held = held;
    }
}
