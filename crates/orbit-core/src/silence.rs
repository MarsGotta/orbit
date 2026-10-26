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
    voiced_frames: [Option<u64>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Ask,
    Close,
    Withdraw,
}

/// Unknown preserves a pending deadline but cannot ask or close. Only positive
/// activity withdraws it. A fresh quiet sample may close an expired question
/// even when a delivery gap restarted the pre-question quiet interval.
#[derive(Debug, Clone, Copy)]
pub enum Observation {
    Unknown,
    Active,
    Quiet { can_ask: bool },
}

impl Question {
    pub fn pending(&self) -> bool {
        self.asked_at_ms.is_some()
    }
    pub fn keep(&mut self, now_ms: i64) {
        self.asked_at_ms = None;
        self.reset_at_ms = Some(now_ms);
    }
    fn bind(&mut self, owner: &str, epoch: Option<&str>) {
        if self.owner != owner {
            *self = Self {
                owner: owner.into(),
                ..Self::default()
            };
        }
        if let Some(epoch) = epoch.filter(|epoch| *epoch != self.epoch) {
            self.epoch = epoch.into();
            self.voiced_frames = [None; 2];
        }
    }
    pub fn observe(
        &mut self,
        owner: &str,
        observation: Observation,
        wait_ms: i64,
        now: i64,
    ) -> Outcome {
        self.bind(owner, None);
        match observation {
            Observation::Unknown => return Outcome::None,
            Observation::Active => {
                let pending = self.pending();
                self.keep(now);
                return if pending {
                    Outcome::Withdraw
                } else {
                    Outcome::None
                };
            }
            Observation::Quiet { can_ask } if !self.pending() && !can_ask => return Outcome::None,
            Observation::Quiet { .. } => {}
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
    pub fn observe_recording(
        &mut self,
        owner: &str,
        signal: Option<&RecordingSignalView>,
        muted: bool,
        now: i64,
    ) -> Outcome {
        self.bind(owner, signal.map(|s| s.capture_epoch_id.as_str()));
        let observation = signal.map_or(Observation::Unknown, |s| {
            let mut quiet = false;
            let mut active = false;
            for (index, track) in [Some(&s.mic_local), s.remote_app.as_ref()]
                .into_iter()
                .enumerate()
            {
                let Some(track) = track else { continue };
                let ignored = index == 0 && muted;
                let fresh = fresh_quiet_ms(track);
                if let Some(frames) = track.voiced_frames.filter(|_| fresh.is_some()) {
                    let previous = self.voiced_frames[index].replace(frames);
                    active |= !ignored && frames > previous.unwrap_or(0);
                }
                if !ignored {
                    let side_quiet = fresh.is_some();
                    quiet = if index == 0 || muted {
                        side_quiet
                    } else {
                        quiet && side_quiet
                    };
                }
            }
            if active {
                Observation::Active
            } else if quiet {
                Observation::Quiet {
                    can_ask: recording_quiet(s, muted)
                        && self.reset_allows(now, RECORDING_SILENCE_MS as i64),
                }
            } else {
                Observation::Unknown
            }
        });
        self.observe(owner, observation, RECORDING_ANSWER_MS, now)
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
    fresh_quiet_ms(track).is_some_and(|quiet| quiet >= RECORDING_SILENCE_MS)
}
fn fresh_quiet_ms(track: &TrackSignalView) -> Option<u64> {
    (matches!(
        track.condition,
        crate::audio_signal::SignalCondition::Receiving
            | crate::audio_signal::SignalCondition::LowSignal
    ) && track
        .since_last_samples_ms
        .is_some_and(|age| age < SAMPLE_DELIVERY_TIMEOUT_MS)
        && track.voiced_frames.is_some())
    .then_some(track.speech_quiet_ms)
    .flatten()
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
