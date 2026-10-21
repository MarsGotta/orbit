//! Who may stop Orbit, and what every other stop cause does instead (REF-12).
//!
//! Mars's own stop is the only one that latches and survives a restart: the
//! app's button or tray, ⌃⌥Esc or the helper's own menu. Every
//! other cause that used to stop Orbit by itself now either cuts the live
//! capture, saying why and what was kept, or is said as a notice with its
//! cause; Orbit keeps working and asks for no rearm. This module is the pure
//! table behind that split, without clocks, files or state of its own.
//!
//! The helper always knew why it stopped and wrote the code to its own log;
//! the code travels in the notice's data (SIM-21), so it is the key into
//! `~/Library/Logs/Orbit/native.log`.

use crate::model::State;
use crate::notice::{Notice, NoticeCode, NoticeKey};
use serde::{Deserialize, Serialize};

/// Who asked for the privacy stop in force.
///
/// `None` next to a stop in force means nobody can say: a stop saved by a
/// version older than this field, or one the helper held on disk when it
/// connected. Absence is not a "no", so such a stop is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopRequester {
    /// Mars, with one of her controls: the only stop carried across a
    /// restart as hers.
    Mars,
    /// The hub refused this device's credential. Remote traffic may stop
    /// Orbit and never rearm it; the next connection asks the hub again, so
    /// this stop is not carried across a restart and never writes the
    /// helper's durable latch.
    Revocation,
}

/// Where Mars asked for her stop.
///
/// Every variant is one of her controls or the helper's record of one: an
/// automatic cause has no way to be written here, which is what keeps the
/// latch hers (REF-12 A5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarsStop {
    /// An order on her own local socket: the app's button or tray.
    Order,
    /// ⌃⌥Esc, as the daemon heard it from the helper's `hotkey` event.
    Hotkey,
    /// The helper stopped because she asked it to, and says which control
    /// with one of [`LATCHING_HELPER_CODES`].
    Helper { code: String },
    /// The helper said its durable latch is set: in its `state`, or in a
    /// stop event whose code is not one of hers (`code`). Nobody can say who
    /// pressed it, and it is kept, so both sides say the same.
    HelperLatch { code: Option<String> },
}

impl MarsStop {
    /// The cause this stop is recorded under in the A5 table.
    pub fn cause(&self) -> StopCause {
        match self {
            Self::Order => StopCause::Order,
            Self::Hotkey => StopCause::Hotkey,
            Self::Helper { .. } => StopCause::HelperMarsStop,
            Self::HelperLatch { .. } => StopCause::HelperLatch,
        }
    }

    /// What she is told: her stop, where it came from and, when the helper
    /// gave one, its code. The interface's entry names the way back
    /// (Rearmar).
    pub fn notice(&self) -> Notice {
        let notice = Notice::new(NoticeCode::PrivacyStopped)
            .with_token(NoticeKey::Origin, self.cause().name());
        match self {
            Self::Helper { code } | Self::HelperLatch { code: Some(code) } => {
                notice.with_token(NoticeKey::Code, code)
            }
            Self::Order | Self::Hotkey | Self::HelperLatch { code: None } => notice,
        }
    }
}

/// Helper stop codes that are Mars's own brake: the only ones for which the
/// helper writes its durable latch. Every other code stops capture and
/// playback and leaves the latch as it was.
///
/// Mirrored in Swift by `StopPolicy.latchingReasons`
/// (`native/macos/Sources/OrbitNativeState/StopPolicy.swift`), compared against
/// `contracts/fixtures/native-latching-stops.json` by tests in both languages.
pub const LATCHING_HELPER_CODES: [&str; 2] = ["local_hotkey", "daemon_requested"];

/// Stop codes the helper sends back when the daemon itself asked for the
/// stop: their echo is not a new cause.
pub const DAEMON_ECHO_CODES: [&str; 3] = ["daemon_requested", "daemon_cut", "shutdown"];

/// True when the helper stops with this code only because Mars asked.
pub fn latches(code: &str) -> bool {
    LATCHING_HELPER_CODES.contains(&code)
}

/// Whether the stop in force belongs in the helper's durable latch: hers, or
/// one nobody can attribute. The hub's revocation stops capture in the helper
/// without writing it, because it is not carried across a restart.
pub fn latches_helper(state: &State) -> bool {
    state.durable.privacy_stopped
        && state.durable.privacy_stop_by != Some(StopRequester::Revocation)
}

