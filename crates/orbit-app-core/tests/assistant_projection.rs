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
            attachments: Vec::new(),
            id: "turn".into(),
            role: "user".into(),
            text: "Pregunta".into(),
            complete: true,
            at_ms: 1,
            voice: None,
        },
        answer: Some(Message {
            attachments: Vec::new(),
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
        (Activity::RetryingOpenai, AssistantActivity::RetryingOpenai),
        (
            Activity::RetryingAnthropic,
            AssistantActivity::RetryingAnthropic,
        ),
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
            attachments: Vec::new(),
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
            origin: orbit_core::CallOrigin::App,
            ceiling_notice_given: false,
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

/// Voice input is the daemon's microphone, not read-aloud: the read-aloud
/// voice unknown (nothing read yet) or absent (the hub's voice did not
/// arrive) neither hides that the microphone listens nor makes it unknown.
#[test]
fn voice_input_does_not_depend_on_read_aloud() {
    let read_aloud = [
        VoiceAvailability::Unknown {
            why: orbit_core::topics::Pending::NotObservedYet,
        },
        VoiceAvailability::Absent {
            code: VoiceAbsence::SpeechUnavailable,
        },
        VoiceAvailability::Present,
    ];
    for voice in read_aloud {
        for listening in [true, false] {
            let mut value = assistant_topic::assistant_value();
            value.voice = voice.clone();
            value.voice_active = listening;
            let chat = assistant_topic::snapshot(value, PermissionState::Unknown).chat;
            let projected = serde_json::to_value(&chat.voice).expect("the voice serializes");
            assert_eq!(
                projected["state"],
                if listening { "listening" } else { "idle" },
                "{voice:?}"
            );
            assert_eq!(projected["availability"]["state"], "current", "{voice:?}");
        }
    }
}

/// A call started with ⌃⌥D is in the app's snapshot as one from the app is:
/// it keeps the unbounded sentinel, origin, agent and earliest idle hangup
/// from the daemon and core constants.
#[test]
fn a_hotkey_call_has_no_duration_deadline_and_keeps_origin_and_agent() {
    let started_at_ms = 1_700_000_000_000;
    let expires_at_ms = i64::MAX;
    let mut value = assistant_topic::assistant_value();
    value.agent_id = Some("atlas".into());
    value.call = Knowledge::Present {
        value: Some(CallState {
            id: "call-hotkey".into(),
            phase: CallPhase::Listening,
            started_at_ms,
            expires_at_ms,
            turns: 0,
            origin: orbit_core::CallOrigin::Hotkey,
            ceiling_notice_given: false,
        }),
        as_of_ms: None,
    };
    let call = assistant_topic::snapshot(value, PermissionState::Granted)
        .call
        .expect("the hotkey call must be projected");
    assert_eq!(call.expires_at_ms, i64::MAX);
    assert_eq!(call.origin, orbit_app_core::CallOrigin::Hotkey);
    assert_eq!(call.agent_id.as_deref(), Some("atlas"));
    assert_eq!(
        call.idle_hangup_ms,
        orbit_core::silence::CALL_SILENCE_MS + orbit_core::silence::CALL_ANSWER_MS
    );
    let wire = serde_json::to_value(&call).expect("the call serializes");
    assert_eq!(wire["origin"], "hotkey");
    assert_eq!(wire["expires_at_ms"], expires_at_ms);
    assert_eq!(wire["idle_hangup_ms"], 45_000);

    // From the app, with the default agent: no agent named, «app».
    let mut value = assistant_topic::assistant_value();
    value.call = Knowledge::Present {
        value: Some(CallState {
            id: "call-app".into(),
            phase: CallPhase::Listening,
            started_at_ms,
            expires_at_ms: started_at_ms + 3_600_000,
            turns: 0,
            origin: orbit_core::CallOrigin::App,
            ceiling_notice_given: false,
        }),
        as_of_ms: None,
    };
    let call = assistant_topic::snapshot(value, PermissionState::Granted)
        .call
        .expect("the app's call must be projected");
    let wire = serde_json::to_value(&call).expect("the call serializes");
    assert_eq!(wire["origin"], "app");
    assert!(wire.get("agent_id").is_none(), "{wire}");
}
