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
    /// Retired wire field: new calls use i64::MAX; never a duration decision.
    pub expires_at_ms: i64,
    pub turns: u64,
    /// How the call began. A state saved before this was said loads as
    /// [`CallOrigin::App`]; no call survives a restart (`call_recovery`), so
    /// no live call is ever named by that default.
    #[serde(default)]
    pub origin: CallOrigin,
    /// Retired snapshot field, retained for old readers. Never consulted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ceiling_notice_given: bool,
}

/// How a call began: from the app («Llamar», «Llamar a Orbit») or with
/// ⌃⌥D from any app. Both last [the retired duration field] at most, and ⌃⌥D hangs up
/// the call it started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOrigin {
    #[default]
    App,
    Hotkey,
}

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
    /// Historical cancellation reason; new calls do not expire by duration.
    Expired,
    /// Retired immediate idle stop, kept for historical cancellation records.
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
    pub fn authorized(generation: u64, started_at_ms: i64, _expires_at_ms: i64) -> Self {
        Self {
            generation,
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

    /// Why the frame cannot be accepted: integrity and authority only.
    fn refusal(&self, _policy: VoicePolicy, frame: &VoiceObservation) -> Option<StopReason> {
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
        } else {
            None
        }
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
