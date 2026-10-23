//! What the capture task does with one frame while the lane is armed: the
//! same steps the loop takes for a recording frame, under the lane's lock,
//! with nothing that waits. Anything it cannot finish the way the loop would
//! goes to the loop instead, and the lane disarms until the loop has served
//! it, so the loop sees every frame after it in order.
use super::{Forwarded, ForwardedItem, LaneCore};
use crate::capture;
use crate::daemon::store_actor::CapturePort;
use orbit_core::wire::native::{NativeAudioIdentityRef, NativeEventHeader};
use serde_json::Value;
use std::{path::Path, time::Instant};

/// Signal observations the lane keeps for a loop that is not reading them:
/// a held loop must not make the lane's memory grow with the meeting. What
/// it protects: memory. Why this size: about 40 s of two tracks at the
/// ~47 frames per second each, longer than any hold the loop is expected to
/// survive; past it the oldest observations go, and the track's level, which
/// only says whether it hears something now, keeps the newest.
pub(super) const MAX_PENDING_SIGNALS: usize = 4096;

pub(super) fn forward(core: &mut LaneCore, seq: u64, item: ForwardedItem) {
    let after_control = core.last_control;
    core.forwarded.push_back(Forwarded {
        seq,
        after_control,
        item,
    });
    core.gate.armed = false;
}

/// Whether the armed lane may serve `frame` itself: an `audio` frame of a
/// stream the loop armed it with, for a source that feeds the recording.
fn servable(core: &LaneCore, frame: &Value) -> bool {
    if !core.gate.armed
        || !NativeEventHeader::from_value(frame).is_ok_and(|header| header.event == "audio")
    {
        return false;
    }
    let Ok(identity) = NativeAudioIdentityRef::from_value(frame) else {
        return false;
    };
    let armed = match identity.source {
        "mic_local" => core.gate.microphone.as_deref(),
        "remote_app" => core.gate.remote.as_deref(),
        _ => None,
    };
    armed == Some(identity.stream_id) && core.tracks.records(identity.source)
}

/// One frame from the helper, at its place `seq` in the helper's order.
/// Returns whether a seal receipt arrived meanwhile.
pub(super) fn serve_frame(
    core: &mut LaneCore,
    port: &CapturePort,
    store: &Path,
    seq: u64,
    mut frame: Value,
) -> bool {
    if !servable(core, &frame) {
        forward(core, seq, ForwardedItem::Event(frame));
        return false;
    }
    // A producer's lifetime is its epoch, as the loop labels it.
    let stream = NativeAudioIdentityRef::from_value(&frame)
        .map(|identity| identity.stream_id.to_owned())
        .unwrap_or_default();
    if let Some(object) = frame.as_object_mut() {
        object.insert("capture_epoch_id".into(), Value::String(stream));
    }
    let received_at_ms = crate::daemon::now();
    let ingested = match capture::ingest(&mut core.tracks, &mut None, &frame, received_at_ms, true)
    {
        Ok(ingested) => ingested,
        Err(error) => {
            let item = ForwardedItem::FrameFailed {
                frame,
                error: error.to_string(),
            };
            forward(core, seq, item);
            return false;
        }
    };
    if let Some(signal) = ingested.signal {
        if core.outbox.signals.len() >= MAX_PENDING_SIGNALS {
            core.outbox.signals.remove(0);
        }
        core.outbox.signals.push((signal, Instant::now()));
    }
    seal(core, port, store, seq, ingested.segments)
}

/// The frame's segments admitted and handed to the writer without waiting,
/// and any receipt already there taken for the loop.
fn seal(
    core: &mut LaneCore,
    port: &CapturePort,
    store: &Path,
    seq: u64,
    segments: Vec<capture::Segment>,
) -> bool {
    if !segments.is_empty() {
        if core.would_overflow(&segments) {
            forward(core, seq, ForwardedItem::Segments { segments });
            return false;
        }
        let overflow = core.admit(segments);
        if !overflow.is_empty() {
            forward(core, seq, ForwardedItem::Segments { segments: overflow });
            return false;
        }
        if let Err(error) = core
            .validate_destinations(store)
            .and_then(|()| core.dispatch(port))
        {
            let error = error.to_string();
            forward(core, seq, ForwardedItem::SealRefused { error });
            return false;
        }
    }
    core.collect(port)
}

/// The task's own clock: receipts read, and seals the writer still owes
/// handed again (one already admitted is not admitted twice).
pub(super) fn upkeep(core: &mut LaneCore, port: &CapturePort) -> bool {
    if core.gate.armed && !core.seals.is_empty() {
        let _ = core.dispatch(port);
    }
    core.collect(port)
}
