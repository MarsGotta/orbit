//! Pure, bounded continuous-call turn detection. Energy is a segmentation hint,
//! never authorization, a wake word, speaker identity, or evidence of AEC quality.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    Starting,
    Listening,
    UserSpeaking,
    Waiting,
    Responding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallState {
    pub id: String,
    pub phase: CallPhase,
    pub started_at_ms: i64,
    pub expires_at_ms: i64,
    pub turns: u64,
    /// How the call began. A state saved before this was said loads as
    /// [`CallOrigin::App`]; no call survives a restart (`call_recovery`), so
    /// no live call is ever named by that default.
    #[serde(default)]
    pub origin: CallOrigin,
    /// The person was told the call nears its ceiling: said once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ceiling_notice_given: bool,
}

/// How a call began: from the app («Llamar», «Llamar a Orbit») or with
/// ⌃⌥D from any app. Both last [`CALL_MINUTES`] at most, and ⌃⌥D hangs up
/// the call it started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOrigin {
    #[default]
    App,
    Hotkey,
}

/// How long a call may last, in minutes, from the app or with ⌃⌥D.
///
/// Protects: a microphone left streaming to Orbit by a call nobody hangs up.
/// A forgotten call ends much sooner, a minute without the user's words
/// (`VoicePolicy::idle_timeout_ms`); this is the bound behind that one. Why
/// a recording's ceiling ([`crate::RECORDING_CEILING_MINUTES`], four hours),
/// the same constant: one rule for whatever keeps the microphone open, with
/// the same notice ten minutes before ([`crate::RECORDING_CEILING_NOTICE_MINUTES`]).
/// Two lengths by origin (an hour, a quarter of an hour) had no reason of
/// their own. It does not limit how many calls there are.
pub const CALL_MINUTES: u16 = crate::RECORDING_CEILING_MINUTES;

/// The shortest stretch of speech that can hold a word, a «sí» or a «no».
/// A call turn needs this much speech to be submitted, and a voice note
/// shorter than this is not sent: the transcriber would get the click of a
/// press, from which it can invent text that would reach the assistant as
/// if the user had said it. It limits neither how long a note can be nor the
/// silence inside one.
pub const MINIMUM_SPEECH_MS: u64 = 160;

/// A voice note the daemon did not send because it lasted less than
/// [`MINIMUM_SPEECH_MS`]: its length and that minimum, which the composer
/// says, both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortVoiceNote {
    pub duration_ms: u64,
    pub minimum_ms: u64,
}

/// `Some` when a note of `duration_ms` is too short to send.
pub fn short_voice_note(duration_ms: u64) -> Option<ShortVoiceNote> {
    (duration_ms < MINIMUM_SPEECH_MS).then_some(ShortVoiceNote {
        duration_ms,
        minimum_ms: MINIMUM_SPEECH_MS,
    })
}

/// Fixed conservative defaults for the initial measurable detector. These are
/// not a learned speech classifier, and must be evaluated on real microphones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicePolicy {
    pub preroll_ms: u64,
    pub onset_ms: u64,
    pub barge_in_ms: u64,
    pub release_ms: u64,
    pub minimum_speech_ms: u64,
    pub maximum_turn_ms: u64,
    /// The user's silence that ends a call by itself: a minute without a
    /// word from them while Deimos is neither working on an answer nor
    /// speaking it. Words, not sound: a turn the hub finds no words in (a
    /// fan, a television) is silence too, or a forgotten call in a noisy room
    /// would stay open until its ceiling. It protects the user from an open
    /// microphone streaming to Orbit after the conversation is over. It is
    /// not the time Deimos takes (counted as the user's, it cut a call 20 s
    /// into his spoken answer on 2026-10-18), not muted time, and not the
    /// daemon's check that audio is still arriving (`CALL_AUDIO_SILENCE_LIMIT`).
    pub idle_timeout_ms: i64,
    pub rms_threshold: u16,
}

