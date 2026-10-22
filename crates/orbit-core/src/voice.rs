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
}

/// How a call began: from the app («Llamar», «Llamar a Orbit») or with
/// ⌃⌥D from any app. Each has its own length ([`CALL_MINUTES`],
/// [`HOTKEY_CALL_MINUTES`]), and ⌃⌥D hangs up the call it started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOrigin {
    #[default]
    App,
    Hotkey,
}

/// How long a call started from the app may last, in minutes.
///
/// Protects: a microphone left streaming to Orbit by a call nobody hangs up
/// (the user's silence ends it sooner: `VoicePolicy::idle_timeout_ms`).
/// Why an hour: the longest call the core admits (`decide_call`); the app
/// says what is left («quedan 55 min»), so a longer conversation is a new
/// call. It does not limit how many calls there are.
pub const CALL_MINUTES: u16 = 60;

/// How long a call started with ⌃⌥D may last, in minutes.
///
/// Protects the same as [`CALL_MINUTES`] for a call begun from another app,
/// often without Orbit's window in sight: a quick question, so a quarter of
/// an hour, after which ⌃⌥D starts another. It does not limit a call started
/// from the app.
pub const HOTKEY_CALL_MINUTES: u16 = 15;

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
    /// The user's silence that ends a call by itself: a minute in which they
    /// say nothing and Deimos is neither working on an answer nor speaking it.
    /// It protects the user from an open microphone streaming to Orbit after
    /// the conversation is over. It is not the time Deimos takes (counted as
    /// the user's, it cut a call 20 s into his spoken answer on 2026-10-18) and not the
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
    /// The user silent for `idle_timeout_ms` with no answer from Deimos in
    /// course.
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
    pub response_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnDetector {
    generation: u64,
    expires_at_ms: i64,
    last_activity_ms: i64,
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

    /// When the user last spoke, or Deimos last answered, in the frames'
    /// received time: where their silence starts.
    pub fn last_activity_ms(&self) -> i64 {
        self.last_activity_ms
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

    /// Pure transition: the caller retains the previous value on adapter failure.
    pub fn observe(&self, policy: VoicePolicy, frame: VoiceObservation) -> (Self, TurnAction) {
        let mut next = self.clone();
        let refusal = if self.stopped || !frame.authorized {
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
        } else if !self.capturing
            && !frame.response_active
            && frame.received_at_ms.saturating_sub(self.last_activity_ms) >= policy.idle_timeout_ms
        {
            Some(StopReason::Idle)
        } else {
            None
        };
        if let Some(reason) = refusal {
            next.stopped = true;
            next.capturing = false;
            return (next, TurnAction::Stop(reason));
        }
        next.last_monotonic_ns = Some(frame.monotonic_ns);
        if frame.response_active {
            // Deimos working or speaking is the conversation going on; their
            // silence counts from when he is done.
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
                next.last_activity_ms = frame.received_at_ms;
            } else {
                next.quiet_ns = next.quiet_ns.saturating_add(frame.duration_ns);
            }
            if next.quiet_ns >= policy.release_ms * 1_000_000
                || next.captured_ns >= policy.maximum_turn_ms * 1_000_000
            {
                let submit = next.spoken_ns >= policy.minimum_speech_ms * 1_000_000;
                next.capturing = false;
                next.candidate_ns = 0;
                next.last_activity_ms = frame.received_at_ms;
                return (next, TurnAction::Finish { submit });
            }
        } else {
            next.candidate_ns = if frame.voiced {
                next.candidate_ns.saturating_add(frame.duration_ns)
            } else {
                0
            };
            let threshold = if frame.response_active {
                policy.barge_in_ms
            } else {
                policy.onset_ms
            };
            if next.candidate_ns >= threshold * 1_000_000 {
                next.capturing = true;
                next.captured_ns = next.candidate_ns;
                next.spoken_ns = next.candidate_ns;
                next.quiet_ns = 0;
                next.last_activity_ms = frame.received_at_ms;
                return (
                    next,
                    TurnAction::Begin {
                        interrupt: frame.response_active,
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
mod tests {
    use super::*;
    fn frame(index: u64, voiced: bool) -> VoiceObservation {
        VoiceObservation {
            generation: 5,
            authorized: true,
            processing_observed: true,
            monotonic_ns: index * 20_000_000,
            duration_ns: 20_000_000,
            received_at_ms: index as i64 * 20,
            voiced,
            response_active: false,
        }
    }
    #[test]
    fn explicit_gate_and_processing_are_required_even_for_loud_audio() {
        let state = TurnDetector::authorized(5, 0, 60_000);
        for (observation, expected) in [
            (
                VoiceObservation {
                    authorized: false,
                    ..frame(0, true)
                },
                StopReason::Unauthorized,
            ),
            (
                VoiceObservation {
                    processing_observed: false,
                    ..frame(0, true)
                },
                StopReason::ProcessingLost,
            ),
            (
                VoiceObservation {
                    generation: 6,
                    ..frame(0, true)
                },
                StopReason::GenerationChanged,
            ),
        ] {
            let (stopped, action) = state.observe(VoicePolicy::default(), observation);
            assert_eq!(action, TurnAction::Stop(expected));
            assert_eq!(
                stopped.observe(VoicePolicy::default(), frame(1, true)).1,
                TurnAction::Stop(StopReason::Unauthorized)
            );
        }
    }
    #[test]
    fn onset_release_and_minimum_speech_are_measured_in_samples_duration() {
        let policy = VoicePolicy::default();
        let mut state = TurnDetector::authorized(5, 0, 60_000);
        for index in 0..5 {
            (state, _) = state.observe(policy, frame(index, true));
            assert!(!state.is_capturing());
        }
        let action;
        (state, action) = state.observe(policy, frame(5, true));
        assert_eq!(action, TurnAction::Begin { interrupt: false });
        for index in 6..8 {
            (state, _) = state.observe(policy, frame(index, true));
        }
        for index in 8..42 {
            (state, _) = state.observe(policy, frame(index, false));
            assert!(state.is_capturing());
        }
        assert_eq!(
            state.observe(policy, frame(42, false)).1,
            TurnAction::Finish { submit: true }
        );
    }
    #[test]
    fn barge_needs_longer_evidence_and_does_not_change_generation() {
        let mut state = TurnDetector::authorized(5, 0, 60_000);
        for index in 0..9 {
            let action;
            (state, action) = state.observe(
                VoicePolicy::default(),
                VoiceObservation {
                    response_active: true,
                    ..frame(index, true)
                },
            );
            assert_eq!(action, TurnAction::None);
        }
        let (next, action) = state.observe(
            VoicePolicy::default(),
            VoiceObservation {
                response_active: true,
                ..frame(9, true)
            },
        );
        assert_eq!(action, TurnAction::Begin { interrupt: true });
        assert_eq!(next.generation, 5);
    }
    #[test]
    fn continuous_sound_is_bounded_and_silence_expires_without_auto_resume() {
        let policy = VoicePolicy::default();
        let mut state = TurnDetector::authorized(5, 0, 120_000);
        let mut last = TurnAction::None;
        for index in 0..1000 {
            (state, last) = state.observe(policy, frame(index, true));
        }
        assert_eq!(last, TurnAction::Finish { submit: true });
        assert_eq!(
            TurnDetector::authorized(5, 0, 120_000)
                .observe(policy, frame(3000, false))
                .1,
            TurnAction::Stop(StopReason::Idle)
        );
        assert_eq!(
            TurnDetector::authorized(5, 0, 100)
                .observe(policy, frame(5, true))
                .1,
            TurnAction::Stop(StopReason::Expired)
        );
    }
    /// A call of 2026-10-18 stopped `idle` 20 s into Deimos's spoken answer,
    /// after ~40 s of tool work: a minute after the user's last turn.
    /// Deimos working or speaking is not the user's silence; the minute counts from
    /// when he is done.
    #[test]
    fn deimos_working_or_speaking_is_not_user_silence() {
        let policy = VoicePolicy::default();
        let idle = policy.idle_timeout_ms as u64;
        let at = |ms: u64, response_active: bool| VoiceObservation {
            monotonic_ns: ms * 1_000_000,
            received_at_ms: ms as i64,
            response_active,
            ..frame(0, false)
        };
        let state = TurnDetector::authorized(5, 0, 10 * idle as i64);
        // A minute and more after the user's last activity, Deimos still answering.
        let (state, action) = state.observe(policy, at(idle, true));
        assert_eq!(action, TurnAction::None);
        let (state, action) = state.observe(policy, at(idle + 20_000, true));
        assert_eq!(action, TurnAction::None);
        // He is done at idle + 20 s: their silence starts there.
        let (state, action) = state.observe(policy, at(idle + 20_000 + idle - 1, false));
        assert_eq!(action, TurnAction::None);
        assert_eq!(
            state.observe(policy, at(idle + 20_000 + idle, false)).1,
            TurnAction::Stop(StopReason::Idle)
        );
    }
    #[test]
    fn pcm_energy_is_bounded_and_not_a_name_or_wake_word() {
        assert!(!energy_exceeds(&[0; 100], 500));
        assert!(energy_exceeds(&i16::MIN.to_le_bytes(), 500));
        assert!(!energy_exceeds(&[1], 500));
    }
}