/// True while something can still be capturing: a recording that has not
/// been interrupted, paused or closed, a call or a push-to-talk note.
pub fn capture_live(state: &State) -> bool {
    state.durable.recording.as_ref().is_some_and(|recording| {
        matches!(
            recording.phase,
            crate::CapturePhase::Starting
                | crate::CapturePhase::Recording
                | crate::CapturePhase::Finalizing
        )
    }) || state.durable.call.is_some()
        || state.durable.voice_active
}

/// What one former automatic stop does now (REF-12 A5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopClass {
    /// Mars's stop: capture, voice and network stop and stay stopped, across
    /// restarts, until she rearms.
    MarsStop,
    /// The live recording, call or voice note is cut, and the notice says
    /// why and how many fragments were kept. Orbit keeps working, asks for no
    /// rearm and does not reopen the microphone on its own. With nothing live
    /// there is nothing to cut and the cause is only said.
    CutCapture,
    /// Nothing live is at risk: the cause is said with its numbers and
    /// nothing stops.
    Notice,
}

impl StopClass {
    /// A stable name for the diagnostic log.
    pub const fn name(self) -> &'static str {
        match self {
            Self::MarsStop => "mars_stop",
            Self::CutCapture => "cut_capture",
            Self::Notice => "notice",
        }
    }
}

/// Every cause that reached the old automatic stop (`fail_closed`), and the
/// feeders that reached it through an error (REF-12 A5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// `Command::PrivacyStop`: the app's button or tray.
    Order,
    /// The helper's `hotkey` event for ⌃⌥Esc.
    Hotkey,
    /// A helper `privacy_stop` event whose code is one of Mars's controls.
    HelperMarsStop,
    /// The helper's `state` said its durable latch is set.
    HelperLatch,
    /// The helper stopped its capture on its own: `capture_stopped`, or a
    /// `privacy_stop` event with any other code (session_inactive,
    /// audio_device_changed, voice_processing_state_lost…).
    HelperCaptureStop,
    /// The helper answered a capture order with an integrity failure
    /// (`audio_drain_incomplete` with audio at risk,
    /// `audio_memory_unavailable`, `capture_clock_unavailable`…).
    HelperIntegrityFailure,
    /// The helper refused an order with `privacy_stopped` or could not
    /// rearm: its latch, or a stop still cleaning up, which only its `state`
    /// tells apart.
    HelperRefusedStopped,
    /// The helper process went away.
    HelperLost,
    /// Handling a helper event about the capture itself failed: audio frames
    /// and gaps, the helper's lifecycle (`ready`, `state`, stops), the answer
    /// to a capture order, or a capture hotkey.
    CaptureEventFailed,
    /// Handling any other helper event failed (calendar, sources, tasks,
    /// meeting detection…): a fault of that vertical, not of the capture.
    NativeEventFailed,
    /// The capture half of the once-a-second maintenance failed: sealing,
    /// the drain of stopped audio, a call's health or its drain.
    CaptureMaintenanceFailed,
    /// Any other part of the maintenance failed: a vertical's journal or
    /// timers, the core tick, the upload queue. A data fault, never a
    /// reason to cut a recording (Mars, 2026-09-25).
    VerticalMaintenanceFailed,
    /// The Mac did not confirm an audio stop within its window.
    AudioStopUnconfirmed,
    /// An audio stop could not reach the helper within 500 ms.
    CaptureStopUndelivered,
    /// Mars's own stop could not reach the helper within 500 ms; it is
    /// already in force in the daemon.
    PrivacyStopUndelivered,
    /// `Command::Shutdown`: Orbit is closing.
    Shutdown,
    /// The IPC listener stopped: Orbit is closing.
    ListenerStopped,
    /// The helper's inventory of applications with audio could not be
    /// verified.
    AudioInventoryUnverifiable,
}

impl StopCause {
    /// Every cause, in the order of the A5 report.
    pub const ALL: [Self; 18] = [
        Self::Order,
        Self::Hotkey,
        Self::HelperMarsStop,
        Self::HelperLatch,
        Self::HelperCaptureStop,
        Self::HelperIntegrityFailure,
        Self::HelperRefusedStopped,
        Self::HelperLost,
        Self::CaptureEventFailed,
        Self::NativeEventFailed,
        Self::CaptureMaintenanceFailed,
        Self::VerticalMaintenanceFailed,
        Self::AudioStopUnconfirmed,
        Self::CaptureStopUndelivered,
        Self::PrivacyStopUndelivered,
        Self::Shutdown,
        Self::ListenerStopped,
        Self::AudioInventoryUnverifiable,
    ];