impl Default for VoicePolicy {
    fn default() -> Self {
        Self {
            preroll_ms: 300,
            onset_ms: 120,
            barge_in_ms: 200,
            release_ms: 700,
            minimum_speech_ms: MINIMUM_SPEECH_MS,
            maximum_turn_ms: 20_000,
            idle_timeout_ms: 60_000,
            rms_threshold: 500,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnAction {
    None,
    Begin { interrupt: bool },
    Finish { submit: bool },
    Stop(StopReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Unauthorized,
    GenerationChanged,
    ProcessingLost,
    Discontinuity,
    Expired,
    /// No words from the user for `idle_timeout_ms` with no answer from
    /// Deimos in course.
    Idle,
    /// No audio arriving from the Mac at all: the daemon's own check, not
    /// the user's silence (their silence is still audio).
    NoAudio,
}

#[derive(Debug, Clone, Copy)]
pub struct VoiceObservation {
    pub generation: u64,
    pub authorized: bool,
    pub processing_observed: bool,
    pub monotonic_ns: u64,
    pub duration_ns: u64,
    pub received_at_ms: i64,
    pub voiced: bool,
    /// Deimos is working on an answer or speaking it.
    pub response_active: bool,
    /// A turn of the user's is being transcribed: its words are not known yet.
    pub transcribing: bool,
    /// The person muted the microphone («Silenciar»): the helper sends
    /// silence, which is neither a turn nor the user's silence.
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnDetector {
    generation: u64,
    expires_at_ms: i64,
    last_activity_ms: i64,
    /// When the last frame arrived, in the frames' received time.
    last_frame_ms: i64,
    last_monotonic_ns: Option<u64>,
    candidate_ns: u64,
    captured_ns: u64,
    spoken_ns: u64,
    quiet_ns: u64,
    capturing: bool,
    stopped: bool,
    suppressed_until_quiet: bool,
}

impl TurnDetector {
    /// Called only after an explicit, durable local CallStart authorization.
    /// This constructor does not grant device access or attest native processing.
    pub fn authorized(generation: u64, started_at_ms: i64, expires_at_ms: i64) -> Self {
        Self {
            generation,
            expires_at_ms,
            last_activity_ms: started_at_ms,
            last_frame_ms: started_at_ms,
            last_monotonic_ns: None,
            candidate_ns: 0,
            captured_ns: 0,
            spoken_ns: 0,
            quiet_ns: 0,
            capturing: false,
            stopped: false,
            suppressed_until_quiet: false,
        }
    }

    pub fn is_capturing(&self) -> bool {
        self.capturing
    }

    /// When the user's words were last heard, Deimos last worked or spoke,
    /// or the microphone was last muted, in the frames' received time: where
    /// their silence starts.
    pub fn last_activity_ms(&self) -> i64 {
        self.last_activity_ms
    }

    /// A turn's words were heard: the user's silence starts again from the
    /// last frame.
    pub fn heard_words(&mut self) {
        self.last_activity_ms = self.last_activity_ms.max(self.last_frame_ms);
    }

    /// Esc discards the current derived turn and waits for a silence boundary;
    /// continuing the same utterance must not immediately submit another turn.
    pub fn cancelled_turn(&self) -> Self {
        let mut next = self.clone();
        next.capturing = false;
        next.candidate_ns = 0;
        next.quiet_ns = 0;
        next.suppressed_until_quiet = true;
        next
    }

    /// Why `frame` stops the call, if it does. The call's ceiling counts
    /// whatever happens in it.
    fn refusal(&self, policy: VoicePolicy, frame: &VoiceObservation) -> Option<StopReason> {
        if self.stopped || !frame.authorized {
            Some(StopReason::Unauthorized)
        } else if frame.generation != self.generation {
            Some(StopReason::GenerationChanged)
        } else if !frame.processing_observed {
            Some(StopReason::ProcessingLost)
        } else if frame.duration_ns == 0
            || frame.duration_ns > 1_000_000_000
            || self
                .last_monotonic_ns
                .is_some_and(|previous| frame.monotonic_ns < previous)
        {
            Some(StopReason::Discontinuity)
        } else if frame.received_at_ms >= self.expires_at_ms {
            Some(StopReason::Expired)
        } else if self.idle(policy, frame) {
            Some(StopReason::Idle)
        } else {
            None
        }
    }

    /// No words for `idle_timeout_ms`, Deimos neither working nor speaking,
    /// the microphone not muted. A turn in course (being captured, or its
    /// words asked for) may still bring words, so it holds the hang-up, but
    /// for one turn's length at most (`maximum_turn_ms`): noise that keeps
    /// opening turns cannot keep a forgotten call open.
    fn idle(&self, policy: VoicePolicy, frame: &VoiceObservation) -> bool {
        if frame.response_active || frame.muted {
            return false;
        }
        let silent = frame.received_at_ms.saturating_sub(self.last_activity_ms);
        let turn_in_course = self.capturing || frame.transcribing;
        let held = if turn_in_course {
            i64::try_from(policy.maximum_turn_ms).unwrap_or(i64::MAX)
        } else {
            0
        };
        silent >= policy.idle_timeout_ms.saturating_add(held)
    }

    /// Pure transition: the caller retains the previous value on adapter failure.
    pub fn observe(&self, policy: VoicePolicy, frame: VoiceObservation) -> (Self, TurnAction) {
        let mut next = self.clone();
        if let Some(reason) = self.refusal(policy, &frame) {
            next.stopped = true;
            next.capturing = false;
            return (next, TurnAction::Stop(reason));
        }
        // What a muted microphone sends is silence, whatever its level.
        let frame = VoiceObservation {
            voiced: frame.voiced && !frame.muted,
            ..frame
        };
        next.last_monotonic_ns = Some(frame.monotonic_ns);
        next.last_frame_ms = frame.received_at_ms;
        if frame.response_active || frame.muted {
            // Deimos working or speaking is the conversation going on, and a
            // muted microphone is not the user's silence: it counts from
            // when he is done, or from the unmute.
            next.last_activity_ms = frame.received_at_ms;
        }
        if next.suppressed_until_quiet {
            next.quiet_ns = if frame.voiced {
                0
            } else {
                next.quiet_ns.saturating_add(frame.duration_ns)
            };
            if next.quiet_ns >= policy.release_ms * 1_000_000 {
                next.suppressed_until_quiet = false;
                next.quiet_ns = 0;
            }
            return (next, TurnAction::None);
        }
        if self.capturing {
            next.captured_ns = next.captured_ns.saturating_add(frame.duration_ns);
            if frame.voiced {
                next.spoken_ns = next.spoken_ns.saturating_add(frame.duration_ns);
                next.quiet_ns = 0;
            } else {
                next.quiet_ns = next.quiet_ns.saturating_add(frame.duration_ns);
            }
            if next.quiet_ns >= policy.release_ms * 1_000_000
                || next.captured_ns >= policy.maximum_turn_ms * 1_000_000
            {
                let submit = next.spoken_ns >= policy.minimum_speech_ms * 1_000_000;
                next.capturing = false;
                next.candidate_ns = 0;
                return (next, TurnAction::Finish { submit });
            }
        } else {
            next.candidate_ns = if frame.voiced {
                next.candidate_ns.saturating_add(frame.duration_ns)
            } else {
                0
            };
            // Over Deimos, or over a turn still being heard, a turn needs
            // the longer evidence of a barge-in.
            let over_answer = frame.response_active || frame.transcribing;
            let threshold = if over_answer {
                policy.barge_in_ms
            } else {
                policy.onset_ms
            };
            if next.candidate_ns >= threshold * 1_000_000 {
                next.capturing = true;
                next.captured_ns = next.candidate_ns;
                next.spoken_ns = next.candidate_ns;
                next.quiet_ns = 0;
                return (
                    next,
                    TurnAction::Begin {
                        interrupt: over_answer,
                    },
                );
            }
        }
        (next, TurnAction::None)
    }
}

/// The RMS of a little-endian 16-bit frame, in sample units (0..=32 768):
/// what `energy_exceeds` compares with its threshold. 0 for an empty or odd
/// frame.
pub fn rms(pcm_s16le: &[u8]) -> u32 {
    if pcm_s16le.is_empty() || !pcm_s16le.len().is_multiple_of(2) {
        return 0;
    }
    let samples = (pcm_s16le.len() / 2) as u128;
    let energy: u128 = pcm_s16le
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let sample = i128::from(i16::from_le_bytes([pair[0], pair[1]]));
            (sample * sample) as u128
        })
        .sum();
    // Integer square root of the mean square: at most 32 768, so u32.
    let mean = energy / samples;
    let mut root = (mean as f64).sqrt() as u128;
    while root * root > mean {
        root -= 1;
    }
    while (root + 1) * (root + 1) <= mean {
        root += 1;
    }
    u32::try_from(root).unwrap_or(u32::MAX)
}

/// Integer mean-square gate over decoded mono PCM16. This is deliberately named
/// energy, not speaker verification or echo cancellation. No floating-point NaN.
pub fn energy_exceeds(pcm_s16le: &[u8], rms_threshold: u16) -> bool {
    if pcm_s16le.is_empty() || !pcm_s16le.len().is_multiple_of(2) {
        return false;
    }
    let energy: u128 = pcm_s16le
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let sample = i128::from(i16::from_le_bytes([pair[0], pair[1]]));
            (sample * sample) as u128
        })
        .sum();
    energy >= u128::from(rms_threshold).pow(2) * (pcm_s16le.len() / 2) as u128
}

#[cfg(test)]
#[path = "voice_tests.rs"]
#[allow(clippy::expect_used)]
mod tests;
