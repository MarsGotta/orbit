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

/// A muted microphone sends silence: no turn opens however loud a frame
/// claims to be, and the muted time is not the user's silence that hangs
/// up; that silence counts again from the unmute. The call's ceiling still
/// counts while muted.
#[test]
fn a_muted_microphone_opens_no_turn_and_is_not_user_silence() {
    let policy = VoicePolicy::default();
    let idle = policy.idle_timeout_ms as u64;
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
        TurnAction::Stop(StopReason::Idle),
        "the silence counts again from the unmute"
    );
    let expiring = TurnDetector::authorized(5, 0, 1_000);
    assert_eq!(
        expiring.observe(policy, muted(1_000, false)).1,
        TurnAction::Stop(StopReason::Expired),
        "the call's ceiling counts while muted"
    );
}
