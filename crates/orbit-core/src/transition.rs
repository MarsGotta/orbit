use crate::FINALIZING_TIMEOUT_MS;
use crate::authority::AuthorityChange;
use crate::failure::DegradedKind;
use crate::model::*;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::privacy_stop::StopRequester;
use crate::protocol::MeetingAudio;
use crate::refusal::Refusal;

#[derive(Debug, Clone)]
pub enum Event {
    Connect(Connection),
    /// The local link is starting; no authority has changed yet.
    Connecting,
    Submit {
        id: String,
        text: String,
    },
    SubmitVoice {
        id: String,
        text: String,
        duration_ms: u64,
    },
    VoiceStored {
        id: String,
        duration_ms: u64,
    },
    VoiceFailed {
        id: String,
        failure: VoiceTranscriptionFailure,
    },
    Session(String),
    /// The transport may report ready after connect() already set Connecting.
    /// That still withdraws the previous context request.
    HubTransportReady,
    HubDisconnected,
    /// A new conversation with `agent_id`; `None` is the hub's default agent.
    NewSession {
        agent_id: Option<String>,
    },
    /// An earlier conversation, with the agent the hub listed it with.
    OpenSession {
        id: String,
        agent_id: Option<String>,
    },
    DeviceChanged,
    Delta {
        request_id: String,
        text: String,
    },
    Completed {
        request_id: String,
        text: Option<String>,
    },
    Approval(Approval),
    Interrupt,
    StartMeeting {
        id: String,
        title: String,
        audio: MeetingAudio,
        duration_ms: i64,
    },
    CaptureStarted,
    CaptureFinalized,
    EmptyCaptureClosed,
    PauseMeeting,
    ResumeMeeting,
    FinishMeeting,
    ChunkSealed,
    CaptureGap,
    VoiceStart,
    VoiceEnd,
    CallStart {
        id: String,
        duration_ms: i64,
    },
    CallStop,
    CallReady,
    CallPhase(crate::voice::CallPhase),
    CallTurnStarted,
    /// The user asked for their privacy stop: the app's button or tray,
    /// `orbit stop`, ⌃⌥Esc or the helper's own menu. The only event that
    /// latches as theirs.
    PrivacyStop,
    /// The helper held a privacy stop on disk when it reported its state.
    /// Adopted so the daemon and the helper say the same; who pressed it is
    /// unknown, and the stop is kept.
    HelperLatched,
    LocalRearm,
    HelperReady,
    HelperLost,
    PermissionObserved {
        microphone: PermissionState,
        screen: PermissionState,
        accessibility: PermissionState,
        calendar: crate::calendar::CalendarAuthorization,
    },
    CalendarAuthorization(crate::calendar::CalendarAuthorization),
    HelperStopUndelivered,
    Tick,
    /// A failed chat turn, carrying the hub code rather than its message.
    Failure(String),
    Degraded(DegradedKind),
    /// The helper refused an `audio_start` for a non-stop reason (a missing
    /// permission, an application scope, etc.). The recording is moved to
    /// `Interrupted` and a Spanish notice names the cause; no latch, no
    /// effects. Used by the daemon after `classify_native_response` returns
    /// `Refused` for an audio start.
    CaptureRefused {
        reason: String,
    },
    /// A failure put the live capture's integrity at risk, or the Mac
    /// stopped it on its own: the recording, call or voice note is cut with
    /// its cause (a code: a helper code or a stop cause's name) and what was
    /// kept. Orbit keeps working; nothing latches,
    /// nothing is resumed.
    CaptureCut {
        cause: String,
    },
    /// Orbit is closing (shutdown, IPC listener lost): the capture is cut,
    /// the turn and network work are retired, and nothing latches, so the
    /// next start comes back working.
    Close {
        cause: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// One turn to Deimos (`chat.send`). `voice` when it is a voice note;
    /// `meeting` when it is a question about a meeting, which the daemon
    /// fills in (the hub adds the transcript).
    SendText {
        id: String,
        text: String,
        voice: Option<crate::wire::hub::ChatVoice>,
        meeting: Option<crate::wire::hub::ChatMeeting>,
    },
    Interrupt,
    StartCapture {
        recording_id: String,
        audio: MeetingAudio,
    },
    StopCapture,
    /// Stop every capture source in the helper without touching its durable
    /// latch, and wait for the drain. `StopCapture` under a privacy
    /// stop asks the helper for the user's latching stop instead.
    CutCapture,
    StartVoice,
    FinishVoice,
    StopPlayback,
    CancelNetwork,
    StartCall,
    StopCall,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub state: State,
    pub effects: Vec<Effect>,
    pub authority_change: Option<AuthorityChange>,
}

pub fn decide(state: &State, event: Event, now_ms: i64) -> Result<Decision, Refusal> {
    let mut next = state.clone();
    let mut effects = Vec::new();
    let mut authority_change = None;
    match event {
        Event::Connecting => {
            if state.durable.privacy_stopped || state.durable.connection == Connection::Revoked {
                return Err(Refusal::PrivacyStopped { since_ms: None });
            }
            next.durable.connection = Connection::Connecting;
        }
        Event::Connect(connection) => {
            authority_change =
                if connection != state.durable.connection || connection == Connection::Revoked {
                    Some(match connection {
                        Connection::Revoked => AuthorityChange::Revoked,
                        Connection::Offline => AuthorityChange::HubLinkOffline,
                        Connection::Connecting | Connection::Online => AuthorityChange::HubLink,
                    })
                } else {
                    None
                };
            decide_capabilities(&mut next, &mut effects, connection);
        }
        Event::HubTransportReady => {
            authority_change = Some(AuthorityChange::HubTransportReady);
        }
        Event::HubDisconnected => {
            authority_change = Some(AuthorityChange::HubDisconnected);
            if next.durable.active_request.is_some() {
                decide_chat_failure(&mut next, NoticeCode::AssistantTurnLostByDisconnect);
            }
            decide_capabilities(&mut next, &mut effects, Connection::Offline);
        }
        Event::Session(id) => {
            if state
                .durable
                .session_id
                .as_ref()
                .is_some_and(|previous| previous != &id)
            {
                authority_change = Some(AuthorityChange::Conversation);
            }
            next.durable.session_id = Some(id);
        }
        Event::NewSession { agent_id } => {
            authority_change = Some(AuthorityChange::Conversation);
            decide_new_session(&mut next);
            next.durable.agent_id = agent_id;
        }
        Event::OpenSession { id, agent_id } => {
            authority_change = Some(AuthorityChange::Conversation);
            decide_open_session(&mut next, id);
            next.durable.agent_id = agent_id;
        }
        Event::DeviceChanged => {
            authority_change = Some(AuthorityChange::Device);
            next.durable.session_id = None;
            next.durable.agent_id = None;
            next.durable.active_request = None;
            next.durable.pending_approval = None;
        }
        Event::HelperReady => next.durable.capabilities.native_helper = true,
        Event::HelperLost => next.durable.capabilities = Capabilities::default(),
        Event::PermissionObserved {
            microphone,
            screen,
            accessibility,
            calendar,
        } => {
            next.durable.capabilities.microphone_state = microphone;
            next.durable.capabilities.screen_state = screen;
            next.durable.capabilities.accessibility_state = accessibility;
            next.durable.capabilities.calendar = Some(calendar);
        }
        Event::CalendarAuthorization(authorization) => {
            next.durable.capabilities.calendar = Some(authorization);
        }
        Event::HelperStopUndelivered => next.durable.capabilities.native_helper = false,
        Event::Submit { id, text } => {
            decide_chat_submit(state, &mut next, &mut effects, id, text, now_ms)?
        }
        Event::SubmitVoice {
            id,
            text,
            duration_ms,
        } => decide_voice_submit(
            state,
            &mut next,
            &mut effects,
            id,
            text,
            duration_ms,
            now_ms,
        )?,
        Event::VoiceStored { id, duration_ms } => {
            decide_voice_stored(state, &mut next, id, duration_ms, now_ms)?
        }
        Event::VoiceFailed { id, failure } => {
            if let Some(voice) = next
                .durable
                .turn
                .as_mut()
                .and_then(|turn| turn.untranscribed_voice_mut(&id))
            {
                voice.failure = Some(failure);
            }
        }
        Event::Delta { request_id, text } => decide_chat_delta(&mut next, request_id, text, now_ms),
        Event::Completed { request_id, text } => {
            decide_chat_completed(&mut next, request_id, text, now_ms)
        }
        Event::Approval(approval) => decide_chat_approval(&mut next, approval),
        Event::Interrupt => {
            authority_change = Some(AuthorityChange::Interrupt);
            decide_chat_interrupt(&mut next, &mut effects);
        }
        Event::Failure(code) => {
            let notice = if code == "session_not_found" {
                decide_new_session(&mut next);
                NoticeCode::AssistantSessionNotFound
            } else {
                NoticeCode::AssistantFailed
            };
            decide_chat_failure(
                &mut next,
                Notice::new(notice).with_token(NoticeKey::Code, &code),
            );
        }
        event @ (Event::StartMeeting { .. }
        | Event::CaptureStarted
        | Event::PauseMeeting
        | Event::ResumeMeeting
        | Event::FinishMeeting
        | Event::CaptureFinalized
        | Event::EmptyCaptureClosed
        | Event::ChunkSealed
        | Event::CaptureGap) => decide_capture(state, &mut next, &mut effects, event, now_ms)?,
        Event::VoiceStart => decide_voice_start(state, &mut next, &mut effects)?,
        Event::VoiceEnd => decide_voice_end(state, &mut next, &mut effects),
        event @ (Event::CallStart { .. }
        | Event::CallStop
        | Event::CallReady
        | Event::CallPhase(_)
        | Event::CallTurnStarted) => decide_call(state, &mut next, &mut effects, event, now_ms)?,
        Event::PrivacyStop => {
            if state.durable.privacy_stopped {
                next.durable.privacy_stop_by = Some(StopRequester::User);
            } else {
                authority_change = Some(AuthorityChange::UserStop);
                decide_privacy_stop(&mut next, &mut effects, Some(StopRequester::User));
            }
        }
        Event::HelperLatched => {
            if state.durable.privacy_stopped {
                if state.durable.privacy_stop_by == Some(StopRequester::Revocation) {
                    next.durable.privacy_stop_by = None;
                }
            } else {
                authority_change = Some(AuthorityChange::UserStop);
                decide_privacy_stop(&mut next, &mut effects, None);
            }
        }
        Event::LocalRearm => {
            decide_privacy_rearm(state, &mut next)?;
            // The base also withdrew control and meeting detection when
            // there was no latch to clear. Only that no-op uses the narrow
            // plan; a generation change closes the base's old authority.
            authority_change = Some(if state.durable.privacy_stopped {
                AuthorityChange::Rearm
            } else {
                AuthorityChange::RearmWithoutStop
            });
        }
        Event::Tick => decide_tick(state, &mut next, &mut effects, now_ms),
        Event::Degraded(kind) => decide_degraded(&mut next, kind),
        Event::CaptureRefused { reason } => {
            let voice_only = next.durable.voice_active
                && !next
                    .durable
                    .recording
                    .as_ref()
                    .is_some_and(|recording| recording.phase.interrupted_on_recovery());
            interrupt_recording(&mut next);
            next.durable.voice_active = false;
            if voice_only {
                next.set_notice(capture_refused_notice(
                    &reason,
                    NoticeCode::VoiceNoteRefused,
                ));
            } else {
                next.set_notice(capture_refused_notice(&reason, NoticeCode::CaptureRefused));
            }
        }
        Event::CaptureCut { cause } => decide_capture_cut(&mut next, &mut effects, &cause, false),
        Event::Close { cause } => {
            authority_change = Some(AuthorityChange::Closing);
            decide_capture_cut(&mut next, &mut effects, &cause, true);
            if next.durable.connection != Connection::Revoked {
                next.durable.connection = Connection::Offline;
            }
            next.durable.generation = next.durable.generation.saturating_add(1);
            next.durable.active_request = None;
            next.durable.pending_approval = None;
            effects.extend([Effect::Interrupt, Effect::CancelNetwork]);
        }
    }
    Ok(Decision {
        state: next,
        effects,
        authority_change,
    })
}

fn decide_new_session(next: &mut State) {
    next.durable.session_id = None;
    next.durable.turn = None;
    next.durable.pending_approval = None;
}

/// The conversation's messages are the hub's; the app reads them.
fn decide_open_session(next: &mut State, id: String) {
    next.durable.turn = None;
    next.durable.pending_approval = None;
    next.durable.session_id = Some(id);
}

fn decide_capabilities(next: &mut State, effects: &mut Vec<Effect>, connection: Connection) {
    if connection != Connection::Online && next.durable.call.take().is_some() {
        next.durable.active_request = None;
        next.durable.pending_approval = None;
        effects.extend([Effect::StopCall, Effect::StopPlayback, Effect::Interrupt]);
        next.set_notice(NoticeCode::CallStoppedByConnection);
    }
    if connection == Connection::Revoked {
        next.durable.generation = next.durable.generation.saturating_add(1);
        // Remote traffic may stop Orbit, never rearm it. A stop the user already
        // pressed stays theirs; otherwise this one is the hub's to repeat.
        if !next.durable.privacy_stopped {
            next.durable.privacy_stop_by = Some(StopRequester::Revocation);
        }
        next.durable.privacy_stopped = true;
        next.durable.voice_active = false;
        next.durable.active_request = None;
        next.durable.pending_approval = None;
        interrupt_recording(next);
        effects.extend([
            Effect::StopCapture,
            Effect::StopPlayback,
            Effect::Interrupt,
            Effect::CancelNetwork,
        ]);
        next.set_notice(NoticeCode::DeviceRevoked);
    }
    next.durable.connection = connection;
}

fn decide_chat_submit(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    id: String,
    text: String,
    now_ms: i64,
) -> Result<(), Refusal> {
    ensure_active(state)?;
    if state.durable.connection != Connection::Online {
        return Err(Refusal::HubUnreachable { since_ms: None });
    }
    if state.durable.active_request.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    let received_bytes = text.len();
    let text = text.trim().to_owned();
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Text,
            expected: crate::refusal::Expected::TextLength {
                bytes: (received_bytes) as u64,
                max_bytes: (MAX_TEXT_BYTES) as u64,
            },
        });
    }
    next.durable.turn = Some(Turn {
        user: Message {
            id: id.clone(),
            role: "user".into(),
            text: text.clone(),
            complete: true,
            at_ms: observed_at_ms(now_ms),
            voice: None,
        },
        answer: None,
    });
    next.durable.active_request = Some(id.clone());
    next.set_notice(NoticeCode::AssistantThinking);
    effects.push(Effect::SendText {
        id,
        text,
        voice: None,
        meeting: None,
    });
    Ok(())
}

