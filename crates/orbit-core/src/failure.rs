//! The three classes every native failure lands in before anything stops.
//!
//! A rejection is shown and changes nothing; a degradation is recorded and
//! the capture keeps going; an integrity failure cuts the live capture with
//! its cause. None of them latches: only the user's own stop does, and no
//! helper answer can stand in for the user. This module is the pure table
//! behind that split.

use crate::model::{CapturePhase, State};

/// What kind of degradation this is, when a recording keeps going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedKind {
    /// A discontinuity in the captured audio source.
    AudioGap,
    /// A problem in the playback path that did not stop the recording.
    PlaybackFailed,
    /// The helper refused the request for a non-permission, non-stop reason.
    HelperRejected,
    /// The helper asked the device to slow down (queue, transport, calendar).
    Backpressure,
}

/// The class of one native response.
///
/// `Refused` is shown to the person and does not change the capture.
/// `Degraded` is recorded and adds a notice while the capture keeps going.
/// `IntegrityFailure` cuts the live capture. None of them latches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Shown, capture untouched.
    Refused { reason: String },
    /// Recorded, additive notice, capture keeps going.
    Degraded { kind: DegradedKind, detail: String },
    /// The audio at risk cannot be kept whole: the live capture is cut with
    /// this detail as its cause, and Orbit keeps working.
    IntegrityFailure { detail: String },
}

/// Classify a helper response to a command by command kind, error code and
/// state.
///
/// Each helper code the helper can emit has an explicit class in the table;
/// a code nobody emits is `Degraded { HelperRejected }` unless the command is
/// a capture command, where it is `IntegrityFailure`. A few codes also
/// refine their class against the recording state: `audio_drain_incomplete`
/// only stops capture when there is a live recording whose audio is at risk.
pub fn classify_native_response(command_kind: &str, error_code: &str, state: &State) -> Outcome {
    match table_lookup(error_code) {
        Some(CodeClass::Refused) => Outcome::Refused {
            reason: error_code.to_owned(),
        },
        Some(CodeClass::Degraded(kind)) => Outcome::Degraded {
            kind,
            detail: error_code.to_owned(),
        },
        Some(CodeClass::IntegrityFailure) => Outcome::IntegrityFailure {
            detail: error_code.to_owned(),
        },
        Some(CodeClass::IntegrityFailureIfLive) => {
            if has_live_recording(state) {
                Outcome::IntegrityFailure {
                    detail: error_code.to_owned(),
                }
            } else {
                Outcome::Degraded {
                    kind: DegradedKind::PlaybackFailed,
                    detail: error_code.to_owned(),
                }
            }
        }
        None => {
            if is_capture_command(command_kind) {
                Outcome::IntegrityFailure {
                    detail: error_code.to_owned(),
                }
            } else {
                Outcome::Degraded {
                    kind: DegradedKind::HelperRejected,
                    detail: error_code.to_owned(),
                }
            }
        }
    }
}

/// A gap in the audio source is declared; the recording continues.
///
/// Gaps used to escalate to a privacy stop. The product decision changed: a
/// hole is recorded with an additive notice, never used to fail closed a live
/// recording, and `escalates_to_privacy_stop` is retired.
pub fn classify_gap(_state: &State) -> Outcome {
    Outcome::Degraded {
        kind: DegradedKind::AudioGap,
        detail: "audio_gap".to_owned(),
    }
}

fn has_live_recording(state: &State) -> bool {
    state
        .durable
        .capture
        .recording
        .as_ref()
        .is_some_and(|recording| {
            matches!(
                recording.phase,
                CapturePhase::Starting | CapturePhase::Recording | CapturePhase::Finalizing
            )
        })
}

/// Orders that start, stop or drain a capture source. The daemon also uses
/// this list to tell a failure of the capture itself from another vertical's:
/// only the first may cut a recording.
pub fn is_capture_command(command_kind: &str) -> bool {
    matches!(
        command_kind,
        "audio_start"
            | "audio_stop"
            | "capture_stop"
            | "audio_pause"
            | "audio_resume"
            | "audio_drain"
            | "stream_start"
            | "stream_stop"
            | "capture"
            | "system_audio_start"
            | "system_audio_stop"
    )
}

