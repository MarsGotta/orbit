//! Flat-model regression oracle for unchanged policies. Duration retirement is
//! mirrored explicitly; the new silence policy has public transition tests in
//! `tests/silence_close.rs` instead of a second implementation here.
#![allow(clippy::all, dead_code)]
use super::model::State;
use crate::FINALIZING_TIMEOUT_MS;
use crate::authority::AuthorityChange;
use crate::failure::DegradedKind;
use crate::model::*;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::privacy_stop::StopRequester;
use crate::protocol::MeetingAudio;
use crate::refusal::Refusal;
use crate::transition::{Effect, Event};

#[path = "discard.rs"]
mod discard;
#[path = "voice_notes.rs"]
mod voice_notes;
use voice_notes::{
    adopt_new_conversation_notes, decide_voice_discard, decide_voice_failed, decide_voice_retry,
    decide_voice_stored, forget_voice_note, retire_turn, set_turn,
};

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
        Event::CallKeep | Event::RecordingKeep | Event::CheckSilence { .. } => {
            unreachable!("new policy tested through public transitions")
        }
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
        Event::Session { id, agent_id } => {
            if state
                .durable
                .session_id
                .as_ref()
                .is_some_and(|previous| previous != &id)
            {
                authority_change = Some(AuthorityChange::Conversation);
            }
            // Only a conversation the hub names for the first time: one
            // opened without an agent is from before agents, and the hub
            // keeps it with whichever agent is its default.
            if state.durable.session_id.is_none() && next.durable.agent_id.is_none() {
                next.durable.agent_id = agent_id;
            }
            if state.durable.session_id.is_none() {
                adopt_new_conversation_notes(&mut next, &id);
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
        Event::Submit {
            id,
            text,
            attachments,
        } => decide_chat_submit(
            state,
            &mut next,
            &mut effects,
            id,
            text,
            attachments,
            now_ms,
        )?,
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
        Event::VoiceRetry { id } => decide_voice_retry(state, &mut next, &id)?,
        Event::VoiceDiscard { id } => decide_voice_discard(&mut next, &id)?,
        Event::VoiceUnheard { id } => forget_voice_note(&mut next, &id),
        Event::VoiceTooShort(short) => {
            next.set_notice(
                Notice::new(NoticeCode::VoiceNoteTooShort)
                    .with(NoticeKey::DurationMs, short.duration_ms)
                    .with(NoticeKey::MinimumMs, short.minimum_ms),
            );
            next.views.short_voice_note = Some(short);
        }
        Event::VoiceFailed { id, failure } => decide_voice_failed(&mut next, &id, failure),
        Event::Delta { request_id, text } => decide_chat_delta(&mut next, request_id, text, now_ms),
        Event::Completed { request_id, text } => {
            decide_chat_completed(&mut next, request_id, text, now_ms)
        }
        Event::Approval { turn_id, approval } => {
            decide_chat_approval(&mut next, &turn_id, approval)
        }
        Event::Interrupt => {
            authority_change = Some(AuthorityChange::Interrupt);
            decide_chat_interrupt(&mut next, &mut effects);
        }
        Event::Failure { code, numbers } => {
            let notice = if code == "session_not_found" {
                decide_new_session(&mut next);
                NoticeCode::AssistantSessionNotFound
            } else {
                NoticeCode::AssistantFailed
            };
            let notice = numbers.into_iter().fold(
                Notice::new(notice).with_token(NoticeKey::Code, &code),
                |notice, (key, value)| notice.with_token(key, &value),
            );
            decide_chat_failure(&mut next, notice);
        }
        event @ (Event::StartMeeting { .. }
        | Event::CaptureStarted
        | Event::PauseMeeting
        | Event::ResumeMeeting
        | Event::FinishMeeting
        | Event::DiscardMeeting { .. }
        | Event::CaptureFinalized
        | Event::EmptyCaptureClosed
        | Event::CutRecordingClosed { .. }
        | Event::ChunkSealed
        | Event::CaptureGap) => decide_capture(state, &mut next, &mut effects, event, now_ms)?,
        Event::VoiceStart => decide_voice_start(state, &mut next, &mut effects)?,
        Event::VoiceEnd => decide_voice_end(state, &mut next, &mut effects),
        Event::VoiceCancel => {
            if state.durable.voice_active {
                next.durable.voice_active = false;
                effects.push(Effect::CancelVoice);
            }
        }
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
    discard::settle(&mut next);
    Ok(Decision {
        state: next,
        effects,
        authority_change,
    })
}

fn decide_new_session(next: &mut State) {
    // The turn is retired while its conversation is still the open one, so
    // a note it keeps stays that conversation's.
    retire_turn(next);
    next.durable.session_id = None;
    next.durable.pending_approval = None;
}

/// The conversation's messages are the hub's; the app reads them.
fn decide_open_session(next: &mut State, id: String) {
    retire_turn(next);
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
    attachments: Vec<crate::attachments::Attachment>,
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
    // A file alone is a question too: only a turn without files needs text.
    if (text.is_empty() && attachments.is_empty()) || text.len() > MAX_TEXT_BYTES {
        return Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Text,
            expected: crate::refusal::Expected::TextLength {
                bytes: (received_bytes) as u64,
                max_bytes: (MAX_TEXT_BYTES) as u64,
            },
        });
    }
    next.views.short_voice_note = None;
    set_turn(
        next,
        Turn {
            user: Message {
                id: id.clone(),
                role: "user".into(),
                text: text.clone(),
                complete: true,
                at_ms: observed_at_ms(now_ms),
                voice: None,
                attachments: attachments.clone(),
            },
            answer: None,
        },
    );
    next.durable.active_request = Some(id.clone());
    next.set_notice(NoticeCode::AssistantThinking);
    effects.push(Effect::SendText {
        id,
        text,
        voice: None,
        meeting: None,
        attachments: attachments
            .into_iter()
            .map(|attachment| attachment.id)
            .collect(),
    });
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
                    attachments: Vec::new(),
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
        attachments: Vec::new(),
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
        attachments: Vec::new(),
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
                        attachments: Vec::new(),
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