fn decide_voice_stored(
    state: &State,
    next: &mut State,
    id: String,
    duration_ms: u64,
    now_ms: i64,
) -> Result<(), Refusal> {
    ensure_active(state)?;
    if state.durable.connection != Connection::Online {
        return Err(Refusal::HubUnreachable { since_ms: None });
    }
    if state.durable.active_request.is_some()
        || next
            .durable
            .turn
            .as_ref()
            .is_some_and(|turn| turn.user.id == id)
    {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    next.durable.turn = Some(Turn {
        user: Message {
            id,
            role: "user".into(),
            text: String::new(),
            complete: true,
            at_ms: observed_at_ms(now_ms),
            voice: Some(VoiceNote {
                duration_ms,
                transcript: None,
                failure: None,
                audio_available: true,
            }),
        },
        answer: None,
    });
    next.set_notice(NoticeCode::VoiceNoteSaved);
    Ok(())
}

fn decide_voice_submit(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    id: String,
    text: String,
    duration_ms: u64,
    now_ms: i64,
) -> Result<(), Refusal> {
    ensure_active(state)?;
    if state.durable.connection != Connection::Online {
        return Err(Refusal::HubUnreachable { since_ms: None });
    }
    if state.durable.active_request.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    let received_bytes = text.len();
    let text = text.trim().to_owned();
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Text,
            expected: crate::refusal::Expected::TextLength {
                bytes: (received_bytes) as u64,
                max_bytes: (MAX_TEXT_BYTES) as u64,
            },
        });
    }
    let voice = VoiceNote {
        duration_ms,
        transcript: Some(text.clone()),
        failure: None,
        audio_available: true,
    };
    // The voice note stored before its transcription becomes the turn's
    // message; a dictation without one starts its own turn.
    match next
        .durable
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == id && turn.user.voice.is_some())
    {
        Some(turn) => {
            turn.user.text.clone_from(&text);
            turn.user.voice = Some(voice);
        }
        None => {
            next.durable.turn = Some(Turn {
                user: Message {
                    id: id.clone(),
                    role: "user".into(),
                    text: text.clone(),
                    complete: true,
                    at_ms: observed_at_ms(now_ms),
                    voice: Some(voice),
                },
                answer: None,
            });
        }
    }
    next.durable.active_request = Some(id.clone());
    next.set_notice(NoticeCode::AssistantThinking);
    effects.push(Effect::SendText {
        id,
        text,
        voice: Some(crate::wire::hub::ChatVoice { duration_ms }),
        meeting: None,
    });
    Ok(())
}

