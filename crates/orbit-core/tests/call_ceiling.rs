//! How long a call with the assistant may last: one rule whatever started
//! it, the recordings' own, said ten minutes before it hangs up.
#![allow(clippy::unwrap_used)]
use orbit_core::notice::NoticeCode;
use orbit_core::refusal::Refusal;
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

/// Retired duration requests from both clients cannot impose a ceiling.
#[test]
fn both_origins_ignore_retired_duration_and_never_warn_or_expire() {
    for origin in [CallOrigin::App, CallOrigin::Hotkey] {
        for duration in [0, MINUTE, 4 * 60 * MINUTE, 8 * 60 * MINUTE] {
            let state = call_from(origin, duration).unwrap();
            assert_eq!(
                state.durable.voice.call.as_ref().unwrap().expires_at_ms,
                i64::MAX
            );
            let d = decide(&state, Event::Tick, 5 * 60 * MINUTE).unwrap();
            assert!(d.state.durable.voice.call.is_some());
            assert!(d.effects.is_empty());
            assert_eq!(said(&d.state, NoticeCode::CallCeilingNear), 0);
        }
    }
}
