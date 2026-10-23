//! The clock's bounds on a recording: the safety ceiling, said once before
//! it is reached, and the wait for the Mac to confirm a close.
use super::{CaptureState, interrupt};
use crate::FINALIZING_TIMEOUT_MS;
use crate::model::CapturePhase;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::transition::{Effect, Step};

/// Every tick: the ceiling near, the ceiling reached, the close that was
/// never confirmed.
pub(in crate::transition) fn tick_ceiling(capture: &mut CaptureState, cx: &mut Step) {
    ceiling_near(capture, cx);
    ceiling_reached(capture, cx);
    close_unconfirmed(capture, cx);
}

/// Said once, before the ceiling, with the minutes left: room to finish
/// the recording and start another.
fn ceiling_near(capture: &mut CaptureState, cx: &mut Step) {
    let now_ms = cx.now_ms;
    let Some(recording) = capture.recording.as_mut() else {
        return;
    };
    if recording.ceiling_notice_given
        || !matches!(
            recording.phase,
            CapturePhase::Recording | CapturePhase::Starting
        )
        || now_ms >= recording.expires_at_ms
        || recording.expires_at_ms.saturating_sub(now_ms)
            > crate::RECORDING_CEILING_NOTICE_MINUTES * 60_000
    {
        return;
    }
    recording.ceiling_notice_given = true;
    let remaining_ms = recording.expires_at_ms.saturating_sub(now_ms);
    cx.say(
        Notice::new(NoticeCode::RecordingCeilingNear)
            .with(NoticeKey::Remaining, (remaining_ms + 59_999) / 60_000)
            .with(
                NoticeKey::Minutes,
                now_ms.saturating_sub(recording.started_at_ms).max(0) / 60_000,
            )
            .with(
                NoticeKey::Max,
                recording
                    .expires_at_ms
                    .saturating_sub(recording.started_at_ms)
                    .max(0)
                    / 60_000,
            ),
    );
}

fn ceiling_reached(capture: &mut CaptureState, cx: &mut Step) {
    let reached = capture.recording.as_ref().is_some_and(|r| {
        r.expires_at_ms <= cx.now_ms
            && matches!(r.phase, CapturePhase::Recording | CapturePhase::Starting)
    });
    if !reached {
        return;
    }
    interrupt(capture);
    cx.effect(Effect::StopCapture);
    let has_fragments = capture
        .recording
        .as_ref()
        .is_some_and(|recording| recording.sealed_chunks > 0);
    // The safety ceiling, in whole minutes: there is no permission to
    // expire.
    let minutes = capture.recording.as_ref().map_or(0, |recording| {
        recording
            .expires_at_ms
            .saturating_sub(recording.started_at_ms)
            .max(0)
            / 60_000
    });
    cx.say(
        Notice::new(NoticeCode::RecordingCeilingReached)
            .with(NoticeKey::Minutes, minutes)
            .with(NoticeKey::Fragments, has_fragments),
    );
}

/// Bound the wait for the Mac's drain ACK. If the confirmation does
/// not arrive, the recording falls back to `Interrupted` with a registered
/// gap so the next tick or a later review can detect it.
fn close_unconfirmed(capture: &mut CaptureState, cx: &mut Step) {
    if let Some(recording) = capture.recording.as_mut()
        && recording.phase == CapturePhase::Finalizing
        && let Some(since) = recording.finalizing_since_ms
        && cx.now_ms > since.saturating_add(FINALIZING_TIMEOUT_MS)
    {
        recording.phase = CapturePhase::Interrupted;
        recording.gaps = recording.gaps.saturating_add(1);
        recording.finalizing_since_ms = None;
        let has_fragments = recording.sealed_chunks > 0;
        cx.say(
            Notice::new(NoticeCode::RecordingCloseUnconfirmed)
                .with(NoticeKey::Fragments, has_fragments),
        );
    }
}
