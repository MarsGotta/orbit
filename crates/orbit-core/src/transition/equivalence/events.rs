//! The corpus's events: every kind, with ids that mostly name what the
//! state holds (its turn, its recording, its voice notes) and sometimes
//! something else, and texts from empty to over the limit.
use super::Rng;
use super::*;

const VOICE_IDS: [&str; 4] = ["voice-1", "voice-2", "voice-3", "voice-4"];
const TURN_IDS: [&str; 3] = ["turn-1", "turn-2", "turn-3"];
const RECORDINGS: [&str; 2] = ["rec-1", "rec-2"];
const AGENTS: [Option<&str>; 3] = [None, Some("agente-norte"), Some("agente-sur")];
const SESSIONS: [&str; 2] = ["ses-1", "ses-2"];
const CAUSES: [&str; 4] = ["helper_lost", "sleep", "una frase libre", "shutdown"];

pub(super) fn event(rng: &mut Rng, state: &State) -> Event {
    match rng.below(100) {
        0..=14 => Event::Tick,
        15..=37 => capture(rng, state),
        38..=57 => assistant(rng, state),
        58..=71 => voice_notes(rng, state),
        72..=83 => voice_and_call(rng, state),
        84..=93 => link_and_helper(rng),
        _ => stops(rng),
    }
}

fn recording_id(rng: &mut Rng, state: &State) -> String {
    match state.durable.capture.recording.as_ref() {
        Some(recording) if rng.chance(85) => recording.id.clone(),
        _ => rng.pick(&RECORDINGS).into(),
    }
}

/// Mostly what comes next for the recording in its phase, so meetings
/// are recorded, paused, finished and thrown away; otherwise anything.
fn capture(rng: &mut Rng, state: &State) -> Event {
    let phase = state
        .durable
        .capture
        .recording
        .as_ref()
        .map(|recording| recording.phase.clone());
    if rng.chance(70) {
        let discard = Event::DiscardMeeting {
            recording_id: recording_id(rng, state),
        };
        let closed = Event::CutRecordingClosed {
            ended_at_ms: 1_790_000_000_000 + rng.below(20_000_000) as i64,
            cause: rng.pick(&CAUSES).into(),
        };
        let next = match phase {
            Some(CapturePhase::Starting) => Some(Event::CaptureStarted),
            Some(CapturePhase::Recording) => Some(rng.pick(&[
                Event::ChunkSealed,
                Event::ChunkSealed,
                Event::PauseMeeting,
                Event::FinishMeeting,
                discard.clone(),
            ])),
            Some(CapturePhase::Paused) => Some(rng.pick(&[
                Event::ResumeMeeting,
                Event::ResumeMeeting,
                Event::FinishMeeting,
                discard.clone(),
            ])),
            Some(CapturePhase::Finalizing) => Some(Event::CaptureFinalized),
            Some(CapturePhase::Interrupted) => Some(rng.pick(&[
                Event::ResumeMeeting,
                Event::EmptyCaptureClosed,
                Event::FinishMeeting,
                Event::EmptyCaptureClosed,
                closed,
            ])),
            Some(CapturePhase::Idle) | None => None,
        };
        if let Some(next) = next {
            return next;
        }
    }
    match rng.below(14) {
        0 | 1 => Event::StartMeeting {
            id: rng.pick(&RECORDINGS).into(),
            title: "Revisión del trimestre".into(),
            audio: match rng.below(4) {
                0 => MeetingAudio::MicrophoneOnly,
                1 => MeetingAudio::Application {
                    application_pid: rng.pick(&[0, 4242]),
                },
                _ => MeetingAudio::System,
            },
        },
        2 | 3 => Event::CaptureStarted,
        4 => Event::PauseMeeting,
        5 => Event::ResumeMeeting,
        6 => Event::FinishMeeting,
        7 => Event::DiscardMeeting {
            recording_id: recording_id(rng, state),
        },
        8 => Event::CaptureFinalized,
        9 => Event::EmptyCaptureClosed,
        10 => Event::CutRecordingClosed {
            ended_at_ms: 1_790_000_000_000 + rng.below(20_000_000) as i64,
            cause: rng.pick(&CAUSES).into(),
        },
        11 | 12 => Event::ChunkSealed,
        _ => Event::CaptureGap,
    }
}

