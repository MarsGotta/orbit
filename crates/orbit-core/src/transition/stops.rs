//! What every domain hears: a change of the hub link, a revocation, the
//! user's privacy stop, a call that ends, a cut, Orbit closing and the
//! clock. Each is routed to every domain it concerns, in a fixed order, and
//! each domain changes only its own part; this module owns no state.
use super::{AssistantState, Effect, Step, assistant, capture, voice_call};
use crate::attachments::Attachment;
use crate::authority::AuthorityChange;
use crate::model::{Connection, DurableState};
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::privacy_stop::StopRequester;
use crate::refusal::Refusal;
use crate::voice::ShortVoiceNote;

/// A written turn is sent: the short voice note said before it is
/// forgotten.
pub(super) fn submit(
    assistant: &mut AssistantState,
    short_note: &mut Option<ShortVoiceNote>,
    id: String,
    text: String,
    attachments: Vec<Attachment>,
    cx: &mut Step,
) -> Result<(), Refusal> {
    assistant::submit(assistant, id, text, attachments, cx)?;
    voice_call::forget_short_note(short_note);
    Ok(())
}

pub(super) fn connect(n: &mut DurableState, connection: Connection, cx: &mut Step) {
    if let Some(change) = cx.before.link.authority_for(&connection) {
        cx.authority(change);
    }
    link_changed(n, connection, cx);
}

pub(super) fn hub_disconnected(n: &mut DurableState, cx: &mut Step) {
    cx.authority(AuthorityChange::HubDisconnected);
    assistant::hub_lost(&mut n.assistant, cx);
    link_changed(n, Connection::Offline, cx);
}

/// A call needs the hub; a revocation stops everything, latched as the
/// hub's.
fn link_changed(n: &mut DurableState, connection: Connection, cx: &mut Step) {
    if connection != Connection::Online && n.voice.end_call() {
        n.assistant.retire_request();
        cx.effects([Effect::StopCall, Effect::StopPlayback, Effect::Interrupt]);
        cx.say(NoticeCode::CallStoppedByConnection);
    }
    if connection == Connection::Revoked {
        n.privacy.revoke();
        n.voice.drop_voice_note();
        n.assistant.retire_request();
        capture::interrupt(&mut n.capture);
        cx.effects([
            Effect::StopCapture,
            Effect::StopPlayback,
            Effect::Interrupt,
            Effect::CancelNetwork,
        ]);
        cx.say(NoticeCode::DeviceRevoked);
    }
    n.link.set(connection);
}

/// The user asked for their privacy stop: the only event that latches as
/// theirs.
pub(super) fn privacy_stop(n: &mut DurableState, cx: &mut Step) {
    if cx.before.privacy.stopped() {
        n.privacy.pressed_again();
    } else {
        cx.authority(AuthorityChange::UserStop);
        stop_everything(n, Some(StopRequester::User), cx);
    }
}

/// The helper held a privacy stop on disk: adopted so the daemon and the
/// helper say the same; who pressed it is unknown, and the stop is kept.
pub(super) fn helper_latched(n: &mut DurableState, cx: &mut Step) {
    if cx.before.privacy.stopped() {
        n.privacy.helper_latch_kept();
    } else {
        cx.authority(AuthorityChange::UserStop);
        stop_everything(n, None, cx);
    }
}

fn stop_everything(n: &mut DurableState, requester: Option<StopRequester>, cx: &mut Step) {
    if n.voice.end_call() {
        cx.effect(Effect::StopCall);
    }
    n.privacy.latch(requester);
    n.link.go_offline();
    n.voice.drop_voice_note();
    n.assistant.retire_request();
    capture::interrupt(&mut n.capture);
    cx.effects([
        Effect::StopCapture,
        Effect::StopPlayback,
        Effect::Interrupt,
        Effect::CancelNetwork,
    ]);
    cx.say(NoticeCode::PrivacyStopped);
}

/// The user hangs up: the call's turn ends with it.
pub(super) fn call_stop(n: &mut DurableState, cx: &mut Step) {
    if n.voice.end_call() {
        n.assistant.retire_request();
        cx.effects([Effect::StopPlayback, Effect::Interrupt, Effect::StopCall]);
        cx.say(NoticeCode::CallStopped);
    }
}

/// The clock bounds drains and unconfirmed closes, never recording duration.
pub(super) fn tick(n: &mut DurableState, cx: &mut Step) {
    capture::drain_timed_out(&mut n.capture, cx);
    capture::tick_close(&mut n.capture, cx);
}

/// The helper refused an `audio_start` for a non-stop reason: the capture
/// is interrupted and the cause said; nothing latches.
pub(super) fn capture_refused(n: &mut DurableState, reason: &str, cx: &mut Step) {
    let voice_only = cx.before.voice.recording_voice_note() && !cx.before.capture.capturing();
    capture::interrupt(&mut n.capture);
    n.voice.drop_voice_note();
    let otherwise = if voice_only {
        NoticeCode::VoiceNoteRefused
    } else {
        NoticeCode::CaptureRefused
    };
    cx.say(capture_refused_notice(reason, otherwise));
}

/// Cuts whatever is live: the recording is interrupted with its
/// fragments counted, a call ends with its turn, a voice note is dropped.
/// The helper is asked to stop capturing without latching. Nothing is
/// resumed from here: the daemon decides that once the cut's audio
/// settled. `closing` says Orbit itself is going away, so the notice does
/// not promise it keeps working.
pub(super) fn cut(n: &mut DurableState, cause: &str, closing: bool, cx: &mut Step) {
    // The cause is a code (a helper code or a stop cause's name); anything
    // else is kept only as `unrecognized`, never as a sentence.
    let code = if closing {
        NoticeCode::OrbitClosing
    } else {
        NoticeCode::CaptureCut
    };
    let mut notice = Notice::new(code).with_token(NoticeKey::Cause, cause);
    if let Some(fragments) = capture::interrupt(&mut n.capture) {
        notice = notice
            .with(NoticeKey::Recording, true)
            .with(NoticeKey::Fragments, fragments);
    }
    if n.voice.end_call() {
        n.assistant.retire_request();
        cx.effect(Effect::StopCall);
        if !closing {
            cx.effect(Effect::Interrupt);
        }
        notice = notice.with(NoticeKey::Call, true);
    }
    if n.voice.drop_voice_note() {
        notice = notice.with(NoticeKey::VoiceNote, true);
    }
    cx.effects([Effect::CutCapture, Effect::StopPlayback]);
    cx.say(notice);
}

/// Orbit is closing (shutdown, IPC listener lost): the capture is cut, the
/// turn and network work are retired, and nothing latches, so the next
/// start comes back working.
pub(super) fn close(n: &mut DurableState, cause: &str, cx: &mut Step) {
    cx.authority(AuthorityChange::Closing);
    cut(n, cause, true, cx);
    n.link.go_offline();
    n.privacy.advance();
    n.assistant.retire_request();
    cx.effects([Effect::Interrupt, Effect::CancelNetwork]);
}

/// Spanish notice for a refused audio start. A missing permission is
/// something the user can grant; any other refusal is said with the
/// helper's code.
fn capture_refused_notice(reason: &str, otherwise: NoticeCode) -> Notice {
    match reason {
        "microphone_permission_required" => Notice::new(NoticeCode::MicrophonePermissionMissing),
        "screen_permission_required" => Notice::new(NoticeCode::ScreenPermissionMissing),
        _ => Notice::new(otherwise).with_token(NoticeKey::Cause, reason),
    }
}

#[path = "stops/silence.rs"]
mod silence;
pub(super) use silence::{retire_questions, silence_event};
