//! The microphone with the assistant: a voice note being recorded, or a
//! call. The meeting owns the microphone; these consume it.
//!
//! Reads of other domains: the microphone guard (`helper`, which reads the
//! privacy stop), the hub link and the helper's voice processing for a
//! call, an open recording (`capture`), and a turn or question in flight or
//! the voice notes waiting (`assistant`).
use super::{Effect, Step, helper};
use crate::model::MAX_PENDING_VOICE_NOTES;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::refusal::{EngineCause, Operation, Refusal, Resource};
use crate::voice::{CallOrigin, CallPhase, CallState, ShortVoiceNote};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceState {
    /// A voice note is being recorded.
    pub voice_active: bool,
    pub call: Option<CallState>,
}

impl VoiceState {
    pub(super) fn in_call(&self) -> bool {
        self.call.is_some()
    }

    /// A voice note is being recorded.
    pub(super) fn recording_voice_note(&self) -> bool {
        self.voice_active
    }

    /// The call ends here, if there was one.
    pub(super) fn end_call(&mut self) -> bool {
        self.call.take().is_some()
    }

    /// The voice note being recorded is dropped, if there was one.
    pub(super) fn drop_voice_note(&mut self) -> bool {
        std::mem::take(&mut self.voice_active)
    }

    /// Revoke live call authority, not its durable original's lifecycle.
    /// The daemon reconciles voice_call manifests separately before capture
    /// or network work can resume; this pure model never performs recovery I/O.
    pub(crate) fn recovered(&mut self) {
        self.voice_active = false;
        self.call = None;
    }
}

pub(super) fn voice_start(
    voice: &mut VoiceState,
    short_note: &mut Option<ShortVoiceNote>,
    cx: &mut Step,
) -> Result<(), Refusal> {
    if voice.in_call() {
        return Err(busy(Operation::AudioSession));
    }
    helper::ensure_microphone(cx.before)?;
    let assistant = &cx.before.assistant;
    if voice.voice_active || assistant.turn_in_flight() || assistant.question_pending() {
        return Err(busy(Operation::Interaction));
    }
    let waiting = assistant.waiting_voice_notes();
    if waiting >= MAX_PENDING_VOICE_NOTES {
        return Err(Refusal::TooMany {
            resource: Resource::Message,
            count: waiting as u64,
            max: MAX_PENDING_VOICE_NOTES as u64,
        });
    }
    voice.voice_active = true;
    *short_note = None;
    cx.effects([Effect::StopPlayback, Effect::StartVoice]);
    Ok(())
}

pub(super) fn voice_end(voice: &mut VoiceState, cx: &mut Step) {
    if voice.drop_voice_note() {
        cx.effect(Effect::FinishVoice);
    }
}

/// Esc in the composer: the note in progress ends and is not sent.
pub(super) fn voice_cancel(voice: &mut VoiceState, cx: &mut Step) {
    if voice.drop_voice_note() {
        cx.effect(Effect::CancelVoice);
    }
}

/// A finished voice note was too short to send; the daemon measured it.
pub(super) fn too_short(
    short_note: &mut Option<ShortVoiceNote>,
    short: ShortVoiceNote,
    cx: &mut Step,
) {
    cx.say(
        Notice::new(NoticeCode::VoiceNoteTooShort)
            .with(NoticeKey::DurationMs, short.duration_ms)
            .with(NoticeKey::MinimumMs, short.minimum_ms),
    );
    *short_note = Some(short);
}

/// The short note is forgotten once a message is written instead.
pub(super) fn forget_short_note(short_note: &mut Option<ShortVoiceNote>) {
    *short_note = None;
}

pub(super) fn call_start(
    voice: &mut VoiceState,
    id: String,
    duration_ms: i64,
    origin: CallOrigin,
    cx: &mut Step,
) -> Result<(), Refusal> {
    let before = cx.before;
    helper::ensure_microphone(before)?;
    before.link.ensure_online()?;
    if !before.capabilities.voice_processing_supported {
        return Err(Refusal::EngineUnavailable {
            cause: EngineCause::VoiceProcessingUnsupported,
        });
    }
    if let Some(recording) = before.capture.open_recording() {
        return Err(Refusal::PendingRecording {
            id: recording.id.clone(),
            title: recording.title.clone(),
            phase: recording.phase.clone(),
            sealed_chunks: recording.sealed_chunks,
            gaps: recording.gaps,
            expires_at_ms: recording.expires_at_ms,
        });
    }
    if voice.in_call() || voice.voice_active || before.assistant.turn_in_flight() {
        return Err(busy(Operation::AudioSession));
    }
    let _ = duration_ms; // Retired wire parameter; calls have no duration bound.
    voice.call = Some(CallState {
        id,
        phase: CallPhase::Starting,
        started_at_ms: cx.now_ms,
        expires_at_ms: i64::MAX,
        turns: 0,
        origin,
        ceiling_notice_given: false,
    });
    cx.effects([Effect::StopPlayback, Effect::StartCall]);
    cx.say(NoticeCode::CallStarting);
    Ok(())
}

pub(super) fn call_ready(voice: &mut VoiceState, cx: &mut Step) {
    if let Some(call) = voice.call.as_mut() {
        call.phase = CallPhase::Listening;
        cx.say(NoticeCode::CallActive);
    }
}

pub(super) fn call_phase(voice: &mut VoiceState, phase: CallPhase) {
    if let Some(call) = voice.call.as_mut() {
        call.phase = phase;
    }
}

pub(super) fn call_turn_started(voice: &mut VoiceState) {
    if let Some(call) = voice.call.as_mut() {
        call.phase = CallPhase::UserSpeaking;
        call.turns = call.turns.saturating_add(1);
    }
}

fn busy(operation: Operation) -> Refusal {
    Refusal::Busy {
        operation,
        retry_after_ms: None,
    }
}
