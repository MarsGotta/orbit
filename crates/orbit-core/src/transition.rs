//! Pure decisions, one domain each. Every domain has its module here, with
//! the part of [`DurableState`] it owns and the decisions on it:
//!
//! - `capture`: the recording and its capture;
//! - `assistant`: the conversation, its turn and the voice notes it keeps;
//! - `voice_call`: the microphone for a voice note or a call;
//! - `privacy`: the privacy stop and the generation of authority;
//! - `hub_link`: the link to the hub;
//! - `helper`: the native helper and the permissions it observed.
//!
//! [`decide`] only routes an event to its domain. A domain changes only its
//! own part; what it needs of another it reads, through a small accessor, in
//! the state as it was before the event. An event every domain hears (a
//! privacy stop, a revocation, a cut, Orbit closing, the clock) is routed by
//! `stops` to each domain in turn, each changing its own part.
use crate::authority::AuthorityChange;
use crate::failure::DegradedKind;
use crate::model::*;
use crate::notice::{Notice, NoticeKey};
use crate::protocol::MeetingAudio;
use crate::refusal::Refusal;

mod assistant;
mod capture;
mod helper;
mod hub_link;
mod privacy;
mod stops;
mod voice_call;

pub use assistant::AssistantState;
pub use capture::CaptureState;
pub use hub_link::HubLinkState;
pub use privacy::PrivacyState;
pub use voice_call::VoiceState;

#[derive(Debug, Clone)]
pub enum Event {
    Connect(Connection),
    /// The local link is starting; no authority has changed yet.
    Connecting,
    Submit {
        id: String,
        text: String,
        /// The files the hub already holds for this turn.
        attachments: Vec<crate::attachments::Attachment>,
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
    /// The user asked again for the transcription of the open turn's voice
    /// note `id`, which failed («Reintentar transcripción»).
    VoiceRetry {
        id: String,
    },
    /// The user discards a voice note that failed («Descartar»): the open
    /// turn's, or one waiting on this Mac. One still being transcribed is
    /// refused; the daemon deletes its audio once this is decided.
    VoiceDiscard {
        id: String,
    },
    /// Nothing was heard in the voice note `id`: the hub answered its audio
    /// with no words, or what the user said next carried it. It leaves the
    /// conversation without a trace: it was noise, or part of the next turn.
    VoiceUnheard {
        id: String,
    },
    /// A finished voice note was too short to send; the daemon measured it.
    VoiceTooShort(crate::voice::ShortVoiceNote),
    /// The hub named the conversation (`chat.session`). `agent_id` is the
    /// agent its turn went to (absent: a hub that lists no agents); a new
    /// conversation keeps it, so a later change of the hub's default does
    /// not move it to another agent.
    Session {
        id: String,
        agent_id: Option<String>,
    },
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
    /// A question the agent asks in the turn `turn_id`, the `chat.send` id
    /// its frame arrived under. `approval.request_id` names the request (the
    /// backend's own id, or one the hub made unique) and is never the turn's.
    Approval {
        turn_id: String,
        approval: Approval,
    },
    Interrupt,
    /// A recording starts. It runs until finished, stopped or cut, or at
    /// most [`crate::RECORDING_CEILING_MINUTES`]: nothing that starts it
    /// sets when it stops.
    StartMeeting {
        id: String,
        title: String,
        audio: MeetingAudio,
    },
    CaptureStarted,
    CaptureFinalized,
    EmptyCaptureClosed,
    /// A recording a cut left interrupted, with audio, that will not resume
    /// (the cause was the Mac's sleep, the person's stop, a refused restart,
    /// or the capture did not come back): it is closed at `ended_at_ms`, the
    /// end of what was recorded, so the hub writes its note. `cause` is the
    /// cut's code, said with the closure.
    CutRecordingClosed {
        ended_at_ms: i64,
        cause: String,
    },
    PauseMeeting,
    ResumeMeeting,
    FinishMeeting,
    /// «Terminar sin guardar» on the recording `recording_id`, recording or
    /// paused: its capture stops and nothing of it is kept or closed.
    DiscardMeeting {
        recording_id: String,
    },
    ChunkSealed,
    CaptureGap,
    VoiceStart,
    VoiceEnd,
    /// Esc in the composer: the note in progress ends and is not sent.
    VoiceCancel,
    CallStart {
        id: String,
        duration_ms: i64,
        origin: crate::voice::CallOrigin,
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
    /// A failed chat turn, carrying the hub code rather than its message,
    /// and the numbers and closed tokens of its refusal (a size and its
    /// ceiling, a provider's status, an agent's stop reason) as notice
    /// tokens; a value that is not a token is kept only as unrecognized.
    Failure {
        code: String,
        numbers: Vec<(NoticeKey, String)>,
    },
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
    /// kept. Orbit keeps working and nothing latches; the daemon resumes or
    /// closes a cut recording once its audio has settled (`cut_recovery`).
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
        /// The hub's ids of the turn's files.
        attachments: Vec<String>,
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
    /// Finish the voice note in progress without sending it.
    CancelVoice,
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

/// One decision in progress: the state as it was before the event, which
/// every domain may read and none may change, the time, and what the
/// decision says besides the state.
struct Step<'a> {
    before: &'a DurableState,
    now_ms: i64,
    effects: Vec<Effect>,
    /// Said in this order once the decision is made.
    notices: Vec<Notice>,
    authority_change: Option<AuthorityChange>,
}

impl Step<'_> {
    fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }
    fn effects<const N: usize>(&mut self, effects: [Effect; N]) {
        self.effects.extend(effects);
    }
    fn say(&mut self, notice: impl Into<Notice>) {
        self.notices.push(notice.into());
    }
    fn authority(&mut self, change: AuthorityChange) {
        self.authority_change = Some(change);
    }
}

