//! The composer's voice note in the pure core: Esc cancels a note without
//! sending it, a note shorter than the daemon's speech minimum is not sent
//! and says its length and that minimum, and a note whose transcription
//! failed can be transcribed again.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_core::{
    Connection, Effect, Event, PermissionState, State, VoiceTranscriptionFailure, decide,
    notice::{DataValue, NoticeCode, NoticeKey},
    protocol::Command,
    refusal::{Operation, Refusal, Resource},
    voice::{MINIMUM_SPEECH_MS, short_voice_note},
};
use serde_json::json;

/// Online, with the helper and the microphone: a voice note can start.
fn ready() -> State {
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let helper = decide(&online, Event::HelperReady, 0).unwrap().state;
    decide(
        &helper,
        Event::PermissionObserved {
            microphone: PermissionState::Granted,
            screen: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        0,
    )
    .unwrap()
    .state
}

/// A stored voice note `id` whose transcription failed with `code`.
fn failed_note(id: &str, code: &str) -> State {
    let stored = decide(
        &ready(),
        Event::VoiceStored {
            id: id.into(),
            duration_ms: 2_400,
        },
        10,
    )
    .unwrap()
    .state;
    decide(
        &stored,
        Event::VoiceFailed {
            id: id.into(),
            failure: VoiceTranscriptionFailure::local(code),
        },
        11,
    )
    .unwrap()
    .state
}

fn note_failure(state: &State) -> Option<String> {
    state
        .durable
        .assistant
        .turn
        .as_ref()
        .and_then(|turn| turn.user.voice.as_ref())
        .and_then(|voice| voice.failure.as_ref())
        .map(|failure| failure.code.clone())
}

#[test]
fn the_new_orders_have_their_wire_names_and_belong_to_the_assistant() {
    let cancel: Command = serde_json::from_value(json!({"command": "voice_cancel"})).unwrap();
    assert!(matches!(cancel, Command::VoiceCancel));
    let retry: Command =
        serde_json::from_value(json!({"command": "voice_retry", "message_id": "voice-1"})).unwrap();
    assert!(matches!(&retry, Command::VoiceRetry { message_id } if message_id == "voice-1"));
    for command in [cancel, retry] {
        assert_eq!(
            command.feature(),
            Some(orbit_core::features::FeatureId::Assistant)
        );
    }
}

#[test]
fn esc_ends_the_note_without_finishing_it_for_sending() {
    let recording = decide(&ready(), Event::VoiceStart, 1).unwrap().state;
    assert!(recording.durable.voice.voice_active);
    let cancelled = decide(&recording, Event::VoiceCancel, 2).unwrap();
    assert!(!cancelled.state.durable.voice.voice_active);
    assert_eq!(cancelled.effects, vec![Effect::CancelVoice]);
    // With no note recording there is nothing to cancel.
    let idle = decide(&ready(), Event::VoiceCancel, 3).unwrap();
    assert!(idle.effects.is_empty());
}

#[test]
fn the_minimum_is_the_call_detectors_speech_minimum() {
    assert_eq!(MINIMUM_SPEECH_MS, 160);
    assert_eq!(
        orbit_core::voice::VoicePolicy::default().minimum_speech_ms,
        MINIMUM_SPEECH_MS
    );
    let short = short_voice_note(90).expect("90 ms is under the minimum");
    assert_eq!((short.duration_ms, short.minimum_ms), (90, 160));
    assert_eq!(
        short_voice_note(159).map(|short| short.duration_ms),
        Some(159)
    );
    assert!(short_voice_note(160).is_none());
}

#[test]
fn a_short_note_is_kept_for_the_composer_with_its_numbers_until_the_next_note() {
    let short = short_voice_note(90).unwrap();
    let refused = decide(&ready(), Event::VoiceTooShort(short.clone()), 5)
        .unwrap()
        .state;
    assert_eq!(refused.views.short_voice_note, Some(short));
    let notice = refused
        .durable
        .notice
        .as_ref()
        .expect("the notice is logged");
    assert_eq!(notice.code, NoticeCode::VoiceNoteTooShort);
    assert_eq!(
        notice.data.get(&NoticeKey::DurationMs),
        Some(&DataValue::from(90_u64))
    );
    assert_eq!(
        notice.data.get(&NoticeKey::MinimumMs),
        Some(&DataValue::from(160_u64))
    );
    let next = decide(&refused, Event::VoiceStart, 6).unwrap().state;
    assert_eq!(next.views.short_voice_note, None);
    let typed = decide(
        &refused,
        Event::Submit {
            attachments: Vec::new(),
            id: "typed".into(),
            text: "Hola".into(),
        },
        7,
    )
    .unwrap()
    .state;
    assert_eq!(typed.views.short_voice_note, None);
}

#[test]
fn retrying_a_failed_note_clears_its_failure_and_says_it_transcribes() {
    let state = failed_note("voice-1", "transcription_upstream_failed");
    let retried = decide(
        &state,
        Event::VoiceRetry {
            id: "voice-1".into(),
        },
        20,
    )
    .unwrap()
    .state;
    assert_eq!(note_failure(&retried), None);
    assert_eq!(
        retried.durable.notice.as_ref().map(|notice| notice.code),
        Some(NoticeCode::VoiceTranscribing)
    );
}

#[test]
fn only_a_failed_note_of_the_open_turn_can_be_retried() {
    let state = failed_note("voice-1", "transcription_upstream_failed");
    let other = decide(&state, Event::VoiceRetry { id: "other".into() }, 20).unwrap_err();
    assert_eq!(
        other,
        Refusal::NotFound {
            resource: Resource::Message
        }
    );
    let pending = decide(
        &ready(),
        Event::VoiceStored {
            id: "voice-2".into(),
            duration_ms: 1_000,
        },
        10,
    )
    .unwrap()
    .state;
    // Still transcribing: there is no failure to retry.
    assert_eq!(
        decide(
            &pending,
            Event::VoiceRetry {
                id: "voice-2".into()
            },
            20
        )
        .unwrap_err(),
        Refusal::NotFound {
            resource: Resource::Message
        }
    );
}

#[test]
fn a_retry_waits_for_a_note_in_progress_and_for_the_hub() {
    let state = failed_note("voice-1", "transcription_upstream_failed");
    let recording = decide(&state, Event::VoiceStart, 21).unwrap().state;
    assert_eq!(
        decide(
            &recording,
            Event::VoiceRetry {
                id: "voice-1".into()
            },
            22
        )
        .unwrap_err(),
        Refusal::Busy {
            operation: Operation::Interaction,
            retry_after_ms: None
        }
    );
    let offline = decide(&state, Event::Connect(Connection::Offline), 23)
        .unwrap()
        .state;
    assert!(matches!(
        decide(
            &offline,
            Event::VoiceRetry {
                id: "voice-1".into()
            },
            24
        )
        .unwrap_err(),
        Refusal::HubUnreachable { .. }
    ));
}
