#![allow(clippy::unwrap_used)]
use super::*;
use crate::FINALIZING_TIMEOUT_MS;
use crate::notice::NoticeCode;

/// The microphone guard, as a voice note asks for it: nothing else is
/// busy in these states, so the only refusal is the microphone's.
fn ensure_microphone(state: &State) -> Result<(), Refusal> {
    decide(state, Event::VoiceStart, 0).map(|_| ())
}

#[test]
fn microphone_decision_uses_observed_permission() {
    for (permission, allowed) in [
        (PermissionState::Unknown, false),
        (PermissionState::Denied, false),
        (PermissionState::NotDetermined, false),
        (PermissionState::Granted, true),
    ] {
        let state = State {
            durable: crate::DurableState {
                capabilities: Capabilities {
                    native_helper: true,
                    microphone_state: permission,
                    ..Capabilities::default()
                },
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        assert_eq!(ensure_microphone(&state).is_ok(), allowed, "{permission:?}");
    }
}

#[test]
fn screen_decision_uses_observed_permission() {
    for (permission, allowed) in [
        (PermissionState::Unknown, false),
        (PermissionState::Denied, false),
        (PermissionState::NotDetermined, false),
        (PermissionState::Granted, true),
    ] {
        let state = State {
            durable: crate::DurableState {
                capabilities: Capabilities {
                    native_helper: true,
                    microphone_state: PermissionState::Granted,
                    screen_state: permission,
                    ..Capabilities::default()
                },
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let result = decide(
            &state,
            Event::StartMeeting {
                id: "screen-permission".into(),
                title: "Fixture sintética".into(),
                audio: MeetingAudio::System,
            },
            0,
        );
        assert_eq!(result.is_ok(), allowed, "{permission:?}");
    }
}

#[test]
fn common_refusal_preserves_unknown_and_denied() {
    for (observed, expected) in [
        (
            PermissionState::Unknown,
            crate::refusal::Refusal::PermissionUnknown {
                permission: crate::refusal::Permission::Microphone,
            },
        ),
        (
            PermissionState::Denied,
            crate::refusal::Refusal::PermissionDenied {
                permission: crate::refusal::Permission::Microphone,
            },
        ),
        (
            PermissionState::NotGranted,
            crate::refusal::Refusal::PermissionNotGranted {
                permission: crate::refusal::Permission::Microphone,
            },
        ),
    ] {
        let state = State {
            durable: crate::DurableState {
                capabilities: Capabilities {
                    native_helper: true,
                    microphone_state: observed,
                    ..Capabilities::default()
                },
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let Err(refusal) = ensure_microphone(&state) else {
            panic!("permission must refuse");
        };
        let actual = refusal;
        assert_eq!(actual, expected, "{observed:?}");
    }
}

#[test]
fn final_public_text_replaces_provisional_deltas_and_late_completion_is_ignored() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(
        &state,
        Event::Delta {
            request_id: "a".into(),
            text: "provisional".into(),
        },
        1,
    )
    .unwrap()
    .state;
    let state = decide(
        &state,
        Event::Completed {
            request_id: "a".into(),
            text: Some("respuesta pública".into()),
        },
        2,
    )
    .unwrap()
    .state;
    let answer = state
        .durable
        .assistant
        .turn
        .as_ref()
        .unwrap()
        .answer
        .as_ref()
        .unwrap();
    assert_eq!(answer.text, "respuesta pública");
    assert!(answer.complete);
    assert!(state.durable.assistant.active_request.is_none());
    let late = decide(
        &state,
        Event::Completed {
            request_id: "a".into(),
            text: Some("tarde".into()),
        },
        3,
    )
    .unwrap()
    .state;
    assert_eq!(state.durable.assistant.turn, late.durable.assistant.turn);
}

#[test]
fn stop_survives_reconnect_and_restart_and_does_not_restart_capture() {
    let state = decide(&State::default(), Event::PrivacyStop, 0)
        .unwrap()
        .state;
    let state = decide(&state, Event::Connect(Connection::Online), 1)
        .unwrap()
        .state
        .recovered();
    assert!(state.durable.privacy.stopped);
    assert_eq!(
        decide(
            &state,
            Event::Submit {
                attachments: Vec::new(),
                id: "a".into(),
                text: "hola".into()
            },
            2
        )
        .unwrap_err(),
        Refusal::PrivacyStopped { since_ms: None }
    );
    let rearmed = decide(&state, Event::LocalRearm, 3).unwrap();
    assert!(rearmed.effects.is_empty());
    assert!(!rearmed.state.durable.voice.voice_active);
}

#[test]
fn old_delta_after_interrupt_is_ignored() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::Interrupt, 1).unwrap().state;
    let late = decide(
        &state,
        Event::Delta {
            request_id: "a".into(),
            text: "tarde".into(),
        },
        2,
    )
    .unwrap();
    assert_eq!(
        state.durable.assistant.turn,
        late.state.durable.assistant.turn
    );
}

#[test]
fn a_recording_can_pause_and_resume_after_five_hours_without_a_ceiling() {
    let state = State {
        durable: crate::DurableState {
            capabilities: Capabilities {
                native_helper: true,
                microphone_state: PermissionState::Granted,
                ..Capabilities::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::StartMeeting {
            id: "r".into(),
            title: "Prueba".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::CaptureStarted, 0).unwrap().state;
    let late = decide(&state, Event::Tick, 5 * 3_600_000).unwrap();
    assert!(!late.effects.contains(&Effect::StopCapture));
    let paused = decide(&late.state, Event::PauseMeeting, 5 * 3_600_000 + 1).unwrap();
    assert!(decide(&paused.state, Event::ResumeMeeting, 5 * 3_600_000 + 2).is_ok());
}

#[test]
fn late_approval_after_interrupt_is_ignored_and_voice_start_is_not_busy() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            capabilities: Capabilities {
                native_helper: true,
                microphone_state: PermissionState::Granted,
                ..Capabilities::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::Interrupt, 1).unwrap().state;
    let after = decide(
        &state,
        Event::Approval {
            turn_id: "a".into(),
            approval: Approval {
                request_id: "a".into(),
                description: "¿Publico el calendario?".into(),
                clarification: false,
                choices: Vec::new(),
                questions: Vec::new(),
                deletes: None,
            },
        },
        2,
    )
    .unwrap();
    assert!(
        after.state.durable.assistant.pending_approval.is_none(),
        "a late approval must not become pending: {:?}",
        after.state.durable.assistant.pending_approval
    );
    assert!(after.effects.is_empty());
    let voice = decide(&after.state, Event::VoiceStart, 3).unwrap();
    assert!(voice.state.durable.voice.voice_active);
}

/// Adapted from `approval_matching_active_request_is_stored`, whose
/// assumption was the defect: it gave the request the turn's id, which the
/// hub never does. The hub names a request by the backend's own id; what
/// makes it current is the turn its frame arrived under.
#[test]
fn an_approval_for_the_active_turn_is_stored_under_the_agents_own_id() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let approval = Approval {
        request_id: "permiso-del-agente".into(),
        description: "¿Publico el calendario?".into(),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
        deletes: None,
    };
    let after = decide(
        &state,
        Event::Approval {
            turn_id: "a".into(),
            approval: approval.clone(),
        },
        1,
    )
    .unwrap();
    assert_eq!(
        after.state.durable.assistant.pending_approval,
        Some(approval.clone())
    );

    let other = decide(
        &state,
        Event::Approval {
            turn_id: "b".into(),
            approval,
        },
        1,
    )
    .unwrap();
    assert!(
        other.state.durable.assistant.pending_approval.is_none(),
        "another turn's question is not this turn's"
    );
    assert!(
        other
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .any(|record| record.code == NoticeCode::AssistantStaleQuestion),
        "the notice says the question belongs to another turn"
    );
}

#[test]
fn approval_after_completed_turn_is_ignored() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(
        &state,
        Event::Completed {
            request_id: "a".into(),
            text: Some("respuesta".into()),
        },
        1,
    )
    .unwrap()
    .state;
    let after = decide(
        &state,
        Event::Approval {
            turn_id: "a".into(),
            approval: Approval {
                request_id: "a".into(),
                description: "¿Publico el calendario?".into(),
                clarification: false,
                choices: Vec::new(),
                questions: Vec::new(),
                deletes: None,
            },
        },
        2,
    )
    .unwrap();
    assert!(
        after.state.durable.assistant.pending_approval.is_none(),
        "the turn already completed; its approval must be ignored: {:?}",
        after.state.durable.assistant.pending_approval
    );
    assert!(after.effects.is_empty());
    assert_eq!(
        state.durable.assistant.turn,
        after.state.durable.assistant.turn
    );
}

#[test]
fn late_clarification_after_interrupt_is_ignored() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::Interrupt, 1).unwrap().state;
    let after = decide(
        &state,
        Event::Approval {
            turn_id: "a".into(),
            approval: Approval {
                request_id: "a".into(),
                description: "¿Qué reunión querías decir?".into(),
                clarification: true,
                choices: Vec::new(),
                questions: Vec::new(),
                deletes: None,
            },
        },
        2,
    )
    .unwrap();
    assert!(
        after.state.durable.assistant.pending_approval.is_none(),
        "a late clarification must be ignored like an approval: {:?}",
        after.state.durable.assistant.pending_approval
    );
    assert!(after.effects.is_empty());
    assert_eq!(
        state.durable.assistant.turn,
        after.state.durable.assistant.turn
    );
}

#[test]
fn ignored_approval_records_notice_without_description() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    let state = decide(&state, Event::Interrupt, 1).unwrap().state;
    let before = state.durable.notice_log.records().len();
    let after = decide(
        &state,
        Event::Approval {
            turn_id: "a".into(),
            approval: Approval {
                request_id: "a".into(),
                description: "SECRETO-DESCRIPCION".into(),
                clarification: false,
                choices: Vec::new(),
                questions: Vec::new(),
                deletes: None,
            },
        },
        2,
    )
    .unwrap();
    let codes: Vec<NoticeCode> = after
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .map(|record| record.code)
        .collect();
    assert!(
        codes.len() > before,
        "an ignored approval must still leave a notice: {codes:?}"
    );
    assert!(
        codes.contains(&NoticeCode::AssistantStaleQuestion),
        "the notice must say the question belongs to a closed turn: {codes:?}"
    );
    let said = serde_json::to_string(after.state.durable.notice_log.records()).unwrap();
    assert!(
        !said.contains("SECRETO-DESCRIPCION"),
        "the notice must never quote the approval description: {said}"
    );
}