pub fn decide(state: &State, event: Event, now_ms: i64) -> Result<Decision, Refusal> {
    let mut next = state.clone();
    let mut step = Step {
        before: &state.durable,
        now_ms,
        effects: Vec::new(),
        notices: Vec::new(),
        authority_change: None,
    };
    route(&mut next.durable, &mut next.views, event, &mut step)?;
    capture::settle(&mut next.durable.capture);
    for notice in step.notices {
        next.set_notice(notice);
    }
    Ok(Decision {
        state: next,
        effects: step.effects,
        authority_change: step.authority_change,
    })
}

/// Each event to the domain that owns it: `a` is the assistant's part, `c`
/// the capture's, `v` the microphone's (`voice_call`) and `h` the helper's.
fn route(n: &mut DurableState, views: &mut Views, e: Event, cx: &mut Step) -> Result<(), Refusal> {
    let (a, c) = (&mut n.assistant, &mut n.capture);
    let (v, h) = (&mut n.voice, &mut n.capabilities);
    match e {
        Event::Connecting => hub_link::connecting(&mut n.link, cx)?,
        Event::Connect(connection) => stops::connect(n, connection, cx),
        Event::HubTransportReady => cx.authority(AuthorityChange::HubTransportReady),
        Event::HubDisconnected => stops::hub_disconnected(n, cx),
        Event::HelperReady => helper::ready(h),
        Event::HelperLost => helper::lost(h),
        Event::PermissionObserved {
            microphone,
            screen,
            accessibility,
            calendar,
        } => helper::observed(h, [microphone, screen, accessibility], calendar),
        Event::CalendarAuthorization(calendar) => helper::calendar(h, calendar),
        Event::HelperStopUndelivered => helper::stop_undelivered(h),
        Event::Session { id, agent_id } => assistant::session(a, id, agent_id, cx),
        Event::NewSession { agent_id } => assistant::new_session(a, agent_id, cx),
        Event::OpenSession { id, agent_id } => assistant::open_session(a, id, agent_id, cx),
        Event::DeviceChanged => assistant::device_changed(a, cx),
        Event::Submit {
            id,
            text,
            attachments,
        } => stops::submit(a, &mut views.short_voice_note, id, text, attachments, cx)?,
        Event::Delta { request_id, text } => assistant::delta(a, request_id, &text, cx),
        Event::Completed { request_id, text } => assistant::completed(a, request_id, text, cx),
        Event::Approval { turn_id, approval } => assistant::approval(a, &turn_id, approval, cx),
        Event::Interrupt => assistant::interrupt(a, cx),
        Event::Failure { code, numbers } => assistant::failure(a, &code, numbers, cx),
        Event::SubmitVoice {
            id,
            text,
            duration_ms,
        } => assistant::submit_voice(a, id, text, duration_ms, cx)?,
        Event::VoiceStored { id, duration_ms } => assistant::voice_stored(a, id, duration_ms, cx)?,
        Event::VoiceRetry { id } => assistant::voice_retry(a, &id, cx)?,
        Event::VoiceDiscard { id } => assistant::voice_discard(a, &id)?,
        Event::VoiceUnheard { id } => assistant::forget_voice_note(a, &id),
        Event::VoiceFailed { id, failure } => assistant::voice_failed(a, &id, failure),
        Event::StartMeeting { id, title, audio } => capture::start(c, id, title, audio, cx)?,
        Event::CaptureStarted => capture::started(c, cx)?,
        Event::PauseMeeting => capture::pause(c, cx)?,
        Event::ResumeMeeting => capture::resume(c, cx)?,
        Event::FinishMeeting => capture::finish(c, cx)?,
        Event::DiscardMeeting { recording_id } => capture::discard(c, &recording_id, cx)?,
        Event::CaptureFinalized => capture::finalized(c, cx)?,
        Event::EmptyCaptureClosed => capture::empty_closed(c, cx)?,
        Event::CutRecordingClosed { ended_at_ms, cause } => {
            capture::cut_closed(c, ended_at_ms, &cause, cx)?
        }
        Event::ChunkSealed => capture::chunk_sealed(c),
        Event::CaptureGap => capture::gap(c, cx),
        Event::Degraded(kind) => capture::degraded(c, kind, cx),
        Event::VoiceTooShort(s) => voice_call::too_short(&mut views.short_voice_note, s, cx),
        Event::VoiceStart => voice_call::voice_start(v, &mut views.short_voice_note, cx)?,
        Event::VoiceEnd => voice_call::voice_end(v, cx),
        Event::VoiceCancel => voice_call::voice_cancel(v, cx),
        Event::CallStart {
            id,
            duration_ms,
            origin,
        } => voice_call::call_start(v, id, duration_ms, origin, cx)?,
        Event::CallStop => stops::call_stop(n, cx),
        Event::CallReady => voice_call::call_ready(v, cx),
        Event::CallPhase(phase) => voice_call::call_phase(v, phase),
        Event::CallTurnStarted => voice_call::call_turn_started(v),
        Event::PrivacyStop => stops::privacy_stop(n, cx),
        Event::HelperLatched => stops::helper_latched(n, cx),
        Event::LocalRearm => privacy::rearm(&mut n.privacy, cx)?,
        Event::Tick => stops::tick(n, cx),
        Event::CaptureRefused { reason } => stops::capture_refused(n, &reason, cx),
        Event::CaptureCut { cause } => stops::cut(n, &cause, false, cx),
        Event::Close { cause } => stops::close(n, &cause, cx),
    }
    Ok(())
}

#[cfg(test)]
mod equivalence;
#[cfg(test)]
mod tests;
