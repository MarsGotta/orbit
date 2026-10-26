//! Silence questions use positive observations, never missing audio or a timer
//! in the window. The daemon supplies the existing detector and track monitors.
use crate::audio_signal::{RecordingSignalView, SAMPLE_DELIVERY_TIMEOUT_MS, TrackSignalView};

/// A conversational pause before asking, rather than an overall call bound.
pub const CALL_SILENCE_MS: i64 = 30_000;
/// Time to answer the audible/visible presence question.
pub const CALL_ANSWER_MS: i64 = 15_000;
/// Meetings include ordinary pauses; two minutes avoids asking in short gaps.
pub const RECORDING_SILENCE_MS: u64 = 120_000;
/// Time to keep a forgotten meeting before finishing through the normal drain.
pub const RECORDING_ANSWER_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy)]
pub struct CallQuiet {
    pub silent_ms: i64,
    /// Speech, an open assistant turn, a pending action or approval.
    pub busy: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SilenceQuestions {
    pub call: Question,
    pub recording: Question,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Question {
    owner: String,
    epoch: String,
    asked_at_ms: Option<i64>,
    reset_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Ask,
    Close,
    Withdraw,
}

impl Question {
    pub fn pending(&self) -> bool {
        self.asked_at_ms.is_some()
    }
    pub fn keep(&mut self, now_ms: i64) {
        self.asked_at_ms = None;
        self.reset_at_ms = Some(now_ms);
    }
    pub fn observe(
        &mut self,
        owner: &str,
        epoch: &str,
        quiet: bool,
        wait_ms: i64,
        now: i64,
    ) -> Outcome {
        let replaced_pending = (self.owner != owner || self.epoch != epoch) && self.pending();
        if self.owner != owner || self.epoch != epoch {
            *self = Self {
                owner: owner.into(),
                epoch: epoch.into(),
                ..Self::default()
            };
        }
        if !quiet {
            return if self.asked_at_ms.take().is_some() || replaced_pending {
                Outcome::Withdraw
            } else {
                Outcome::None
            };
        }
        if let Some(asked) = self.asked_at_ms {
            if now < asked {
                self.keep(now);
                return Outcome::Withdraw;
            }
            if now.saturating_sub(asked) >= wait_ms {
                return Outcome::Close;
            }
        } else {
            self.asked_at_ms = Some(now);
            return Outcome::Ask;
        }
        Outcome::None
    }
    pub fn reset_allows(&self, now: i64, pause: i64) -> bool {
        self.reset_at_ms
            .is_none_or(|reset| now.saturating_sub(reset) >= pause)
    }
}

/// Exact short replies are local only while the question is open. Punctuation
/// and case are incidental; phrases with additional words remain normal turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceReply {
    Keep,
    HangUp,
    Turn,
}
pub fn presence_reply(text: &str) -> PresenceReply {
    let words = text
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c == '¿' || c == '¡')
        .to_lowercase();
    match words.as_str() {
        "sí" | "si" | "sigo" | "aquí" | "aqui" | "espera" | "mantén" | "manten" | "no cuelgues" => {
            PresenceReply::Keep
        }
        "cuelga" => PresenceReply::HangUp,
        _ => PresenceReply::Turn,
    }
}

/// Voice uses the call's speech threshold, independently of low-signal warnings. A
/// missing, stale or unknown track cannot certify silence. No speech model is
/// added, and no samples or content are retained by this decision.
pub fn recording_quiet(signal: &RecordingSignalView, muted: bool) -> bool {
    match &signal.remote_app {
        Some(remote) => (muted || track_quiet(&signal.mic_local)) && track_quiet(remote),
        None => !muted && track_quiet(&signal.mic_local),
    }
}
fn track_quiet(track: &TrackSignalView) -> bool {
    matches!(
        track.condition,
        crate::audio_signal::SignalCondition::Receiving
            | crate::audio_signal::SignalCondition::LowSignal
    ) && track
        .since_last_samples_ms
        .is_some_and(|age| age < SAMPLE_DELIVERY_TIMEOUT_MS)
        && track
            .speech_quiet_ms
            .is_some_and(|quiet| quiet >= RECORDING_SILENCE_MS)
}

/// A saved question cannot retain a live countdown after recovery. Withdraw it
/// on the same op so replaying the notice log also clears the window's choice.
pub(crate) fn recovered_notices(log: &crate::notice::NoticeLog) -> Vec<crate::notice::Notice> {
    use crate::notice::{Notice, NoticeCode, Op};
    let mut result = Vec::new();
    for (question, resolved, op) in [
        (
            NoticeCode::CallSilenceQuestion,
            NoticeCode::CallSilenceResolved,
            "call-silence",
        ),
        (
            NoticeCode::RecordingSilenceQuestion,
            NoticeCode::RecordingSilenceResolved,
            "recording-silence",
        ),
    ] {
        if log
            .records()
            .iter()
            .rev()
            .find(|n| n.code == question || n.code == resolved)
            .is_some_and(|n| n.code == question)
        {
            let mut notice = Notice::new(resolved);
            notice.op = Op::parse(op);
            result.push(notice);
        }
    }
    result
}