fn decide_chat_delta(next: &mut State, request_id: String, text: String, now_ms: i64) {
    if next.durable.active_request.as_deref() != Some(&request_id) {
        return;
    }
    let Some(turn) = next
        .durable
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == request_id)
    else {
        return;
    };
    if let Some(answer) = turn.answer.as_mut() {
        if answer.text.len().saturating_add(text.len()) <= MAX_TEXT_BYTES * 3 {
            answer.text.push_str(&text);
        }
        return;
    }
    let end = clamp_utf8(&text, MAX_TEXT_BYTES);
    turn.answer = Some(Message {
        id: format!("{request_id}:assistant"),
        role: "assistant".into(),
        text: text[..end].to_owned(),
        complete: false,
        at_ms: answer_at_ms(&turn.user, now_ms),
        voice: None,
    });
    next.set_notice(NoticeCode::AssistantResponding);
}

fn decide_chat_completed(next: &mut State, request_id: String, text: Option<String>, now_ms: i64) {
    if next.durable.active_request.as_deref() != Some(&request_id) {
        return;
    }
    next.durable.active_request = None;
    next.durable.pending_approval = None;
    if let Some(turn) = next
        .durable
        .turn
        .as_mut()
        .filter(|turn| turn.user.id == request_id)
    {
        if let Some(text) = text {
            let end = clamp_utf8(&text, MAX_TEXT_BYTES * 3);
            let text = text[..end].to_owned();
            match turn.answer.as_mut() {
                Some(answer) => answer.text = text,
                None => {
                    turn.answer = Some(Message {
                        id: format!("{request_id}:assistant"),
                        role: "assistant".into(),
                        text,
                        complete: true,
                        at_ms: answer_at_ms(&turn.user, now_ms),
                        voice: None,
                    });
                }
            }
        }
        if let Some(answer) = turn.answer.as_mut() {
            answer.complete = true;
        }
    }
    next.set_notice(NoticeCode::AssistantAnswered);
}

