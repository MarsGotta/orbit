//! The call turn detector, observed frame by frame.
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
        transcribing: false,
        muted: false,
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
fn turns_are_bounded_but_silence_and_old_deadlines_never_stop_the_detector() {
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
        TurnAction::None
    );
    assert_eq!(
        TurnDetector::authorized(5, 0, 100)
            .observe(policy, frame(5, true))
            .1,
        TurnAction::None
    );
}
/// A call of 2026-10-18 stopped `idle` 20 s into Deimos's spoken answer,
/// after ~40 s of tool work: a minute after the user's last turn.
/// Assistant work advances the last activity used by the presence question.
#[test]
fn deimos_working_or_speaking_is_not_user_silence() {
    let policy = VoicePolicy::default();
    let idle = crate::silence::CALL_SILENCE_MS as u64;
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
        TurnAction::None
    );
}
#[test]
fn pcm_energy_is_bounded_and_not_a_name_or_wake_word() {
    assert!(!energy_exceeds(&[0; 100], 500));
    assert!(energy_exceeds(&i16::MIN.to_le_bytes(), 500));
    assert!(!energy_exceeds(&[1], 500));
}

/// A muted microphone sends silence: no turn opens however loud a frame
/// claims to be, and the muted time is not the user's silence that hangs
/// up; silence counts again from the unmute, without a duration ceiling.
#[test]
fn a_muted_microphone_opens_no_turn_and_is_not_user_silence() {
    let policy = VoicePolicy::default();
    let idle = crate::silence::CALL_SILENCE_MS as u64;
    let muted = |ms: u64, voiced: bool| VoiceObservation {
        monotonic_ns: ms * 1_000_000,
        received_at_ms: ms as i64,
        muted: true,
        ..frame(0, voiced)
    };
    let mut state = TurnDetector::authorized(5, 0, 10 * idle as i64);
    for step in 0..50 {
        let action;
        (state, action) = state.observe(policy, muted(step * 20, true));
        assert_eq!(action, TurnAction::None, "no turn opens while muted");
    }
    let (state, action) = state.observe(policy, muted(3 * idle, false));
    assert_eq!(action, TurnAction::None, "muted time never hangs up");
    let unmuted = |ms: u64| VoiceObservation {
        monotonic_ns: ms * 1_000_000,
        received_at_ms: ms as i64,
        ..frame(0, false)
    };
    let (state, action) = state.observe(policy, unmuted(3 * idle + 20));
    assert_eq!(action, TurnAction::None);
    let (_, action) = state.observe(policy, unmuted(4 * idle));
    assert_eq!(
        action,
        TurnAction::None,
        "the detector reports silence; the question policy decides closure"
    );
    let expiring = TurnDetector::authorized(5, 0, 1_000);
    assert_eq!(
        expiring.observe(policy, muted(1_000, false)).1,
        TurnAction::None,
        "the retired ceiling cannot close a muted call"
    );
}

/// A 20 ms frame at `ms` of the call: voice or quiet, and whether a turn's
/// words are being asked for.
fn noise_at(ms: u64, voiced: bool, transcribing: bool) -> VoiceObservation {
    VoiceObservation {
        monotonic_ns: ms * 1_000_000,
        received_at_ms: ms as i64,
        transcribing,
        ..frame(0, voiced)
    }
}

/// What a stretch of a call did: when it stopped, if it did, and how many
/// turns opened.
struct Run {
    detector: TurnDetector,
    stopped_at_ms: Option<u64>,
    turns: u32,
}