fn decide_chat_approval(next: &mut State, turn_id: &str, approval: Approval) {
    if next.durable.active_request.as_deref() == Some(turn_id) {
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
        Event::StartMeeting { id, title, audio } => {
            decide_capture_start(state, next, effects, id, title, audio, now_ms)
        }
        Event::CaptureStarted => decide_capture_started(next),
        Event::PauseMeeting => decide_capture_pause(next, effects),
        Event::ResumeMeeting => decide_capture_resume(state, next, effects, now_ms),
        Event::FinishMeeting => decide_capture_finish(state, next, effects, now_ms),
        Event::DiscardMeeting { recording_id } => {
            discard::decide_discard(state, next, effects, &recording_id, now_ms)
        }
        Event::CaptureFinalized if discard::drained(next) => Ok(()),
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
        Event::CutRecordingClosed { ended_at_ms, cause } => {
            decide_cut_recording_closed(next, ended_at_ms, &cause, now_ms)
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

fn decide_capture_start(
    state: &State,
    next: &mut State,
    effects: &mut Vec<Effect>,
    id: String,
    title: String,
    audio: MeetingAudio,
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
        expires_at_ms: i64::MAX,
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
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
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
    _now_ms: i64,
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

/// Closes a cut recording that will not resume. Its end is the end of what
/// was recorded, never before its start nor after now; the closure says the
/// cut's cause and how many fragments it keeps.
fn decide_cut_recording_closed(
    next: &mut State,
    ended_at_ms: i64,
    cause: &str,
    now_ms: i64,
) -> Result<(), Refusal> {
    let recording = next
        .durable
        .recording
        .as_mut()
        .filter(|recording| {
            recording.phase == CapturePhase::Interrupted && recording.ended_at_ms.is_none()
        })
        .ok_or(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::CorrectCaptureState,
        })?;
    recording.phase = CapturePhase::Idle;
    recording.ended_at_ms =
        Some(ended_at_ms.clamp(recording.started_at_ms, now_ms.max(recording.started_at_ms)));
    recording.finalizing_since_ms = None;
    let fragments = recording.sealed_chunks;
    next.set_notice(
        Notice::new(NoticeCode::RecordingClosedAfterCut)
            .with_token(NoticeKey::Cause, cause)
            .with(NoticeKey::Fragments, fragments),
    );
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
    if state.durable.pending_voice.len() >= MAX_PENDING_VOICE_NOTES {
        return Err(Refusal::TooMany {
            resource: crate::refusal::Resource::Message,
            count: state.durable.pending_voice.len() as u64,
            max: MAX_PENDING_VOICE_NOTES as u64,
        });
    }
    next.durable.voice_active = true;
    next.views.short_voice_note = None;
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
        Event::CallStart {
            id,
            duration_ms,
            origin,
        } => {
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
            // Mirrors the one call ceiling, a recording's (it was an hour).
            let _ = duration_ms;
            next.durable.call = Some(crate::voice::CallState {
                id,
                phase: crate::voice::CallPhase::Starting,
                started_at_ms: now_ms,
                expires_at_ms: i64::MAX,
                turns: 0,
                origin,
                ceiling_notice_given: false,
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

fn decide_tick(state: &State, next: &mut State, _effects: &mut Vec<Effect>, now_ms: i64) {
    discard::drain_timed_out(next, now_ms);
    let _ = state;
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
/// The helper is asked to stop capturing without latching. Nothing is
/// resumed from here: the daemon decides that once the cut's audio settled. `closing` says Orbit itself is going away, so the
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