fn request_id(rng: &mut Rng, state: &State) -> String {
    match state.durable.assistant.active_request.as_ref() {
        Some(active) if rng.chance(80) => active.clone(),
        _ => rng.pick(&TURN_IDS).into(),
    }
}

fn text(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => String::new(),
        1 => "   ".into(),
        2 => "é".repeat(MAX_TEXT_BYTES / 2 + 1),
        3 => "ñandú ".repeat(4_000),
        _ => " ¿Qué tengo mañana? ".into(),
    }
}

fn assistant(rng: &mut Rng, state: &State) -> Event {
    match rng.below(12) {
        0 | 1 => Event::Submit {
            id: rng.pick(&TURN_IDS).into(),
            text: text(rng),
            attachments: if rng.chance(30) {
                vec![Attachment {
                    id: "att-1".into(),
                    name: "agenda.pdf".into(),
                    kind: AttachmentKind::Pdf,
                    mime: "application/pdf".into(),
                    bytes: 2_048,
                    pages: Some(2),
                }]
            } else {
                Vec::new()
            },
        },
        2 | 3 => Event::Delta {
            request_id: request_id(rng, state),
            text: text(rng),
        },
        4 => Event::Completed {
            request_id: request_id(rng, state),
            text: rng.chance(60).then(|| text(rng)),
        },
        5 => Event::Approval {
            turn_id: request_id(rng, state),
            approval: Approval {
                request_id: "req-1".into(),
                description: "Leer el calendario".into(),
                clarification: rng.chance(30),
                choices: vec!["once".into(), "deny".into()],
                questions: Vec::new(),
                deletes: None,
            },
        },
        6 => Event::Interrupt,
        7 => Event::Failure {
            code: rng
                .pick(&["session_not_found", "hub_busy", "una frase"])
                .into(),
            numbers: vec![
                (NoticeKey::Max, "12".into()),
                (NoticeKey::Minutes, "3".into()),
            ],
        },
        8 => Event::Session {
            id: rng.pick(&SESSIONS).into(),
            agent_id: rng.pick(&AGENTS).map(Into::into),
        },
        9 => Event::NewSession {
            agent_id: rng.pick(&AGENTS).map(Into::into),
        },
        10 => Event::OpenSession {
            id: rng.pick(&SESSIONS).into(),
            agent_id: rng.pick(&AGENTS).map(Into::into),
        },
        _ => Event::DeviceChanged,
    }
}

fn voice_id(rng: &mut Rng, state: &State) -> String {
    let assistant = &state.durable.assistant;
    let mut known: Vec<String> = assistant
        .pending_voice
        .iter()
        .map(|note| note.message.id.clone())
        .collect();
    known.extend(assistant.turn.iter().map(|turn| turn.user.id.clone()));
    if !known.is_empty() && rng.chance(75) {
        rng.pick(&known)
    } else {
        rng.pick(&VOICE_IDS).into()
    }
}

fn voice_notes(rng: &mut Rng, state: &State) -> Event {
    match rng.below(10) {
        0 | 1 => Event::VoiceStored {
            id: rng.pick(&VOICE_IDS).into(),
            duration_ms: 1_000 + rng.below(60_000),
        },
        2 => Event::SubmitVoice {
            id: voice_id(rng, state),
            text: text(rng),
            duration_ms: rng.below(60_000),
        },
        3 => Event::VoiceFailed {
            id: voice_id(rng, state),
            failure: VoiceTranscriptionFailure {
                provider_status: rng.chance(50).then_some(503),
                ..VoiceTranscriptionFailure::local(rng.pick(&[
                    VOICE_HUB_UNREACHABLE,
                    VOICE_WAITING_TURN,
                    "voice_transcription_provider",
                ]))
            },
        },
        4 | 8 | 9 => Event::VoiceRetry {
            id: voice_id(rng, state),
        },
        5 => Event::VoiceDiscard {
            id: voice_id(rng, state),
        },
        6 => Event::VoiceUnheard {
            id: voice_id(rng, state),
        },
        _ => Event::VoiceTooShort(ShortVoiceNote {
            duration_ms: rng.below(400),
            minimum_ms: 400,
        }),
    }
}