fn observed_at_ms(now_ms: i64) -> u64 {
    u64::try_from(now_ms).unwrap_or_default()
}

/// The answer is never before the question, whatever the wall clock did.
fn answer_at_ms(user: &Message, now_ms: i64) -> u64 {
    observed_at_ms(now_ms).max(user.at_ms.saturating_add(1))
}

fn decide_chat_approval(next: &mut State, approval: Approval) {
    if next.durable.active_request.as_deref() == Some(approval.request_id.as_str()) {
        next.durable.pending_approval = Some(approval);
    } else {
        next.set_notice(NoticeCode::AssistantStaleQuestion);
    }
}

fn decide_chat_interrupt(next: &mut State, effects: &mut Vec<Effect>) {
    next.durable.active_request = None;
    next.durable.pending_approval = None;
    effects.extend([Effect::Interrupt, Effect::StopPlayback]);
    next.set_notice(NoticeCode::AssistantInterruptRequested);
}

fn decide_chat_failure(next: &mut State, notice: impl Into<Notice>) {
    next.set_notice(notice);
    next.durable.active_request = None;
    next.durable.pending_approval = None;
}

fn decide_capture(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    event: Event,
    now_ms: i64,
) -> Result<(), Refusal> {
    match event {
        Event::StartMeeting {
            id,
            title,
            audio,
            duration_ms,
        } => decide_capture_start(state, next, effects, id, title, audio, duration_ms, now_ms),
        Event::CaptureStarted => decide_capture_started(next),
        Event::PauseMeeting => decide_capture_pause(next, effects),
        Event::ResumeMeeting => decide_capture_resume(state, next, effects, now_ms),
        Event::FinishMeeting => decide_capture_finish(state, next, effects, now_ms),
        Event::CaptureFinalized => decide_capture_finalized(next, now_ms),
        Event::EmptyCaptureClosed => {
            let recording = next
                .durable
                .recording
                .as_mut()
                .ok_or(Refusal::InvalidParameter {
                    parameter: crate::refusal::Parameter::Request,
                    expected: crate::refusal::Expected::CorrectCaptureState,
                })?;
            if recording.phase != CapturePhase::Interrupted
                || recording.sealed_chunks != 0
                || recording.ended_at_ms.is_some()
            {
                return Err(Refusal::InvalidParameter {
                    parameter: crate::refusal::Parameter::Request,
                    expected: crate::refusal::Expected::CorrectCaptureState,
                });
            }
            recording.ended_at_ms = Some(now_ms.max(recording.started_at_ms));
            recording.finalizing_since_ms = None;
            next.set_notice(NoticeCode::RecordingEmpty);
            Ok(())
        }
        Event::ChunkSealed => {
            if let Some(r) = next.durable.recording.as_mut() {
                r.sealed_chunks += 1;
            }
            Ok(())
        }
        Event::CaptureGap => {
            if let Some(r) = next.durable.recording.as_mut() {
                r.gaps = r.gaps.saturating_add(1);
            }
            next.set_notice(NoticeCode::AudioGap);
            Ok(())
        }
        _ => unreachable!("decide_capture only receives capture events"),
    }
}

