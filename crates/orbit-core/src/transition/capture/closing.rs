//! Wait for the Mac to confirm a normal close; no recording duration bound.
use super::CaptureState;
use crate::FINALIZING_TIMEOUT_MS;
use crate::model::CapturePhase;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::transition::Step;
pub(in crate::transition) fn tick_close(capture: &mut CaptureState, cx: &mut Step) {
    close_unconfirmed(capture, cx);
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
