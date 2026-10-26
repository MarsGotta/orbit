//! Silence closes only after an unanswered, visible question.
#![allow(clippy::unwrap_used)]
use orbit_core::audio_signal::{RecordingSignalView, SignalCondition, TrackSignalView};
use orbit_core::notice::NoticeCode;
use orbit_core::protocol::MeetingAudio;
use orbit_core::silence::CallQuiet;
use orbit_core::{CallOrigin, Connection, Effect, Event, PermissionState, State, decide};

fn ready() -> State {
    let mut s = State::default();
    for e in [
        Event::Connect(Connection::Online),
        Event::HelperReady,
        Event::PermissionObserved {
            microphone: PermissionState::Granted,
            screen: PermissionState::Granted,
            accessibility: PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
    ] {
        s = decide(&s, e, 0).unwrap().state;
    }
    s.set_voice_processing_supported(true);
    s
}
fn call() -> State {
    let s = decide(
        &ready(),
        Event::CallStart {
            id: "call".into(),
            duration_ms: 60_000,
            origin: CallOrigin::App,
        },
        0,
    )
    .unwrap()
    .state;
    decide(&s, Event::CallReady, 0).unwrap().state
}
fn check_call(s: &State, now: i64, silent_ms: i64, busy: bool) -> orbit_core::Decision {
    decide(
        s,
        Event::CheckSilence {
            call: Some(CallQuiet { silent_ms, busy }),
            recording: None,
            microphone_muted: false,
        },
        now,
    )
    .unwrap()
}
fn meeting() -> State {
    let s = decide(
        &ready(),
        Event::StartMeeting {
            id: "meeting".into(),
            title: "Planning".into(),
            audio: MeetingAudio::System,
        },
        0,
    )
    .unwrap()
    .state;
    decide(&s, Event::CaptureStarted, 0).unwrap().state
}
fn signal(low: u64) -> TrackSignalView {
    TrackSignalView {
        condition: SignalCondition::LowSignal,
        received_frames: 100,
        since_last_samples_ms: Some(0),
        low_signal_ms: low,
        speech_quiet_ms: Some(low),
    }
}
fn check_meeting(s: &State, now: i64, mic: u64, remote: u64, muted: bool) -> orbit_core::Decision {
    decide(
        s,
        Event::CheckSilence {
            call: None,
            recording: Some(Box::new(RecordingSignalView {
                recording_id: "meeting".into(),
                capture_epoch_id: "epoch".into(),
                mic_local: signal(mic),
                remote_app: Some(signal(remote)),
            })),
            microphone_muted: muted,
        },
        now,
    )
    .unwrap()
}
#[test]
fn call_asks_at_thirty_then_hangs_up_exactly_like_stop() {
    let s = call();
    assert!(
        !check_call(&s, 29_999, 29_999, false)
            .effects
            .contains(&Effect::AskCallPresence)
    );
    let asked = check_call(&s, 30_000, 30_000, false);
    assert!(asked.effects.contains(&Effect::AskCallPresence));
    assert_eq!(
        asked.state.durable.notice.unwrap().code,
        NoticeCode::CallSilenceQuestion
    );
    let asked = check_call(&s, 30_000, 30_000, false).state;
    assert!(
        check_call(&asked, 44_999, 44_999, false)
            .state
            .durable
            .voice
            .call
            .is_some()
    );
    let timed = check_call(&asked, 45_000, 45_000, false);
    let manual = decide(&asked, Event::CallStop, 45_000).unwrap();
    assert_eq!(timed.effects, manual.effects);
    assert!(timed.state.durable.voice.call.is_none());
}
#[test]
fn words_and_continue_reset_but_noise_does_not() {
    let asked = check_call(&call(), 30_000, 30_000, false).state;
    let kept = decide(&asked, Event::CallKeep, 31_000).unwrap().state;
    assert!(
        !check_call(&kept, 60_999, 60_999, false)
            .effects
            .contains(&Effect::AskCallPresence)
    );
    assert!(
        check_call(&kept, 61_000, 61_000, false)
            .effects
            .contains(&Effect::AskCallPresence)
    );
    assert!(
        check_call(&asked, 45_000, 45_000, false)
            .state
            .durable
            .voice
            .call
            .is_none()
    );
    let words = check_call(&asked, 31_000, 0, false).state;
    assert!(
        check_call(&words, 45_000, 14_000, false)
            .state
            .durable
            .voice
            .call
            .is_some()
    );
}
#[test]
fn assistant_work_never_asks_and_active_calls_have_no_duration_ceiling() {
    for now in [30_000, 60_000, 5 * 3_600_000] {
        let d = check_call(&call(), now, now, true);
        assert!(!d.effects.contains(&Effect::AskCallPresence));
        assert!(
            decide(&d.state, Event::Tick, now)
                .unwrap()
                .state
                .durable
                .voice
                .call
                .is_some()
        );
    }
}
#[test]
fn recording_asks_only_after_two_minutes_on_both_sides() {
    let s = meeting();
    for (mic, remote) in [
        (119_999, 120_000),
        (120_000, 119_999),
        (0, 120_000),
        (120_000, 0),
    ] {
        assert_ne!(
            check_meeting(&s, 120_000, mic, remote, false)
                .state
                .durable
                .notice
                .unwrap()
                .code,
            NoticeCode::RecordingSilenceQuestion
        );
    }
    assert_eq!(
        check_meeting(&s, 120_000, 120_000, 120_000, false)
            .state
            .durable
            .notice
            .unwrap()
            .code,
        NoticeCode::RecordingSilenceQuestion
    );
    assert_eq!(
        check_meeting(&s, 120_000, 0, 120_000, true)
            .state
            .durable
            .notice
            .unwrap()
            .code,
        NoticeCode::RecordingSilenceQuestion
    );
}
#[test]
fn recording_timeout_uses_the_full_normal_drain_and_save() {
    let s = decide(&meeting(), Event::ChunkSealed, 1).unwrap().state;
    let asked = check_meeting(&s, 120_000, 120_000, 120_000, false).state;
    let d = check_meeting(&asked, 150_000, 150_000, 150_000, false);
    assert_eq!(
        d.effects,
        decide(&asked, Event::FinishMeeting, 150_000)
            .unwrap()
            .effects
    );
    let drained = decide(&d.state, Event::ChunkSealed, 150_001).unwrap().state;
    let saved = decide(&drained, Event::CaptureFinalized, 150_002)
        .unwrap()
        .state;
    let r = saved.durable.capture.recording.unwrap();
    assert_eq!(r.sealed_chunks, 2);
    assert_eq!(r.started_at_ms, 0);
    assert_eq!(r.ended_at_ms, Some(150_002));
}
#[test]
fn recording_continue_voice_or_unknown_prevents_close_and_time_alone_never_ends() {
    let asked = check_meeting(&meeting(), 120_000, 120_000, 120_000, false).state;
    let kept = decide(&asked, Event::RecordingKeep, 121_000).unwrap().state;
    assert_eq!(
        check_meeting(&kept, 150_000, 150_000, 150_000, false)
            .state
            .durable
            .capture
            .recording
            .unwrap()
            .phase,
        orbit_core::CapturePhase::Recording
    );
    let voice = check_meeting(&asked, 149_000, 0, 149_000, false).state;
    assert_eq!(
        check_meeting(&voice, 150_000, 1_000, 150_000, false)
            .state
            .durable
            .capture
            .recording
            .unwrap()
            .phase,
        orbit_core::CapturePhase::Recording
    );
    let unknown = decide(
        &asked,
        Event::CheckSilence {
            call: None,
            recording: None,
            microphone_muted: false,
        },
        150_000,
    )
    .unwrap();
    assert!(unknown.effects.is_empty());
    assert_eq!(
        decide(&unknown.state, Event::Tick, 5 * 3_600_000)
            .unwrap()
            .state
            .durable
            .capture
            .recording
            .unwrap()
            .phase,
        orbit_core::CapturePhase::Recording
    );
}

#[test]
fn stale_unknown_or_missing_sides_withdraw_the_question_and_never_stop() {
    let asked = check_meeting(&meeting(), 120_000, 120_000, 120_000, false).state;
    for condition in [
        SignalCondition::Unknown,
        SignalCondition::DeliveryDelayed,
        SignalCondition::AwaitingSamples,
    ] {
        let mut mic = signal(150_000);
        mic.condition = condition;
        let event = Event::CheckSilence {
            call: None,
            recording: Some(Box::new(RecordingSignalView {
                recording_id: "meeting".into(),
                capture_epoch_id: "epoch".into(),
                mic_local: mic,
                remote_app: Some(signal(150_000)),
            })),
            microphone_muted: false,
        };
        let d = decide(&asked, event, 150_000).unwrap();
        assert!(d.effects.is_empty());
        assert!(!d.state.views.silence_questions.recording.pending());
        assert_eq!(
            d.state.durable.capture.recording.unwrap().phase,
            orbit_core::CapturePhase::Recording
        );
    }
    let d = check_meeting(&meeting(), 5 * 3_600_000, 0, 5 * 3_600_000, false);
    assert!(d.effects.is_empty());
    assert_eq!(
        d.state.durable.capture.recording.unwrap().phase,
        orbit_core::CapturePhase::Recording
    );
}

#[test]
fn recovery_retires_saved_questions_without_resuming_their_countdowns() {
    for asked in [
        check_call(&call(), 30_000, 30_000, false).state,
        check_meeting(&meeting(), 120_000, 120_000, 120_000, false).state,
    ] {
        let recovered = asked.recovered();
        assert!(!recovered.views.silence_questions.call.pending());
        assert!(!recovered.views.silence_questions.recording.pending());
        assert!(
            recovered
                .durable
                .notice_log
                .records()
                .iter()
                .any(|n| matches!(
                    n.code,
                    NoticeCode::CallSilenceResolved | NoticeCode::RecordingSilenceResolved
                ))
        );
    }
}

fn measured_sides(mic: i16, remote: i16, seconds: u64) -> RecordingSignalView {
    use orbit_core::audio_signal::{TrackSignalMonitor, measure_s16le};
    let mut microphone = TrackSignalMonitor::new(0);
    let mut meeting = TrackSignalMonitor::new(0);
    for (monitor, amplitude) in [(&mut microphone, mic), (&mut meeting, remote)] {
        let pcm: Vec<_> = std::iter::repeat_n(amplitude.to_le_bytes(), 8_000)
            .flatten()
            .collect();
        let level = measure_s16le(&pcm).unwrap();
        for second in 1..=seconds {
            assert!(monitor.observe(level, 8_000, second * 1_000));
        }
    }
    RecordingSignalView {
        recording_id: "meeting".into(),
        capture_epoch_id: "room".into(),
        mic_local: microphone.view(seconds * 1_000),
        remote_app: Some(meeting.view(seconds * 1_000)),
    }
}

fn check_levels(state: &State, signal: RecordingSignalView, now: i64) -> orbit_core::Decision {
    decide(
        state,
        Event::CheckSilence {
            call: None,
            recording: Some(Box::new(signal)),
            microphone_muted: false,
        },
        now,
    )
    .unwrap()
}

#[test]
fn room_noise_below_the_calls_speech_threshold_asks_after_two_minutes() {
    for amplitude in [33, 250, 499] {
        let early = check_levels(&meeting(), measured_sides(amplitude, 0, 119), 119_000);
        assert!(!early.state.views.silence_questions.recording.pending());
        let levels = measured_sides(amplitude, 0, 120);
        assert_eq!(
            levels.mic_local.condition,
            SignalCondition::Receiving,
            "room noise is not a dead microphone"
        );
        assert_eq!(levels.mic_local.low_signal_ms, 0);
        let asked = check_levels(&early.state, levels, 120_000);
        assert_eq!(
            asked.state.notice_code(),
            Some(NoticeCode::RecordingSilenceQuestion),
            "RMS {amplitude}"
        );
    }
}

#[test]
fn speech_at_or_above_the_call_threshold_on_either_side_prevents_the_question() {
    for amplitude in [500, 900] {
        for (mic, remote) in [(amplitude, 0), (0, amplitude)] {
            let decision = check_levels(&meeting(), measured_sides(mic, remote, 150), 150_000);
            assert!(!decision.state.views.silence_questions.recording.pending());
            assert!(decision.effects.is_empty());
        }
    }
}

#[test]
fn legacy_signal_without_speech_observation_cannot_close_a_recording() {
    let asked = check_meeting(&meeting(), 120_000, 120_000, 120_000, false).state;
    let mut legacy = serde_json::to_value(signal(150_000)).unwrap();
    legacy.as_object_mut().unwrap().remove("speech_quiet_ms");
    let track = serde_json::from_value(legacy).unwrap();
    let observation = RecordingSignalView {
        recording_id: "meeting".into(),
        capture_epoch_id: "epoch".into(),
        mic_local: track,
        remote_app: Some(signal(150_000)),
    };
    let decision = check_levels(&asked, observation, 150_000);
    assert!(decision.effects.is_empty());
    assert!(!decision.state.views.silence_questions.recording.pending());
    assert_eq!(
        decision.state.durable.capture.recording.unwrap().phase,
        orbit_core::CapturePhase::Recording
    );
}
