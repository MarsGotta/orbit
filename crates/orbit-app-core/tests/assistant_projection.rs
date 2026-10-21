#![allow(clippy::expect_used)]

#[path = "support/assistant_topic.rs"]
mod assistant_topic;

use orbit_app_core::AssistantActivity;
use orbit_core::{
    CallPhase, CallState, Message, PermissionState, Turn, VoiceNote,
    topics::{
        Knowledge,
        assistant::{Activity, VoiceAbsence, VoiceAvailability},
    },
};

/// A turn whose answer is `text`.
fn answered(text: &str, complete: bool) -> Turn {
    Turn {
        user: Message {
            id: "turn".into(),
            role: "user".into(),
            text: "Pregunta".into(),
            complete: true,
            at_ms: 1,
            voice: None,
        },
        answer: Some(Message {
            id: "turn:assistant".into(),
            role: "assistant".into(),
            text: text.into(),
            complete,
            at_ms: 2,
            voice: None,
        }),
    }
}

#[test]
fn projects_each_active_phase_and_clears_it_with_the_turn() {
    for (phase, expected) in [
        (Activity::Transcribing, AssistantActivity::Transcribing),
        (Activity::Thinking, AssistantActivity::Thinking),
        (Activity::Responding, AssistantActivity::Responding),
    ] {
        let mut value = assistant_topic::assistant_value();
        value.turn_in_flight = true;
        value.activity = Some(phase.clone());
        let snapshot = assistant_topic::snapshot(value, PermissionState::Unknown);
        assert_eq!(snapshot.chat.activity, Some(expected), "{phase:?}");
        assert!(snapshot.chat.turn.is_empty(), "{phase:?}");
    }

    let mut streaming = assistant_topic::assistant_value();
    streaming.turn_in_flight = true;
    streaming.activity = Some(Activity::Responding);
    streaming.turn = Some(Box::new(answered("Respuesta parcial", false)));
    assert_eq!(
        assistant_topic::snapshot(streaming.clone(), PermissionState::Unknown)
            .chat
            .activity,
        Some(AssistantActivity::Responding)
    );
    streaming.turn_in_flight = false;
    streaming.activity = None;
    assert_eq!(
        assistant_topic::snapshot(streaming, PermissionState::Unknown)
            .chat
            .activity,
        None
    );
}

#[test]
fn projects_voice_transcript_and_audio_availability() {
    let mut value = assistant_topic::assistant_value();
    value.turn = Some(Box::new(Turn {
        user: Message {
            id: "voice-app-12".into(),
            role: "user".into(),
            text: "Si Dario no contesta, lo llevo yo a la daily.".into(),
            complete: true,
            at_ms: 7,
            voice: Some(VoiceNote {
                duration_ms: 14_000,
                transcript: Some("Si Dario no contesta, lo llevo yo a la daily.".into()),
                failure: None,
                audio_available: true,
            }),
        },
        answer: None,
    }));
    let voice = assistant_topic::snapshot(value, PermissionState::Unknown)
        .chat
        .turn[0]
        .voice
        .clone()
        .expect("voice metadata must be projected");
    assert_eq!(
        voice.transcript.as_deref(),
        Some("Si Dario no contesta, lo llevo yo a la daily.")
    );
    assert!(voice.audio_available);

    let command = orbit_app_core::AppCommand::Assistant(
        orbit_app_core::features::assistant::AssistantCommand::VoiceNoteAudio {
            message_id: "voice-app-12".into(),
        },
    );
    assert_eq!(
        serde_json::to_value(command).expect("voice audio command must serialize"),
        serde_json::json!({ "command": "voice_note_audio", "message_id": "voice-app-12" })
    );
}

#[test]
fn projects_real_call_content_and_typed_voice_absence() {
    let mut value = assistant_topic::assistant_value();
    value.call = Knowledge::Present {
        value: Some(CallState {
            id: "call-app-12".into(),
            phase: CallPhase::Responding,
            started_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_700_003_600_000,
            turns: 1,
        }),
        as_of_ms: None,
    };
    value.voice = VoiceAvailability::Absent {
        code: VoiceAbsence::SpeechUnavailable,
    };
    value.turn = Some(Box::new(answered(
        "La plantilla no está en el bucket.",
        true,
    )));
    let call = assistant_topic::snapshot(value, PermissionState::Granted)
        .call
        .expect("the active call must be projected");
    assert_eq!(call.started_at_ms, 1_700_000_000_000);
    assert_eq!(
        call.last_line.as_deref(),
        Some("La plantilla no está en el bucket.")
    );
    assert!(call.seeing_screen);
    assert!(call.voice_unavailable);
}
