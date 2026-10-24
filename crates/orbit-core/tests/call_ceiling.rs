//! How long a call with the assistant may last: one rule whatever started
//! it, the recordings' own, said ten minutes before it hangs up.
#![allow(clippy::unwrap_used)]
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::{Expected, Refusal};
use orbit_core::{CallOrigin, Connection, Event, PermissionState, State, decide};

const MINUTE: i64 = 60_000;

fn ready_for_a_call() -> State {
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let ready = decide(&online, Event::HelperReady, 0).unwrap().state;
    let mut state = decide(
        &ready,
        Event::PermissionObserved {
            microphone: PermissionState::Granted,
            screen: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        0,
    )
    .unwrap()
    .state;
    state.set_voice_processing_supported(true);
    state
}

fn call_from(origin: CallOrigin, duration_ms: i64) -> Result<State, Refusal> {
    decide(
        &ready_for_a_call(),
        Event::CallStart {
            id: "llamada-de-prueba".into(),
            duration_ms,
            origin,
        },
        0,
    )
    .map(|decision| decision.state)
}

fn said(state: &State, code: NoticeCode) -> usize {
    state
        .durable
        .notice_log
        .records()
        .iter()
        .filter(|notice| notice.code == code)
        .count()
}

/// A call lasts what a recording lasts, from the app or with ⌃⌥D: the
/// same constant, never a copy, and nothing longer is admitted.
#[test]
fn both_origins_get_the_recordings_four_hours() {
    assert_eq!(
        orbit_core::voice::CALL_MINUTES,
        orbit_core::RECORDING_CEILING_MINUTES
    );
    let ceiling_ms = i64::from(orbit_core::voice::CALL_MINUTES) * MINUTE;
    assert_eq!(ceiling_ms, 4 * 60 * MINUTE);
    for origin in [CallOrigin::App, CallOrigin::Hotkey] {
        let state = call_from(origin, ceiling_ms).unwrap();
        let call = state.durable.voice.call.as_ref().unwrap();
        assert_eq!(call.expires_at_ms, ceiling_ms, "{origin:?}");
        let refusal = call_from(origin, ceiling_ms + 1).unwrap_err();
        assert!(
            matches!(
                refusal,
                Refusal::InvalidParameter {
                    expected: Expected::DurationRange { max_ms, .. },
                    ..
                } if max_ms == ceiling_ms
            ),
            "{origin:?}: {refusal:?}"
        );
    }
}

/// At 3 h 50 the person is told once how long is left, how long the call
/// has lasted and its ceiling, as a recording says it; at 4 h it ends.
#[test]
fn both_origins_are_warned_at_three_hours_fifty_and_end_at_four() {
    let ceiling_ms = i64::from(orbit_core::voice::CALL_MINUTES) * MINUTE;
    for origin in [CallOrigin::App, CallOrigin::Hotkey] {
        let state = call_from(origin, ceiling_ms).unwrap();
        let early = decide(&state, Event::Tick, 229 * MINUTE).unwrap().state;
        assert_eq!(said(&early, NoticeCode::CallCeilingNear), 0, "{origin:?}");
        let warned = decide(&early, Event::Tick, 230 * MINUTE).unwrap().state;
        let notice = warned.durable.notice.clone().unwrap();
        assert_eq!(
            Notice {
                sequence: None,
                op: None,
                ..notice
            },
            Notice::new(NoticeCode::CallCeilingNear)
                .with(NoticeKey::Remaining, 10_i64)
                .with(NoticeKey::Minutes, 230_i64)
                .with(NoticeKey::Max, 240_i64),
            "{origin:?}"
        );
        assert!(warned.durable.voice.call.is_some(), "the call goes on");
        let again = decide(&warned, Event::Tick, 235 * MINUTE).unwrap().state;
        assert_eq!(said(&again, NoticeCode::CallCeilingNear), 1, "said once");
        let ended = decide(&again, Event::Tick, ceiling_ms).unwrap().state;
        assert!(ended.durable.voice.call.is_none(), "{origin:?}");
        assert_eq!(
            ended.durable.notice.as_ref().map(|notice| notice.code),
            Some(NoticeCode::CallExpired)
        );
    }
}
