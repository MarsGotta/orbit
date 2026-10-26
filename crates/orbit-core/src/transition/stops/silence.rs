//! Coordinator for the two existing lifecycle domains; no audio or clocks.
use super::super::{Effect, Step, capture};
use crate::audio_signal::RecordingSignalView;
use crate::notice::{Notice, NoticeCode, Op};
use crate::refusal::Refusal;
use crate::silence::*;
use crate::{CapturePhase, DurableState, Views};

fn say(cx: &mut Step, code: NoticeCode, op: &str) {
    let mut notice = Notice::new(code);
    notice.op = Op::parse(op);
    cx.say(notice);
}
pub(in crate::transition) fn keep_call(views: &mut Views, cx: &mut Step) {
    views.silence_questions.call.keep(cx.now_ms);
    say(cx, NoticeCode::CallSilenceResolved, "call-silence");
}
pub(in crate::transition) fn keep_recording(views: &mut Views, cx: &mut Step) {
    views.silence_questions.recording.keep(cx.now_ms);
    say(
        cx,
        NoticeCode::RecordingSilenceResolved,
        "recording-silence",
    );
}
pub(in crate::transition) fn silence(
    n: &mut DurableState,
    views: &mut Views,
    call: Option<CallQuiet>,
    recording: Option<&RecordingSignalView>,
    muted: bool,
    cx: &mut Step,
) -> Result<(), Refusal> {
    if let Some(live) = &n.voice.call {
        let q = &mut views.silence_questions.call;
        let quiet = call.is_some_and(|f| !f.busy && f.silent_ms >= CALL_SILENCE_MS)
            && q.reset_allows(cx.now_ms, CALL_SILENCE_MS);
        match q.observe(&live.id, "", quiet, CALL_ANSWER_MS, cx.now_ms) {
            Outcome::Ask => {
                say(cx, NoticeCode::CallSilenceQuestion, "call-silence");
                cx.effect(Effect::AskCallPresence);
            }
            Outcome::Close => {
                keep_call(views, cx);
                super::call_stop(n, cx);
            }
            Outcome::Withdraw => keep_call(views, cx),
            Outcome::None => {}
        }
    } else {
        views.silence_questions.call = Question::default();
    }
    if let Some(live) = &n.capture.recording {
        let q = &mut views.silence_questions.recording;
        let signal = recording.filter(|s| s.recording_id == live.id);
        let quiet = live.phase == CapturePhase::Recording
            && signal.is_some_and(|s| recording_quiet(s, muted))
            && q.reset_allows(cx.now_ms, RECORDING_SILENCE_MS as i64);
        let epoch = signal.map_or("", |s| s.capture_epoch_id.as_str());
        match q.observe(&live.id, epoch, quiet, RECORDING_ANSWER_MS, cx.now_ms) {
            Outcome::Ask => say(
                cx,
                NoticeCode::RecordingSilenceQuestion,
                "recording-silence",
            ),
            Outcome::Close => {
                keep_recording(views, cx);
                capture::finish(&mut n.capture, cx)?;
            }
            Outcome::Withdraw => keep_recording(views, cx),
            Outcome::None => {}
        }
    } else {
        views.silence_questions.recording = Question::default();
    }
    Ok(())
}

pub(in crate::transition) fn silence_event(
    n: &mut DurableState,
    views: &mut Views,
    event: crate::Event,
    cx: &mut Step,
) -> Result<(), Refusal> {
    match event {
        crate::Event::CallKeep => keep_call(views, cx),
        crate::Event::RecordingKeep => keep_recording(views, cx),
        crate::Event::CheckSilence {
            call,
            recording,
            microphone_muted,
        } => silence(n, views, call, recording.as_deref(), microphone_muted, cx)?,
        _ => {}
    }
    Ok(())
}

/// Manual stops, cuts and privacy changes retire their visible question too.
pub(in crate::transition) fn retire_questions(n: &DurableState, views: &mut Views, cx: &mut Step) {
    if n.voice.call.is_none() && views.silence_questions.call.pending() {
        keep_call(views, cx);
    }
    if n.capture
        .recording
        .as_ref()
        .is_none_or(|r| r.phase != CapturePhase::Recording)
        && views.silence_questions.recording.pending()
    {
        keep_recording(views, cx);
    }
}
