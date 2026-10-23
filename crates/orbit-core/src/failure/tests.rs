use crate::failure::{DegradedKind, Outcome, classify_gap, classify_native_response};
use crate::model::{Capabilities, CapturePhase, State};
use crate::transition::{Effect, Event, decide};

/// Same shape the transition tests use: a permitted meeting that reached
/// the recording phase, with no gaps yet.
fn live_recording() -> State {
    let base = State {
        durable: crate::DurableState {
            capabilities: Capabilities {
                native_helper: true,
                microphone_state: crate::PermissionState::Granted,
                ..Capabilities::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let started = decide(
        &base,
        Event::StartMeeting {
            id: "rec-taxonomy".into(),
            title: "Reunión".into(),
            audio: crate::protocol::MeetingAudio::MicrophoneOnly,
        },
        0,
    )
    .expect("a permitted meeting cannot be refused")
    .state;
    decide(&started, Event::CaptureStarted, 1)
        .expect("capture confirmation follows the start")
        .state
}

#[test]
fn audio_gap_degradation_is_declared_and_recording_continues() {
    let state = live_recording();
    // A gap is classified, not escalated: declaring it is the product
    // decision.
    assert!(matches!(
        classify_gap(&state),
        Outcome::Degraded {
            kind: DegradedKind::AudioGap,
            ..
        }
    ));
    let previous_notice = state.durable.notice.clone();
    let notices_before = state.durable.notice_log.records().len();
    let decision = decide(&state, Event::Degraded(DegradedKind::AudioGap), 2)
        .expect("a declared gap is never refused");
    let recording = decision
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .expect("the recording survives a declared gap");
    assert_eq!(recording.gaps, 1);
    assert_eq!(recording.phase, CapturePhase::Recording);
    assert!(!decision.state.durable.privacy.stopped);
    assert!(decision.effects.is_empty());
    assert!(
        decision.state.durable.notice_log.records().len() > notices_before,
        "a declared gap must leave an additive notice"
    );
    assert!(
        decision
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .any(|record| Some(record) == previous_notice.as_ref()),
        "the new notice must not overwrite what was already said"
    );
    // Only an audio gap counts as one; another degradation leaves the
    // counter and the phase alone.
    let other = decide(&state, Event::Degraded(DegradedKind::PlaybackFailed), 3)
        .expect("a degradation is never refused");
    let other_recording = other
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .expect("the recording survives any degradation");
    assert_eq!(other_recording.gaps, 0);
    assert_eq!(other_recording.phase, CapturePhase::Recording);
    assert!(other.effects.is_empty());
}

#[test]
fn the_user_stop_keeps_its_effects_and_an_integrity_failure_cuts_without_latching() {
    // The removed event_for adapter had no production caller. Exercise the
    // real decision boundary for the integrity cut instead.
    let state = live_recording();
    let Outcome::IntegrityFailure { detail } =
        classify_native_response("audio_stop", "audio_drain_incomplete", &state)
    else {
        panic!("a live drain failure must cut capture");
    };
    let cut = decide(&state, Event::CaptureCut { cause: detail }, 2)
        .expect("an integrity cut is always applied");
    assert!(!cut.state.durable.privacy.stopped);
    assert_eq!(
        cut.state
            .durable
            .capture
            .recording
            .as_ref()
            .expect("recording retained")
            .phase,
        CapturePhase::Interrupted
    );
    assert_eq!(cut.effects, [Effect::CutCapture, Effect::StopPlayback]);

    let state = live_recording();
    let decision = decide(&state, Event::PrivacyStop, 2).expect("a privacy stop is always applied");
    assert!(decision.state.durable.privacy.stopped);
    assert_eq!(
        decision.state.durable.privacy.generation,
        state.durable.privacy.generation + 1
    );
    assert_eq!(
        decision
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .expect("the recording record survives the stop")
            .phase,
        CapturePhase::Interrupted
    );
    // Exactly the current effects: nothing dropped, nothing added.
    assert_eq!(decision.effects.len(), 4);
    for effect in [
        Effect::StopCapture,
        Effect::StopPlayback,
        Effect::Interrupt,
        Effect::CancelNetwork,
    ] {
        assert!(decision.effects.contains(&effect), "missing {effect:?}");
    }
}

#[test]
fn native_response_classification_pins_the_audit_cases() {
    let idle = State::default();
    assert!(matches!(
        classify_native_response("playback", "playback_backpressure", &idle),
        Outcome::Degraded { .. }
    ));
    assert!(matches!(
        classify_native_response("audio_start", "microphone_permission_required", &idle),
        Outcome::Refused { .. }
    ));
    let live = live_recording();
    assert!(matches!(
        classify_native_response("audio_stop", "audio_drain_incomplete", &live),
        Outcome::IntegrityFailure { .. }
    ));
    assert!(matches!(
        classify_native_response("audio_start", "code_from_an_unreleased_helper", &idle),
        Outcome::IntegrityFailure { .. }
    ));
    assert!(matches!(
        classify_native_response("permissions", "code_from_an_unreleased_helper", &idle),
        Outcome::Degraded {
            kind: DegradedKind::HelperRejected,
            ..
        }
    ));
}

/// The helper's main thread held its backlog and the order never ran: a
/// refusal of that one order, never a cut of what is already recording.
#[test]
fn an_order_refused_behind_a_held_main_thread_is_refused_not_a_cut() {
    let live = live_recording();
    for command in [
        "audio_start",
        "status",
        "permissions",
        "screenshot",
        "rearm",
    ] {
        assert!(
            matches!(
                classify_native_response(command, "native_main_thread_busy", &live),
                Outcome::Refused { ref reason } if reason == "native_main_thread_busy"
            ),
            "{command}"
        );
    }
}