#[allow(clippy::too_many_arguments)]
fn decide_capture_start(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    id: String,
    title: String,
    audio: MeetingAudio,
    duration_ms: i64,
    now_ms: i64,
) -> Result<(), Refusal> {
    if state.durable.call.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::AudioSession,
            retry_after_ms: None,
        });
    }
    ensure_microphone(state)?;
    if state
        .durable
        .recording
        .as_ref()
        .is_some_and(|r| !matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted))
    {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Recording,
            retry_after_ms: None,
        });
    }
    let allowed_duration_ms = 60_000..=14_400_000;
    if !allowed_duration_ms.contains(&duration_ms) {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Duration,
            expected: crate::refusal::Expected::DurationRange {
                actual_ms: duration_ms,
                min_ms: *allowed_duration_ms.start(),
                max_ms: *allowed_duration_ms.end(),
            },
        });
    }
    let application_pid = match audio {
        MeetingAudio::Application { application_pid } if application_pid > 0 => {
            Some(application_pid)
        }
        MeetingAudio::Application { .. } => {
            return Err(Refusal::InvalidParameter {
                parameter: crate::refusal::Parameter::Meeting,
                expected: crate::refusal::Expected::ValidMeeting,
            });
        }
        MeetingAudio::System | MeetingAudio::MicrophoneOnly => None,
    };
    if !matches!(audio, MeetingAudio::MicrophoneOnly)
        && let Some(refusal) = permission_refusal(
            state.durable.capabilities.screen_state,
            crate::refusal::Permission::ScreenRecording,
        )
    {
        return Err(refusal);
    }
    next.durable.recording = Some(Recording {
        id: id.clone(),
        title,
        started_at_ms: now_ms,
        ended_at_ms: None,
        expires_at_ms: now_ms.saturating_add(duration_ms),
        application_pid,
        audio,
        phase: CapturePhase::Starting,
        sealed_chunks: 0,
        gaps: 0,
        tracks: match audio {
            MeetingAudio::MicrophoneOnly => vec![TrackProgress {
                name: "mic_local".into(),
                ..TrackProgress::default()
            }],
            MeetingAudio::System | MeetingAudio::Application { .. } => vec![
                TrackProgress {
                    name: "mic_local".into(),
                    ..TrackProgress::default()
                },
                TrackProgress {
                    name: "remote_app".into(),
                    ..TrackProgress::default()
                },
            ],
        },
        finalizing_since_ms: None,
    });
    effects.push(Effect::StartCapture {
        recording_id: id,
        audio,
    });
    next.set_notice(NoticeCode::CaptureStarting);
    Ok(())
}

fn decide_capture_started(next: &mut State) -> Result<(), Refusal> {
    let recording = next
        .durable
        .recording
        .as_mut()
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    if recording.phase != CapturePhase::Starting {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        });
    }
    recording.phase = CapturePhase::Recording;
    next.set_notice(NoticeCode::RecordingStarted);
    Ok(())
}

fn decide_capture_pause(next: &mut State, effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    let recording = next
        .durable
        .recording
        .as_mut()
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    if recording.phase != CapturePhase::Recording {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        });
    }
    recording.phase = CapturePhase::Paused;
    effects.push(Effect::StopCapture);
    next.set_notice(NoticeCode::RecordingPaused);
    Ok(())
}

fn decide_capture_resume(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    now_ms: i64,
) -> Result<(), Refusal> {
    if state.durable.call.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::AudioSession,
            retry_after_ms: None,
        });
    }
    ensure_microphone(state)?;
    let recording = next
        .durable
        .recording
        .as_mut()
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    if recording.ended_at_ms.is_some() {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::OpenRecording,
        });
    }
    if recording.expires_at_ms <= now_ms {
        return Err(Refusal::Expired {
            resource: crate::refusal::Resource::Recording,
            expired_at_ms: None,
        });
    }
    if !matches!(
        recording.phase,
        CapturePhase::Paused | CapturePhase::Interrupted
    ) {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        });
    }
    recording.phase = CapturePhase::Starting;
    effects.push(Effect::StartCapture {
        recording_id: recording.id.clone(),
        audio: recording.audio,
    });
    Ok(())
}

fn decide_capture_finish(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    now_ms: i64,
) -> Result<(), Refusal> {
    let recording = next
        .durable
        .recording
        .as_mut()
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    if recording.ended_at_ms.is_some() {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::OpenRecording,
        });
    }
    if matches!(recording.phase, CapturePhase::Finalizing) {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        });
    }
    // An interrupted capture or one finalised under the privacy brake
    // has no producer left to drain, so we close the recording locally
    // instead of parking it in `Finalizing` waiting for an ACK that will
    // never come.
    if matches!(recording.phase, CapturePhase::Interrupted) || state.durable.privacy_stopped {
        recording.phase = CapturePhase::Idle;
        recording.ended_at_ms = Some(now_ms);
        recording.finalizing_since_ms = None;
        next.set_notice(NoticeCode::RecordingFinished);
    } else {
        recording.phase = CapturePhase::Finalizing;
        recording.finalizing_since_ms = Some(now_ms);
        effects.push(Effect::StopCapture);
        next.set_notice(NoticeCode::RecordingFinishing);
    }
    Ok(())
}

fn decide_capture_finalized(next: &mut State, now_ms: i64) -> Result<(), Refusal> {
    let recording = next
        .durable
        .recording
        .as_mut()
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    if recording.phase != CapturePhase::Finalizing {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        });
    }
    recording.phase = CapturePhase::Idle;
    recording.ended_at_ms = Some(now_ms);
    recording.finalizing_since_ms = None;
    next.set_notice(NoticeCode::RecordingFinished);
    Ok(())
}

fn decide_voice_start(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
) -> Result<(), Refusal> {
    if state.durable.call.is_some() {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::AudioSession,
            retry_after_ms: None,
        });
    }
    ensure_microphone(state)?;
    if state.durable.voice_active
        || state.durable.active_request.is_some()
        || state.durable.pending_approval.is_some()
    {
        return Err(Refusal::Busy {
            operation: crate::refusal::Operation::Interaction,
            retry_after_ms: None,
        });
    }
    next.durable.voice_active = true;
    effects.extend([Effect::StopPlayback, Effect::StartVoice]);
    Ok(())
}

fn decide_voice_end(state: &State, next: &mut State, effects: &mut Vec<Effect>) {
    if state.durable.voice_active {
        next.durable.voice_active = false;
        effects.push(Effect::FinishVoice);
    }
}