fn recording_in_phase(phase: CapturePhase) -> Recording {
    Recording {
        id: "r".into(),
        title: "Sesión".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: 1_000_000,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase,
        sealed_chunks: 2,
        gaps: 1,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    }
}

fn state_with_recording(recording: Recording) -> State {
    State {
        durable: crate::DurableState {
            capabilities: Capabilities {
                native_helper: true,
                microphone_state: PermissionState::Granted,
                ..Capabilities::default()
            },
            capture: CaptureState {
                recording: Some(recording),
                ..CaptureState::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    }
}

#[test]
fn finish_meeting_while_privacy_stopped_closes_the_recording_locally() {
    for phase in [CapturePhase::Interrupted, CapturePhase::Paused] {
        let state = crate::authority_fixture::stopped_state(state_with_recording(
            recording_in_phase(phase.clone()),
        ));
        let decision = decide(&state, Event::FinishMeeting, 5_000).unwrap();
        let recording = decision.state.durable.capture.recording.as_ref().unwrap();
        assert_ne!(recording.phase, CapturePhase::Finalizing, "{phase:?}");
        assert_eq!(recording.phase, CapturePhase::Idle, "{phase:?}");
        assert_eq!(recording.ended_at_ms, Some(5_000), "{phase:?}");
        assert_eq!(recording.sealed_chunks, 2, "{phase:?}");
        assert_eq!(recording.gaps, 1, "{phase:?}");
        assert!(
            decision
                .effects
                .iter()
                .all(|effect| !matches!(effect, Effect::StopCapture)),
            "without Finalizing there is no producer to drain: {:#?}",
            decision.effects
        );
    }
}

/// Old persisted deadlines never stop a recording or emit retired notices.
#[test]
fn a_recording_ignores_its_retired_expiry() {
    let mut recording = recording_in_phase(CapturePhase::Recording);
    recording.started_at_ms = 0;
    recording.expires_at_ms = 240 * 60_000;
    let decision = decide(&state_with_recording(recording), Event::Tick, 5 * 3_600_000).unwrap();
    assert!(decision.effects.is_empty());
    assert_eq!(
        decision.state.durable.capture.recording.unwrap().phase,
        CapturePhase::Recording
    );
}

/// No start source gives a recording a duration deadline.
#[test]
fn a_meeting_starts_without_a_duration_deadline() {
    let mut state = state_with_recording(recording_in_phase(CapturePhase::Idle));
    state.durable.capture.recording = None;
    state.durable.capabilities.screen_state = PermissionState::Granted;
    let decision = decide(
        &state,
        Event::StartMeeting {
            id: "planning".into(),
            title: "Planificación".into(),
            audio: MeetingAudio::System,
        },
        5_000,
    )
    .unwrap();
    let recording = decision.state.durable.capture.recording.unwrap();
    assert_eq!(recording.expires_at_ms, i64::MAX);
}

/// Retired warnings never appear even when a saved deadline is near.
#[test]
fn retired_ceiling_notices_are_never_emitted() {
    let mut recording = recording_in_phase(CapturePhase::Recording);
    recording.expires_at_ms = 240 * 60_000;
    let mut state = state_with_recording(recording);
    for minute in [229, 230, 231, 240, 300] {
        let decision = decide(&state, Event::Tick, minute * 60_000).unwrap();
        assert!(decision.effects.is_empty());
        assert_eq!(decision.state.durable.notice, state.durable.notice);
        state = decision.state;
    }
}

#[test]
fn tick_past_the_finalizing_deadline_interrupts_with_gap_and_notice() {
    let since = 10_000;
    let state = state_with_recording(Recording {
        finalizing_since_ms: Some(since),
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
        ..recording_in_phase(CapturePhase::Finalizing)
    });
    let decision = decide(&state, Event::Tick, since + FINALIZING_TIMEOUT_MS + 1).unwrap();
    let recording = decision.state.durable.capture.recording.as_ref().unwrap();
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert_eq!(recording.gaps, 2);
    assert_ne!(decision.state.durable.notice, state.durable.notice);
    assert_eq!(
        decision.state.notice_code(),
        Some(NoticeCode::RecordingCloseUnconfirmed)
    );
}

#[test]
fn tick_before_the_finalizing_deadline_keeps_the_phase() {
    let since = 10_000;
    let state = state_with_recording(Recording {
        finalizing_since_ms: Some(since),
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
        ..recording_in_phase(CapturePhase::Finalizing)
    });
    let decision = decide(&state, Event::Tick, since + FINALIZING_TIMEOUT_MS).unwrap();
    let recording = decision.state.durable.capture.recording.as_ref().unwrap();
    assert_eq!(recording.phase, CapturePhase::Finalizing);
    assert_eq!(recording.gaps, 1);
    assert_eq!(recording.ended_at_ms, None);
}

#[test]
fn finish_or_resume_on_a_closed_recording_is_refused_as_capture_closed() {
    let state = state_with_recording(Recording {
        ended_at_ms: Some(2_000),
        ..recording_in_phase(CapturePhase::Idle)
    });
    assert_eq!(
        decide(&state, Event::FinishMeeting, 3_000).unwrap_err(),
        Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::OpenRecording
        }
    );
    assert_eq!(
        decide(&state, Event::ResumeMeeting, 3_000).unwrap_err(),
        Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::OpenRecording
        }
    );
}

