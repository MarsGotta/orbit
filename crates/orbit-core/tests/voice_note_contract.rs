#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_core::{
    Event, Message, State, VoiceNote, VoiceTranscriptionFailure, decide,
    protocol::{Command, VoiceNoteAudio},
};
use serde_json::json;

#[test]
fn a1_voice_note_audio_command_and_payload_have_a_bounded_local_wire_shape() {
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
fn ref308_failure_marks_only_the_pending_note_and_cannot_replace_a_transcript() {
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
fn a2_voice_metadata_is_additive_and_defaults_for_an_older_message() {
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
fn ref308_restart_marks_a_pending_voice_note_as_interrupted_instead_of_in_progress() {
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