/// Frames from `from_ms` to `to_ms`: `voiced(ms)` says which are loud, and
/// each turn is transcribed for `transcription_ms` after it ends, its words
/// heard when `words(ms)` says so.
fn run(
    mut detector: TurnDetector,
    (from_ms, to_ms): (u64, u64),
    voiced: impl Fn(u64) -> bool,
    transcription_ms: u64,
    words: impl Fn(u64) -> bool,
) -> Run {
    let policy = VoicePolicy::default();
    let mut transcribed_until = None;
    let mut turns = 0;
    let mut ms = from_ms;
    let mut question = crate::silence::Question::default();
    while ms < to_ms {
        if transcribed_until.is_some_and(|until| ms >= until) {
            transcribed_until = None;
            if words(ms) {
                detector.heard_words();
            }
        }
        let action;
        (detector, action) = detector.observe(
            policy,
            noise_at(ms, voiced(ms), transcribed_until.is_some()),
        );
        match action {
            TurnAction::Stop(StopReason::Idle) => {
                return Run {
                    detector,
                    stopped_at_ms: Some(ms),
                    turns,
                };
            }
            TurnAction::Begin { .. } => turns += 1,
            TurnAction::Finish { .. } => transcribed_until = Some(ms + transcription_ms),
            TurnAction::Stop(other) => panic!("stopped as {other:?} at {ms} ms"),
            TurnAction::None => {}
        }
        let silent = ms as i64 - detector.last_activity_ms();
        let quiet = silent >= crate::silence::CALL_SILENCE_MS;
        if question.observe("call", "", quiet, crate::silence::CALL_ANSWER_MS, ms as i64)
            == crate::silence::Outcome::Close
        {
            return Run {
                detector,
                stopped_at_ms: Some(ms),
                turns,
            };
        }
        ms += 20;
    }
    Run {
        detector,
        stopped_at_ms: None,
        turns,
    }
}

/// A forgotten call in a room with a fan that starts now and then, or a
/// television: the noise opens turns, the hub finds no words in them, and
/// the question closes it after 30 s plus the 15 s answer window.
/// Energy alone kept such a call open until its ceiling.
#[test]
fn a_noisy_call_asks_then_hangs_up_without_words() {
    let idle = crate::silence::CALL_SILENCE_MS as u64;
    let run = run(
        TurnDetector::authorized(5, 0, 4 * 3_600_000),
        (0, 4 * idle),
        // Every 5 s, 400 ms of noise.
        |ms| ms % 5_000 < 400,
        1_000,
        |_| false,
    );
    assert_eq!(run.turns, 9, "one burst every 5 s before the 45 s closure");
    let stopped = run
        .stopped_at_ms
        .expect("a call with only noise never hung up");
    assert!(
        (idle + 15_000..idle + 20_000).contains(&stopped),
        "hung up at {stopped} ms"
    );
}

/// Noise that never stops opens turn after turn, each one heard before the
/// next ends: a turn in course holds the hang-up, but only for as long as
/// one turn can last, so even that call hangs up.
#[test]
fn unbroken_noise_holds_the_hang_up_for_one_turn_at_most() {
    let policy = VoicePolicy::default();
    let idle = crate::silence::CALL_SILENCE_MS as u64;
    let run = run(
        TurnDetector::authorized(5, 0, 4 * 3_600_000),
        (0, 4 * idle),
        |_| true,
        1_000,
        |_| false,
    );
    let stopped = run
        .stopped_at_ms
        .expect("unbroken noise kept the call open");
    assert!(
        (idle + 15_000..=idle + 15_000 + policy.maximum_turn_ms).contains(&stopped),
        "hung up at {stopped} ms"
    );
}

/// The person speaks now and then, with the same noise around: their words
/// keep the call going; after they stop, the unanswered question closes it.
#[test]
fn words_keep_a_call_going_and_noise_does_not() {
    let idle = crate::silence::CALL_SILENCE_MS as u64;
    // Words in the first five quiet intervals, every half minute.
    let talked = 5 * idle;
    let speaking = run(
        TurnDetector::authorized(5, 0, 4 * 3_600_000),
        (0, talked),
        |ms| ms % 5_000 < 400,
        1_000,
        |ms| ms % 30_000 < 5_000,
    );
    assert_eq!(speaking.stopped_at_ms, None, "a call with words hung up");
    let last_words = speaking.detector.last_activity_ms() as u64;
    assert!(talked - last_words < 30_000, "words at {last_words} ms");
    let silent = run(
        speaking.detector,
        (talked, talked + 2 * idle),
        |ms| ms % 5_000 < 400,
        1_000,
        |_| false,
    );
    let stopped = silent.stopped_at_ms.expect("noise kept the call open");
    assert!(
        (last_words + idle + 15_000..last_words + idle + 20_000).contains(&stopped),
        "hung up at {stopped} ms, words last at {last_words} ms"
    );
}