fn decide_call(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    event: Event,
    now_ms: i64,
) -> Result<(), Refusal> {
    match event {
        Event::CallStart { id, duration_ms } => {
            ensure_microphone(state)?;
            if state.durable.connection != Connection::Online {
                return Err(Refusal::HubUnreachable { since_ms: None });
            }
            if !state.durable.capabilities.voice_processing_supported {
                return Err(Refusal::EngineUnavailable {
                    cause: crate::refusal::EngineCause::VoiceProcessingUnsupported,
                });
            }
            if let Some(recording) = state.durable.recording.as_ref().filter(|recording| {
                recording.ended_at_ms.is_none() && recording.phase != CapturePhase::Idle
            }) {
                return Err(Refusal::PendingRecording {
                    id: recording.id.clone(),
                    title: recording.title.clone(),
                    phase: recording.phase.clone(),
                    sealed_chunks: recording.sealed_chunks,
                    gaps: recording.gaps,
                    expires_at_ms: recording.expires_at_ms,
                });
            }
            if state.durable.call.is_some()
                || state.durable.voice_active
                || state.durable.active_request.is_some()
            {
                return Err(Refusal::Busy {
                    operation: crate::refusal::Operation::AudioSession,
                    retry_after_ms: None,
                });
            }
            let allowed_duration_ms = 60_000..=3_600_000;
            if !allowed_duration_ms.contains(&duration_ms) {
                return Err(Refusal::InvalidParameter {
                    parameter: crate::refusal::Parameter::Duration,
                    expected: crate::refusal::Expected::DurationRange {
                        actual_ms: duration_ms,
                        min_ms: *allowed_duration_ms.start(),
                        max_ms: *allowed_duration_ms.end(),
                    },
                });
            }
            next.durable.call = Some(crate::voice::CallState {
                id,
                phase: crate::voice::CallPhase::Starting,
                started_at_ms: now_ms,
                expires_at_ms: now_ms.saturating_add(duration_ms),
                turns: 0,
            });
            effects.extend([Effect::StopPlayback, Effect::StartCall]);
            next.set_notice(NoticeCode::CallStarting);
            Ok(())
        }
        Event::CallStop => {
            if next.durable.call.take().is_some() {
                next.durable.active_request = None;
                next.durable.pending_approval = None;
                effects.extend([Effect::StopPlayback, Effect::Interrupt, Effect::StopCall]);
                next.set_notice(NoticeCode::CallStopped);
            }
            Ok(())
        }
        Event::CallReady => {
            if let Some(call) = next.durable.call.as_mut() {
                call.phase = crate::voice::CallPhase::Listening;
                next.set_notice(NoticeCode::CallActive);
            }
            Ok(())
        }
        Event::CallPhase(phase) => {
            if let Some(call) = next.durable.call.as_mut() {
                call.phase = phase;
            }
            Ok(())
        }
        Event::CallTurnStarted => {
            if let Some(call) = next.durable.call.as_mut() {
                call.phase = crate::voice::CallPhase::UserSpeaking;
                call.turns = call.turns.saturating_add(1);
            }
            Ok(())
        }
        _ => unreachable!("decide_call only receives call events"),
    }
}

/// The only decision that latches a stop the user can come back to.
/// `requester` is `None` only for a latch the helper already held: a stop
/// the user pressed again stays theirs, and nothing downgrades one of theirs.
fn decide_privacy_stop(
    next: &mut State,
    effects: &mut Vec<Effect>,
    requester: Option<StopRequester>,
) {
    if next.durable.call.take().is_some() {
        effects.push(Effect::StopCall);
    }
    next.durable.generation = next.durable.generation.saturating_add(1);
    if requester.is_some() || !next.durable.privacy_stopped {
        next.durable.privacy_stop_by = requester;
    }
    next.durable.privacy_stopped = true;
    if next.durable.connection != Connection::Revoked {
        next.durable.connection = Connection::Offline;
    }
    next.durable.voice_active = false;
    next.durable.active_request = None;
    next.durable.pending_approval = None;
    interrupt_recording(next);
    effects.extend([
        Effect::StopCapture,
        Effect::StopPlayback,
        Effect::Interrupt,
        Effect::CancelNetwork,
    ]);
    next.set_notice(NoticeCode::PrivacyStopped);
}