/// Internal class a code maps to before any state-based refinement.
///
/// `IntegrityFailureIfLive` collapses into `IntegrityFailure` when there is
/// a live recording; otherwise it falls back to a degraded playback notice
/// because no audio is at risk of being lost.
#[derive(Debug, Clone, Copy)]
enum CodeClass {
    Refused,
    Degraded(DegradedKind),
    IntegrityFailure,
    IntegrityFailureIfLive,
}

fn table_lookup(code: &str) -> Option<CodeClass> {
    TABLE
        .iter()
        .find_map(|(listed, class)| if *listed == code { Some(*class) } else { None })
}

/// The full table of codes the helper can emit, grouped by the Swift file the
/// code is defined in.
///
/// Each entry has one explicit class; `audio_drain_incomplete` and
/// `application_binding_changed` carry the "only if live" refinement so the
/// classifier can downgrade them when no audio is at risk.
const TABLE: &[(&str, CodeClass)] = &[
    // AudioCapture.swift: NativeFailure throws and the "error" event.
    ("audio_memory_unavailable", CodeClass::IntegrityFailure),
    ("microphone_already_active", CodeClass::Refused),
    ("microphone_permission_required", CodeClass::Refused),
    (
        "unsupported_microphone_format",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "voice_processing_reference_unavailable",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "microphone_processing_changed_during_start",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    ("system_audio_already_active", CodeClass::Refused),
    (
        "application_binding_changed",
        CodeClass::IntegrityFailureIfLive,
    ),
    ("screen_permission_required", CodeClass::Refused),
    ("operation_cancelled_by_stop", CodeClass::Refused),
    (
        "application_not_available",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "cannot_capture_orbit_audio",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "system_audio_clock_unavailable",
        CodeClass::IntegrityFailure,
    ),
    (
        "system_audio_stop_failed",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    // AudioPlayback.swift and CompressedAudioDecoder.swift: NativeFailure throws.
    (
        "empty_audio",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "invalid_audio",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "playback_session_exhausted",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "stale_or_out_of_order_playback",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "playback_format_changed",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "playback_converter_unavailable",
        CodeClass::Degraded(DegradedKind::PlaybackFailed),
    ),
    (
        "playback_backpressure",
        CodeClass::Degraded(DegradedKind::Backpressure),
    ),
    // OrbitApplication.swift: channel.reject codes, NativeFailure throws
    // and "error" events.
    (
        "isolated_fixture_scope",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    ("audio_drain_incomplete", CodeClass::IntegrityFailureIfLive),
    ("local_rearm_unavailable", CodeClass::Refused),
    // The user's rearm could not remove the latch: their stop stays in
    // force, which is said; nothing new stops.
    ("privacy_rearm_failed", CodeClass::Refused),
    (
        "daemon_disconnected",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "native_backpressure",
        CodeClass::Degraded(DegradedKind::Backpressure),
    ),
    // NativeChannel.swift: the main thread held a full backlog and this one
    // order never ran. Refused, never a cut: what already runs is untouched.
    ("native_main_thread_busy", CodeClass::Refused),
    // The helper refuses with this while its latch is set and also while a
    // capture stop is still cleaning up. Only its `state` says which, so a
    // refusal is shown and never taken for the user's stop.
    ("privacy_stopped", CodeClass::Refused),
    (
        "native_operation_failed",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "invalid_stream_id",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "invalid_application_scope",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    ("audio_session_busy", CodeClass::Refused),
    (
        "processing_not_supported_for_source",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "application_pid_required",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "stream_not_found",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    ("capture_clock_unavailable", CodeClass::IntegrityFailure),
    (
        "unsupported_command",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "calendar_busy",
        CodeClass::Degraded(DegradedKind::Backpressure),
    ),
    (
        "calendar_read_failed",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "hotkey_registration_failed",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    // The helper could not make the user's latch durable; the daemon's own
    // record still holds their stop, and the failure is said.
    ("privacy_stop_persistence_failed", CodeClass::Refused),
    // NativeChannel.swift: admission and transport failure reasons.
    (
        "invalid_protocol_or_disconnected",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
    (
        "native_command_backpressure",
        CodeClass::Degraded(DegradedKind::Backpressure),
    ),
    (
        "daemon_output_disconnected",
        CodeClass::Degraded(DegradedKind::HelperRejected),
    ),
];

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
