#![allow(clippy::expect_used)]

#[path = "support/assistant_topic.rs"]
mod assistant_topic;

use orbit_core::{
    Approval, CallPhase, CallState, Message, PermissionState, Turn, VoiceNote, topics::Knowledge,
};

fn question(id: &str) -> Message {
    Message {
        id: id.into(),
        role: "user".into(),
        text: "Pregunta".into(),
        complete: true,
        at_ms: 1_700_000_000_500,
        voice: None,
    }
}
use serde_json::{Value, json};

fn projected_json(value: orbit_core::topics::assistant::Value, screen: PermissionState) -> Value {
    serde_json::to_value(assistant_topic::snapshot(value, screen)).expect("typed app snapshot")
}

#[test]
fn projects_pending_call_and_voice_note_from_topic_frames() {
    let mut approval = assistant_topic::assistant_value();
    approval.pending = Some(Approval {
        request_id: "approval-1".into(),
        description: "Actualizar la agenda".into(),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
    });
    assert_eq!(
        projected_json(approval, PermissionState::Unknown)["chat"]["pending"],
        json!({ "kind": "approval", "id": "approval-1", "text": "Actualizar la agenda", "choices": [] })
    );

    let mut call = assistant_topic::assistant_value();
    call.call = Knowledge::Present {
        value: Some(CallState {
            id: "call-1".into(),
            phase: CallPhase::Responding,
            started_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_700_003_600_000,
            turns: 2,
        }),
        as_of_ms: None,
    };
    call.turn = Some(Box::new(Turn {
        user: question("turn-1"),
        answer: Some(Message {
            id: "turn-1:assistant".into(),
            role: "assistant".into(),
            text: "Estoy revisando el error de la pantalla.".into(),
            complete: false,
            at_ms: 1_700_000_001_000,
            voice: None,
        }),
    }));
    assert_eq!(
        projected_json(call, PermissionState::Granted)["call"],
        json!({
            "started_at_ms": 1_700_000_000_000_i64,
            "seeing_screen": true,
            "last_line": "Estoy revisando el error de la pantalla.",
            "speaking": true,
        })
    );

    let mut voice = assistant_topic::assistant_value();
    voice.turn = Some(Box::new(Turn {
        user: Message {
            id: "voice-1".into(),
            role: "user".into(),
            text: "Si Dario no contesta, lo llevo yo a la daily.".into(),
            complete: true,
            at_ms: 1_700_000_014_000,
            voice: Some(VoiceNote {
                duration_ms: 14_000,
                ..VoiceNote::default()
            }),
        },
        answer: None,
    }));
    let snapshot = projected_json(voice, PermissionState::Unknown);
    let message = &snapshot["chat"]["turn"][0];
    assert_eq!(message["at_ms"], json!(1_700_000_014_000_i64));
    assert_eq!(message["voice"], json!({ "duration_ms": 14_000 }));
    assert_eq!(message["receipts"], json!([]));
    assert!(message.get("seeing_screen").is_none());
}

#[test]
fn projects_voice_failure_without_inventing_a_transcript() {
    let message: Message = serde_json::from_value(json!({
        "id":"voice-failed", "role":"user", "text":"", "complete":true,
        "voice":{"duration_ms":2000,"audio_available":true,
            "failure":{"code":"transcription_http_error","provider_status":503,
                "hub_status":502,"timeout_ms":null,
                "bytes":9007199254740993u64,"max_bytes":1048576}}
    }))
    .expect("synthetic voice note");
    let mut value = assistant_topic::assistant_value();
    value.turn = Some(Box::new(Turn {
        user: message,
        answer: None,
    }));
    let snapshot = projected_json(value, PermissionState::Unknown);
    let voice = &snapshot["chat"]["turn"][0]["voice"];
    assert_eq!(voice["failure"]["code"], "transcription_http_error");
    assert_eq!(voice["failure"]["provider_status"], 503);
    assert_eq!(voice["failure"]["bytes"], "9007199254740993");
    assert_eq!(voice["failure"]["max_bytes"], "1048576");
    assert!(voice["transcript"].is_null());
}

#[test]
fn projects_the_open_session_and_no_list_of_messages() {
    let mut value = assistant_topic::assistant_value();
    value.session_id = Some("session-active".into());
    let chat = projected_json(value, PermissionState::Unknown)["chat"].clone();
    assert_eq!(chat["session_id"], "session-active");
    assert_eq!(chat["turn"], json!([]));
    assert!(chat.get("messages").is_none());
    assert!(chat.get("sessions").is_none());
}