/// A call is started often enough to be live while turns come and go.
fn voice_and_call(rng: &mut Rng, state: &State) -> Event {
    if state.durable.voice.call.is_none() && rng.chance(40) {
        return Event::CallStart {
            id: "call-1".into(),
            duration_ms: rng.pick(&[60_000, 600_000, 3_600_000]),
            origin: CallOrigin::App,
        };
    }
    match rng.below(11) {
        0 | 1 => Event::VoiceStart,
        2 => Event::VoiceEnd,
        3 => Event::VoiceCancel,
        4 | 9 | 10 => Event::CallStart {
            id: "call-1".into(),
            duration_ms: rng.pick(&[30_000, 60_000, 600_000, 3_600_000, 4_000_000]),
            origin: rng.pick(&[CallOrigin::App, CallOrigin::Hotkey]),
        },
        5 => Event::CallStop,
        6 => Event::CallReady,
        7 => Event::CallPhase(rng.pick(&[
            CallPhase::Starting,
            CallPhase::Listening,
            CallPhase::UserSpeaking,
            CallPhase::Waiting,
            CallPhase::Responding,
        ])),
        _ => Event::CallTurnStarted,
    }
}

fn permission(rng: &mut Rng) -> PermissionState {
    rng.pick(&[
        PermissionState::Granted,
        PermissionState::Granted,
        PermissionState::Granted,
        PermissionState::Granted,
        PermissionState::Granted,
        PermissionState::Granted,
        PermissionState::Denied,
        PermissionState::NotGranted,
        PermissionState::NotDetermined,
        PermissionState::Unknown,
    ])
}

fn link_and_helper(rng: &mut Rng) -> Event {
    match rng.below(12) {
        0..=2 => Event::Connect(rng.pick(&[
            Connection::Online,
            Connection::Online,
            Connection::Online,
            Connection::Online,
            Connection::Online,
            Connection::Online,
            Connection::Offline,
            Connection::Offline,
            Connection::Connecting,
            Connection::Revoked,
        ])),
        3 => Event::Connecting,
        4 => Event::HubTransportReady,
        5 => Event::HubDisconnected,
        6 | 10 => Event::HelperReady,
        11 => Event::LocalRearm,
        7 => Event::PermissionObserved {
            microphone: permission(rng),
            screen: permission(rng),
            accessibility: permission(rng),
            calendar: CalendarAuthorization::FullAccess,
        },
        8 => Event::CalendarAuthorization(rng.pick(&[
            CalendarAuthorization::Denied,
            CalendarAuthorization::FullAccess,
        ])),
        _ => rng.pick(&[Event::HelperLost, Event::HelperStopUndelivered]),
    }
}

fn stops(rng: &mut Rng) -> Event {
    match rng.below(9) {
        0 => Event::PrivacyStop,
        1 => Event::HelperLatched,
        2 | 3 => Event::LocalRearm,
        4 => Event::Degraded(rng.pick(&[
            DegradedKind::AudioGap,
            DegradedKind::PlaybackFailed,
            DegradedKind::Backpressure,
        ])),
        5 => Event::CaptureRefused {
            reason: rng
                .pick(&[
                    "microphone_permission_required",
                    "screen_permission_required",
                    "x",
                ])
                .into(),
        },
        6 | 7 => Event::CaptureCut {
            cause: rng.pick(&CAUSES).into(),
        },
        _ => Event::Close {
            cause: rng.pick(&CAUSES).into(),
        },
    }
}
