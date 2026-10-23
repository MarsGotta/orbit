#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_core::{
    Event, Message, State, VoiceNote, VoiceTranscriptionFailure, decide,
    protocol::{Command, VoiceNoteAudio},
};
use serde_json::json;

#[test]
fn voice_note_audio_command_and_payload_have_a_bounded_local_wire_shape() {
    let command: Command = serde_json::from_value(json!({
        "command": "voice_note_audio",
        "message_id": "30000000-0000-4000-8000-000000000003"
    }))
    .unwrap();
    assert!(
        matches!(command, Command::VoiceNoteAudio { message_id } if message_id == "30000000-0000-4000-8000-000000000003")
    );

    let payload = VoiceNoteAudio {
        message_id: "30000000-0000-4000-8000-000000000003".into(),
        format: "wav".into(),
        bytes_base64: "UklGRg==".into(),
    };
    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        json!({
            "message_id": "30000000-0000-4000-8000-000000000003",
            "format": "wav",
            "bytes_base64": "UklGRg=="
        })
    );
    assert_eq!(orbit_core::MAX_VOICE_NOTE_BYTES, 8 * 1024 * 1024);
}

#[test]
fn failure_marks_only_the_pending_note_and_cannot_replace_a_transcript() {
    let state = decide(
        &State::default(),
        Event::Connect(orbit_core::Connection::Online),
        0,
    )
    .unwrap()
    .state;
    let pending = decide(
        &state,
        Event::VoiceStored {
            id: "voice".into(),
            duration_ms: 1000,
        },
        1,
    )
    .expect("pending voice");
    let failure = VoiceTranscriptionFailure {
        code: "transcription_http_error".into(),
        provider_status: Some(503),
        hub_status: Some(502),
        timeout_ms: None,
        bytes: None,
        max_bytes: None,
        attempts: Some(1),
    };
    let failed = decide(
        &pending.state,
        Event::VoiceFailed {
            id: "voice".into(),
            failure: failure.clone(),
        },
        2,
    )
    .expect("voice failure");
    assert_eq!(
        failed
            .state
            .durable
            .turn
            .as_ref()
            .expect("turn")
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref()),
        Some(&failure)
    );
    assert!(failed.effects.is_empty());
    let unrelated = decide(
        &failed.state,
        Event::VoiceFailed {
            id: "another".into(),
            failure: failure.clone(),
        },
        3,
    )
    .expect("unrelated failure");
    assert_eq!(
        unrelated
            .state
            .durable
            .turn
            .as_ref()
            .expect("turn")
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref()),
        Some(&failure)
    );
    let mut complete = failed.state;
    complete
        .durable
        .turn
        .as_mut()
        .expect("turn")
        .user
        .voice
        .as_mut()
        .expect("voice")
        .transcript = Some("texto".into());
    let late = decide(
        &complete,
        Event::VoiceFailed {
            id: "voice".into(),
            failure: VoiceTranscriptionFailure {
                code: "transcription_timeout".into(),
                provider_status: None,
                hub_status: None,
                timeout_ms: Some(90_000),
                bytes: None,
                max_bytes: None,
                attempts: Some(1),
            },
        },
        4,
    )
    .expect("late failure");
    assert_eq!(
        late.state
            .durable
            .turn
            .as_ref()
            .expect("turn")
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref()),
        Some(&failure)
    );
}

#[test]
fn voice_metadata_is_additive_and_defaults_for_an_older_message() {
    let note = VoiceNote {
        duration_ms: 1_250,
        transcript: Some("nota sintética".into()),
        failure: None,
        audio_available: true,
    };
    assert_eq!(
        serde_json::to_value(&note).unwrap(),
        json!({
            "duration_ms": 1_250,
            "transcript": "nota sintética",
            "audio_available": true
        })
    );

    let old: Message = serde_json::from_value(json!({
        "id": "old-voice",
        "role": "user",
        "text": "texto anterior",
        "complete": true,
        "voice": { "duration_ms": 900 }
    }))
    .unwrap();
    let voice = old.voice.expect("voice metadata");
    assert_eq!(voice.transcript, None);
    assert!(!voice.audio_available);
}

#[test]
fn restart_marks_a_pending_voice_note_as_interrupted_instead_of_in_progress() {
    let pending = decide(
        &decide(
            &State::default(),
            Event::Connect(orbit_core::Connection::Online),
            0,
        )
        .unwrap()
        .state,
        Event::VoiceStored {
            id: "voice-restart".into(),
            duration_ms: 1_000,
        },
        1,
    )
    .expect("pending voice")
    .state;
    let recovered = State::from(pending.durable().recovered());
    assert_eq!(
        recovered
            .durable
            .turn
            .as_ref()
            .expect("turn")
            .user
            .voice
            .as_ref()
            .and_then(|voice| voice.failure.as_ref())
            .map(|failure| failure.code.as_str()),
        Some("voice_transcription_interrupted_restart")
    );
}