    /// The class of this cause. Exhaustive on purpose: a new cause does not
    /// compile until someone decides what it does.
    pub const fn class(self) -> StopClass {
        match self {
            Self::Order | Self::Hotkey | Self::HelperMarsStop | Self::HelperLatch => {
                StopClass::MarsStop
            }
            Self::HelperCaptureStop
            | Self::HelperIntegrityFailure
            | Self::HelperLost
            | Self::CaptureEventFailed
            | Self::CaptureMaintenanceFailed
            | Self::AudioStopUnconfirmed
            | Self::CaptureStopUndelivered
            | Self::Shutdown
            | Self::ListenerStopped => StopClass::CutCapture,
            Self::HelperRefusedStopped
            | Self::NativeEventFailed
            | Self::VerticalMaintenanceFailed
            | Self::PrivacyStopUndelivered
            | Self::AudioInventoryUnverifiable => StopClass::Notice,
        }
    }

    /// A stable name for the diagnostic log.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Hotkey => "hotkey",
            Self::HelperMarsStop => "helper_mars_stop",
            Self::HelperLatch => "helper_latch",
            Self::HelperCaptureStop => "helper_capture_stop",
            Self::HelperIntegrityFailure => "helper_integrity_failure",
            Self::HelperRefusedStopped => "helper_refused_stopped",
            Self::HelperLost => "helper_lost",
            Self::CaptureEventFailed => "capture_event_failed",
            Self::NativeEventFailed => "native_event_failed",
            Self::CaptureMaintenanceFailed => "capture_maintenance_failed",
            Self::VerticalMaintenanceFailed => "vertical_maintenance_failed",
            Self::AudioStopUnconfirmed => "audio_stop_unconfirmed",
            Self::CaptureStopUndelivered => "capture_stop_undelivered",
            Self::PrivacyStopUndelivered => "privacy_stop_undelivered",
            Self::Shutdown => "shutdown",
            Self::ListenerStopped => "listener_stopped",
            Self::AudioInventoryUnverifiable => "audio_inventory_unverifiable",
        }
    }
}

#[cfg(test)]
mod tests {
    //! Fixture setup only; production code above never panics.
    use super::*;

    /// Every code the helper can send, kept in sync by review of
    /// `OrbitApplication.swift`.
    const HELPER_CODES: [&str; 10] = [
        "local_hotkey",
        "shutdown",
        "daemon_requested",
        "daemon_cut",
        "daemon_heartbeat_expired",
        "helper_disconnected",
        "audio_device_changed",
        "session_inactive",
        "voice_processing_state_lost",
        "system_audio_stream_stopped",
    ];

    #[test]
    fn her_stop_says_where_it_came_from_and_the_helper_code() {
        use crate::notice::{DataValue, Token};
        for origin in [
            MarsStop::Order,
            MarsStop::Hotkey,
            MarsStop::HelperLatch { code: None },
            MarsStop::HelperLatch {
                code: Some("unreported".into()),
            },
            MarsStop::Helper {
                code: "local_hotkey".into(),
            },
        ] {
            let notice = origin.notice();
            assert_eq!(notice.code, NoticeCode::PrivacyStopped, "{origin:?}");
            assert_eq!(
                notice.data.get(&NoticeKey::Origin),
                Token::new(origin.cause().name())
                    .map(DataValue::Token)
                    .as_ref(),
                "{origin:?}"
            );
            assert_eq!(origin.cause().class(), StopClass::MarsStop, "{origin:?}");
        }
        assert_eq!(
            MarsStop::Helper {
                code: "local_hotkey".into()
            }
            .notice()
            .data
            .get(&NoticeKey::Code),
            Token::new("local_hotkey").map(DataValue::Token).as_ref()
        );
        assert_eq!(
            MarsStop::HelperLatch { code: None }
                .notice()
                .data
                .get(&NoticeKey::Code),
            None,
            "a latch found on arrival says only what is known"
        );
    }

    #[test]
    fn only_her_brake_latches_the_helper() {
        // Quitting, sleeping, a changed audio route, a dead service or the
        // daemon's own shutdown stop capture; none of them is her brake.
        for code in HELPER_CODES {
            assert_eq!(
                latches(code),
                matches!(code, "local_hotkey" | "daemon_requested"),
                "{code} was classified wrong"
            );
        }
        assert!(!latches("something_new_upstream"));
        for code in DAEMON_ECHO_CODES {
            assert!(HELPER_CODES.contains(&code), "{code} is a helper code");
        }
    }

    #[test]
    fn every_cause_has_one_name() {
        let mut names: Vec<&str> = StopCause::ALL.iter().map(|cause| cause.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), StopCause::ALL.len(), "two causes share a name");
    }
}