#[test]
fn after_the_local_closure_a_new_meeting_is_accepted() {
    let interrupted = crate::authority_fixture::stopped_state(state_with_recording(
        recording_in_phase(CapturePhase::Interrupted),
    ));
    let closed = decide(&interrupted, Event::FinishMeeting, 5_000)
        .unwrap()
        .state;
    let rearmed = decide(&closed, Event::LocalRearm, 6_000).unwrap().state;
    assert!(!rearmed.durable.privacy.stopped);
    let decision = decide(
        &rearmed,
        Event::StartMeeting {
            id: "next".into(),
            title: "Otra sesión".into(),
            audio: MeetingAudio::MicrophoneOnly,
        },
        7_000,
    )
    .unwrap();
    assert_eq!(
        decision
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .id,
        "next",
        "the new meeting must replace the closed one"
    );
    assert!(
        decision
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartCapture { .. }))
    );
}

#[test]
fn initial_delta_of_200_kib_is_clamped_to_max_text_bytes_of_valid_utf8() {
    let state = State {
        durable: crate::DurableState {
            link: HubLinkState {
                connection: Connection::Online,
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let state = decide(
        &state,
        Event::Submit {
            attachments: Vec::new(),
            id: "a".into(),
            text: "hola".into(),
        },
        0,
    )
    .unwrap()
    .state;
    // 200 KiB of two-byte characters: cutting by characters doubles the
    // byte budget and a cut blind to boundaries splits a character.
    let payload = "é".repeat(100 * 1024);
    assert_eq!(payload.len(), 200 * 1024);
    let after = decide(
        &state,
        Event::Delta {
            request_id: "a".into(),
            text: payload.clone(),
        },
        1,
    )
    .unwrap()
    .state;
    let message = after
        .durable
        .assistant
        .turn
        .as_ref()
        .unwrap()
        .answer
        .as_ref()
        .unwrap();
    assert_eq!(
        message.text.len(),
        MAX_TEXT_BYTES,
        "the clamp is measured in bytes, not characters"
    );
    assert!(
        message.text.is_char_boundary(MAX_TEXT_BYTES),
        "the clamp must fall on a character boundary"
    );
    assert!(payload.starts_with(message.text.as_str()));
}

fn audio_start(audio: crate::protocol::MeetingAudio) -> Event {
    Event::StartMeeting {
        id: "audio-scope-recording".into(),
        title: "Reunión de sistema".into(),
        audio,
    }
}

#[test]
fn system_and_application_scopes_need_screen_audio_but_not_microphone_only() {
    let capable = |screen_granted: bool| State {
        durable: crate::DurableState {
            capabilities: Capabilities {
                native_helper: true,
                microphone_state: PermissionState::Granted,
                screen_state: if screen_granted {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                },
                ..Capabilities::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    // The system scope is authorized by the same screen
    // recording permission an application scope already needed.
    assert_eq!(
        decide(
            &capable(false),
            audio_start(crate::protocol::MeetingAudio::System),
            0
        )
        .unwrap_err(),
        Refusal::PermissionDenied {
            permission: crate::refusal::Permission::ScreenRecording,
        }
    );
    assert_eq!(
        decide(
            &capable(false),
            audio_start(crate::protocol::MeetingAudio::Application {
                application_pid: 4242
            }),
            0
        )
        .unwrap_err(),
        Refusal::PermissionDenied {
            permission: crate::refusal::Permission::ScreenRecording,
        }
    );
    let admitted = decide(
        &capable(true),
        audio_start(crate::protocol::MeetingAudio::System),
        0,
    )
    .unwrap();
    assert!(admitted.state.durable.capture.recording.is_some());
    assert!(
        admitted
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartCapture { .. }))
    );
    // Microphone-only records nothing remote, so no screen permission.
    decide(
        &capable(false),
        audio_start(crate::protocol::MeetingAudio::MicrophoneOnly),
        0,
    )
    .unwrap();
    // An application scope without a positive PID is not an application.
    assert!(
        decide(
            &capable(true),
            audio_start(crate::protocol::MeetingAudio::Application { application_pid: 0 }),
            0
        )
        .is_err(),
        "a null PID identifies no application"
    );
}