fn stored(state: &State, id: &str, at_ms: i64) -> State {
    decide(
        state,
        Event::VoiceStored {
            id: id.into(),
            duration_ms: 2_000,
        },
        at_ms,
    )
    .expect("voice note stored")
    .state
}

/// A note spoken with the hub away is a message all the same: the WAV is on
/// the Mac and its transcription waits.
#[test]
fn a_voice_note_is_stored_without_the_hub() {
    let offline = State::default();
    let state = stored(&offline, "nota-sin-hub", 1);
    assert_eq!(
        state
            .durable
            .turn
            .as_ref()
            .map(|turn| turn.user.id.as_str()),
        Some("nota-sin-hub")
    );
}

/// A later turn does not drop a note the hub never took: it waits in
/// `pending_voice`, survives a restart, and «Reintentar» makes it the turn.
#[test]
fn a_replaced_voice_note_waits_on_the_mac_and_can_be_retried() {
    let online = decide(
        &State::default(),
        Event::Connect(orbit_core::Connection::Online),
        0,
    )
    .unwrap()
    .state;
    let noted = stored(&online, "nota-fallida", 1);
    let failed = decide(
        &noted,
        Event::VoiceFailed {
            id: "nota-fallida".into(),
            failure: VoiceTranscriptionFailure::local("transcription_http_error"),
        },
        2,
    )
    .unwrap()
    .state;
    let chatted = decide(
        &failed,
        Event::Submit {
            id: "pregunta".into(),
            text: "Otra pregunta sintética".into(),
            attachments: Vec::new(),
        },
        3,
    )
    .unwrap()
    .state;
    assert_eq!(
        chatted
            .durable
            .pending_voice
            .iter()
            .map(|note| note.message.id.as_str())
            .collect::<Vec<_>>(),
        ["nota-fallida"]
    );
    let restarted: State = serde_json::from_value::<orbit_core::DurableState>(
        serde_json::to_value(&chatted.durable).unwrap(),
    )
    .unwrap()
    .recovered()
    .into();
    assert_eq!(
        restarted.durable.pending_voice.len(),
        1,
        "kept across a restart"
    );
    let mut answered = chatted.clone();
    answered.durable.active_request = None;
    let retried = decide(
        &answered,
        Event::VoiceRetry {
            id: "nota-fallida".into(),
        },
        4,
    )
    .unwrap()
    .state;
    assert!(retried.durable.pending_voice.is_empty());
    let turn = retried.durable.turn.as_ref().unwrap();
    assert_eq!(turn.user.id, "nota-fallida");
    assert_eq!(turn.user.voice.as_ref().unwrap().failure, None);
}

/// Past the bound a new note is refused with its numbers, never stored
/// without a name.
#[test]
fn a_full_list_of_waiting_voice_notes_refuses_the_next_with_its_count() {
    let mut state = decide(&State::default(), Event::HelperReady, 0)
        .unwrap()
        .state;
    state = decide(
        &state,
        Event::PermissionObserved {
            microphone: orbit_core::PermissionState::Granted,
            screen: orbit_core::PermissionState::Unknown,
            accessibility: orbit_core::PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        0,
    )
    .unwrap()
    .state;
    let waiting = Message {
        id: "nota".into(),
        role: "user".into(),
        text: String::new(),
        complete: true,
        at_ms: 1,
        voice: Some(VoiceNote {
            duration_ms: 1_000,
            transcript: None,
            failure: Some(VoiceTranscriptionFailure::local("voice_hub_unreachable")),
            audio_available: true,
        }),
        attachments: Vec::new(),
    };
    state.durable.pending_voice = (0..orbit_core::MAX_PENDING_VOICE_NOTES)
        .map(|index| orbit_core::PendingVoiceNote {
            message: Message {
                id: format!("nota-{index}"),
                ..waiting.clone()
            },
            conversation: None,
        })
        .collect();
    let refused = decide(&state, Event::VoiceStart, 2).unwrap_err();
    assert_eq!(
        refused,
        orbit_core::refusal::Refusal::TooMany {
            resource: orbit_core::refusal::Resource::Message,
            count: orbit_core::MAX_PENDING_VOICE_NOTES as u64,
            max: orbit_core::MAX_PENDING_VOICE_NOTES as u64,
        }
    );
}