fn decide_privacy_rearm(state: &State, next: &mut State) -> Result<(), Refusal> {
    if state.durable.connection == Connection::Revoked {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    if !state.durable.privacy_stopped {
        // A rearm that finds nothing stopped is a no-op.
        return Ok(());
    }
    next.durable.generation = next.durable.generation.saturating_add(1);
    next.durable.privacy_stopped = false;
    next.durable.privacy_stop_by = None;
    next.set_notice(NoticeCode::PrivacyRearmed);
    Ok(())
}

fn decide_tick(state: &State, next: &mut State, effects: &mut Vec<Effect>, now_ms: i64) {
    let _ = state;
    if next
        .durable
        .call
        .as_ref()
        .is_some_and(|call| call.expires_at_ms <= now_ms)
    {
        next.durable.call = None;
        next.durable.active_request = None;
        next.durable.pending_approval = None;
        effects.extend([Effect::StopCall, Effect::StopPlayback, Effect::Interrupt]);
        next.set_notice(NoticeCode::CallExpired);
    }
    if next.durable.recording.as_ref().is_some_and(|r| {
        r.expires_at_ms <= now_ms
            && matches!(r.phase, CapturePhase::Recording | CapturePhase::Starting)
    }) {
        interrupt_recording(next);
        effects.push(Effect::StopCapture);
        let has_fragments = next
            .durable
            .recording
            .as_ref()
            .is_some_and(|recording| recording.sealed_chunks > 0);
        // The ceiling it was started with (by hand, from the calendar or for
        // a call), in whole minutes: there is no permission to expire.
        let minutes = next.durable.recording.as_ref().map_or(0, |recording| {
            recording
                .expires_at_ms
                .saturating_sub(recording.started_at_ms)
                .max(0)
                / 60_000
        });
        next.set_notice(
            Notice::new(NoticeCode::RecordingCeilingReached)
                .with(NoticeKey::Minutes, minutes)
                .with(NoticeKey::Fragments, has_fragments),
        );
    }
    // Bound the wait for the Mac's drain ACK. If the confirmation does
    // not arrive, the recording falls back to `Interrupted` with a registered
    // gap so the next tick or a later review can detect it.
    if let Some(recording) = next.durable.recording.as_mut()
        && recording.phase == CapturePhase::Finalizing
        && let Some(since) = recording.finalizing_since_ms
        && now_ms > since.saturating_add(FINALIZING_TIMEOUT_MS)
    {
        recording.phase = CapturePhase::Interrupted;
        recording.gaps = recording.gaps.saturating_add(1);
        recording.finalizing_since_ms = None;
        let has_fragments = recording.sealed_chunks > 0;
        next.set_notice(
            Notice::new(NoticeCode::RecordingCloseUnconfirmed)
                .with(NoticeKey::Fragments, has_fragments),
        );
    }
}

/// Cuts whatever is live: the recording is interrupted with its
/// fragments counted, a call ends with its turn, a voice note is dropped.
/// The helper is asked to stop capturing without latching, and nothing is
/// ever resumed from here. `closing` says Orbit itself is going away, so the
/// notice does not promise it keeps working.
fn decide_capture_cut(next: &mut State, effects: &mut Vec<Effect>, cause: &str, closing: bool) {
    // The cause is a code (a helper code or a stop cause's name); anything
    // else is kept only as `unrecognized`, never as a sentence.
    let mut notice = Notice::new(if closing {
        NoticeCode::OrbitClosing
    } else {
        NoticeCode::CaptureCut
    })
    .with_token(NoticeKey::Cause, cause);
    if let Some(recording) = next.durable.recording.as_mut()
        && matches!(
            recording.phase,
            CapturePhase::Recording | CapturePhase::Starting | CapturePhase::Finalizing
        )
    {
        recording.phase = CapturePhase::Interrupted;
        notice = notice
            .with(NoticeKey::Recording, true)
            .with(NoticeKey::Fragments, recording.sealed_chunks);
    }
    if next.durable.call.take().is_some() {
        next.durable.active_request = None;
        next.durable.pending_approval = None;
        effects.push(Effect::StopCall);
        if !closing {
            effects.push(Effect::Interrupt);
        }
        notice = notice.with(NoticeKey::Call, true);
    }
    if next.durable.voice_active {
        next.durable.voice_active = false;
        notice = notice.with(NoticeKey::VoiceNote, true);
    }
    effects.extend([Effect::CutCapture, Effect::StopPlayback]);
    next.set_notice(notice);
}

fn decide_degraded(next: &mut State, kind: DegradedKind) {
    if matches!(kind, DegradedKind::AudioGap) {
        if let Some(recording) = next.durable.recording.as_mut() {
            recording.gaps = recording.gaps.saturating_add(1);
        }
        next.set_notice(NoticeCode::AudioGap);
    }
}

/// Longest byte offset of `text` not exceeding `limit` that lies on a UTF-8
/// character boundary. Treating the byte budget as raw bytes (instead of
/// characters) preserves the `MAX_TEXT_BYTES` cap: cutting
/// `é` (two bytes) by characters would double the budget and a blind cut
/// could split a multi-byte character.
fn clamp_utf8(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn ensure_active(state: &State) -> Result<(), Refusal> {
    if state.durable.privacy_stopped {
        Err(Refusal::PrivacyStopped { since_ms: None })
    } else {
        Ok(())
    }
}
fn ensure_microphone(state: &State) -> Result<(), Refusal> {
    ensure_active(state)?;
    // Named apart on purpose: the helper being gone is a different problem from
    // a permission being refused, and only one of them is fixed in Settings.
    if !state.durable.capabilities.native_helper {
        return Err(Refusal::HelperNotRunning);
    }
    if let Some(refusal) = permission_refusal(
        state.durable.capabilities.microphone_state,
        crate::refusal::Permission::Microphone,
    ) {
        return Err(refusal);
    }
    Ok(())
}

fn permission_refusal(
    state: PermissionState,
    permission: crate::refusal::Permission,
) -> Option<Refusal> {
    match state {
        PermissionState::Granted => None,
        PermissionState::Denied => Some(Refusal::PermissionDenied { permission }),
        PermissionState::NotGranted => Some(Refusal::PermissionNotGranted { permission }),
        PermissionState::NotDetermined | PermissionState::Unknown => {
            Some(Refusal::PermissionUnknown { permission })
        }
    }
}
fn interrupt_recording(state: &mut State) {
    if let Some(recording) = state.durable.recording.as_mut()
        && matches!(
            recording.phase,
            CapturePhase::Recording | CapturePhase::Starting | CapturePhase::Finalizing
        )
    {
        recording.phase = CapturePhase::Interrupted;
    }
}

/// Spanish notice for a refused audio start. The known permission codes are
/// named; every other reason is shown verbatim so a future Swift file can be
/// reflected without an edit here.
/// A missing permission is something the user can grant; any other refusal is
/// said with the helper's code.
fn capture_refused_notice(reason: &str, otherwise: NoticeCode) -> Notice {
    match reason {
        "microphone_permission_required" => Notice::new(NoticeCode::MicrophonePermissionMissing),
        "screen_permission_required" => Notice::new(NoticeCode::ScreenPermissionMissing),
        _ => Notice::new(otherwise).with_token(NoticeKey::Cause, reason),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
                    duration_ms: 60_000,
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
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
            .turn
            .as_ref()
            .unwrap()
            .answer
            .as_ref()
            .unwrap();
        assert_eq!(answer.text, "respuesta pública");
        assert!(answer.complete);
        assert!(state.durable.active_request.is_none());
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
        assert_eq!(state.durable.turn, late.durable.turn);
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
        assert!(state.durable.privacy_stopped);
        assert_eq!(
            decide(
                &state,
                Event::Submit {
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
        assert!(!rearmed.state.durable.voice_active);
    }

    #[test]
    fn old_delta_after_interrupt_is_ignored() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
        assert_eq!(state.durable.turn, late.state.durable.turn);
    }

    #[test]
    fn recording_duration_ceiling_expires_and_resume_does_not_extend_it() {
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
                duration_ms: 60_000,
            },
            0,
        )
        .unwrap()
        .state;
        let state = decide(&state, Event::CaptureStarted, 0).unwrap().state;
        let expired = decide(&state, Event::Tick, 60_000).unwrap();
        assert!(expired.effects.contains(&Effect::StopCapture));
        assert_eq!(
            decide(&expired.state, Event::ResumeMeeting, 60_001).unwrap_err(),
            Refusal::Expired {
                resource: crate::refusal::Resource::Recording,
                expired_at_ms: None
            }
        );
    }

    #[test]
    fn late_approval_after_interrupt_is_ignored_and_voice_start_is_not_busy() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
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
            Event::Approval(Approval {
                request_id: "a".into(),
                description: "¿Publico el calendario?".into(),
                clarification: false,
            }),
            2,
        )
        .unwrap();
        assert!(
            after.state.durable.pending_approval.is_none(),
            "a late approval must not become pending: {:?}",
            after.state.durable.pending_approval
        );
        assert!(after.effects.is_empty());
        let voice = decide(&after.state, Event::VoiceStart, 3).unwrap();
        assert!(voice.state.durable.voice_active);
    }

    #[test]
    fn approval_matching_active_request_is_stored() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
                id: "a".into(),
                text: "hola".into(),
            },
            0,
        )
        .unwrap()
        .state;
        let approval = Approval {
            request_id: "a".into(),
            description: "¿Publico el calendario?".into(),
            clarification: false,
        };
        let after = decide(&state, Event::Approval(approval.clone()), 1).unwrap();
        assert_eq!(after.state.durable.pending_approval, Some(approval));
    }

    #[test]
    fn approval_after_completed_turn_is_ignored() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
            Event::Approval(Approval {
                request_id: "a".into(),
                description: "¿Publico el calendario?".into(),
                clarification: false,
            }),
            2,
        )
        .unwrap();
        assert!(
            after.state.durable.pending_approval.is_none(),
            "the turn already completed; its approval must be ignored: {:?}",
            after.state.durable.pending_approval
        );
        assert!(after.effects.is_empty());
        assert_eq!(state.durable.turn, after.state.durable.turn);
    }

    #[test]
    fn late_clarification_after_interrupt_is_ignored() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
            Event::Approval(Approval {
                request_id: "a".into(),
                description: "¿Qué reunión querías decir?".into(),
                clarification: true,
            }),
            2,
        )
        .unwrap();
        assert!(
            after.state.durable.pending_approval.is_none(),
            "a late clarification must be ignored like an approval: {:?}",
            after.state.durable.pending_approval
        );
        assert!(after.effects.is_empty());
        assert_eq!(state.durable.turn, after.state.durable.turn);
    }

    #[test]
    fn ignored_approval_records_notice_without_description() {
        let state = State {
            durable: crate::DurableState {
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
            Event::Approval(Approval {
                request_id: "a".into(),
                description: "SECRETO-DESCRIPCION".into(),
                clarification: false,
            }),
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
                recording: Some(recording),
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
            let recording = decision.state.durable.recording.as_ref().unwrap();
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

    /// A recording that reaches its ceiling says so with the
    /// minutes it was started with; there is no permission to expire.
    #[test]
    fn a_recording_at_its_ceiling_says_its_minutes() {
        let mut recording = recording_in_phase(CapturePhase::Recording);
        recording.started_at_ms = 0;
        recording.expires_at_ms = 240 * 60_000;
        let decision = decide(&state_with_recording(recording), Event::Tick, 240 * 60_000).unwrap();
        assert_eq!(
            decision.state.durable.notice,
            Some(
                Notice {
                    sequence: Some(1),
                    ..Notice::new(NoticeCode::RecordingCeilingReached)
                }
                .with(NoticeKey::Minutes, 240_i64)
                .with(NoticeKey::Fragments, true)
            )
        );
    }

    #[test]
    fn tick_past_the_finalizing_deadline_interrupts_with_gap_and_notice() {
        let since = 10_000;
        let state = state_with_recording(Recording {
            finalizing_since_ms: Some(since),
            ..recording_in_phase(CapturePhase::Finalizing)
        });
        let decision = decide(&state, Event::Tick, since + FINALIZING_TIMEOUT_MS + 1).unwrap();
        let recording = decision.state.durable.recording.as_ref().unwrap();
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
            ..recording_in_phase(CapturePhase::Finalizing)
        });
        let decision = decide(&state, Event::Tick, since + FINALIZING_TIMEOUT_MS).unwrap();
        let recording = decision.state.durable.recording.as_ref().unwrap();
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
        assert!(!rearmed.durable.privacy_stopped);
        let decision = decide(
            &rearmed,
            Event::StartMeeting {
                id: "next".into(),
                title: "Otra sesión".into(),
                audio: MeetingAudio::MicrophoneOnly,
                duration_ms: 60_000,
            },
            7_000,
        )
        .unwrap();
        assert_eq!(
            decision.state.durable.recording.as_ref().unwrap().id,
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
                connection: Connection::Online,
                ..crate::DurableState::default()
            },
            ..State::default()
        };
        let state = decide(
            &state,
            Event::Submit {
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
            duration_ms: 60_000,
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
        assert!(admitted.state.durable.recording.is_some());
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
}
