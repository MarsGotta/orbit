//! What a helper code, a failed start or a released slot says to Deimos: the
//! typed refusal that travels in `device.reply`.
use super::super::Runtime;
use orbit_core::refusal::{
    EngineCause, Expected, InternalCause, Operation, Parameter, Permission, ReadInterruption,
    Refusal, Resource,
};
use serde_json::json;

/// Bounds a helper error-code label before it enters a typed refusal.
/// The 39 helper codes top out at 39 bytes (measured 2026-10-20); 64 retains
/// the existing cause-label budget. This is neither
/// the frame-size limit nor a filter for conversation content.
const MAX_BRIDGE_CODE_BYTES: usize = 64;

/// Bounds only fixed or validated parameter detail in a refusal frame.
/// Retains the former wire field's 400-character budget so retiring that
/// format does not enlarge these diagnostics. It does not bound conversation
/// content or the number of events in a calendar answer.
pub(super) const MAX_BRIDGE_DETAIL_CHARS: usize = 400;

pub(super) fn source_start_refusal(error: super::super::sources::SourceStartError) -> Refusal {
    use super::super::sources::SourceStartError;
    match error {
        SourceStartError::PrivacyStopped => Refusal::PrivacyStopped { since_ms: None },
        SourceStartError::HelperNotRunning | SourceStartError::NativeChannelClosed => {
            Refusal::HelperNotRunning
        }
        SourceStartError::Busy | SourceStartError::NativeChannelBusy => Refusal::Busy {
            operation: Operation::Capture,
            retry_after_ms: None,
        },
        SourceStartError::NativeCommand(_) => Refusal::Internal {
            cause: InternalCause::NativeCommand,
        },
    }
}

/// Why a bridge slot was freed outside the helper's own answer.
///
/// Each ending names the cause so the refusal the caller sees matches the same
/// wire shape the rejection path already uses, and the person can tell which
/// ending fired rather than only learning the request was abandoned.
#[derive(Debug, Clone, Copy)]
pub(in crate::daemon) enum BridgeReleaseReason<'a> {
    /// The helper did not reply before the operation's own deadline.
    Timeout { waited_ms: u64, deadline_ms: u64 },
    /// The hub did not confirm a capture upload within the bridge's margin.
    CaptureUploadTimeout { waited_ms: u64, deadline_ms: u64 },
    /// The operation ended before any answer arrived (helper restart, turn
    /// interrupt, new conversation, manual stop).
    Cancelled,
    /// A local control failure withdrew a remote action before its reply.
    ComputerRevoked(&'a Refusal),
    /// An authority fact withdrew computer control before its reply.
    Authority(orbit_core::authority::AuthorityChange),
    /// The person latched privacy while the request was still in flight.
    PrivacyStopped,
}

impl BridgeReleaseReason<'_> {
    pub(super) fn refusal(self) -> Refusal {
        match self {
            Self::Timeout {
                waited_ms,
                deadline_ms,
            }
            | Self::CaptureUploadTimeout {
                waited_ms,
                deadline_ms,
            } => Refusal::Timeout {
                operation: Operation::Capture,
                waited_ms,
                deadline_ms,
            },
            Self::Cancelled => Refusal::ReadInterrupted {
                cause: ReadInterruption::Cancelled,
            },
            Self::ComputerRevoked(refusal) => refusal.clone(),
            Self::Authority(change) => Refusal::ReadInterrupted {
                cause: ReadInterruption::Authority { change },
            },
            Self::PrivacyStopped => Refusal::PrivacyStopped { since_ms: None },
        }
    }
}

impl Runtime {
    /// The refusal a helper code (or a bridge-local cause named like one)
    /// stands for. Every code the helper can send is named; one it cannot is
    /// said as such, with its bounded label.
    pub(in crate::daemon) fn bridge_refusal_code(code: &str) -> Refusal {
        general_code(code)
            .or_else(|| permission_code(code))
            .or_else(|| interruption_code(code))
            .or_else(|| calendar_code(code))
            .unwrap_or_else(|| helper_code(code))
    }
}

fn general_code(code: &str) -> Option<Refusal> {
    Some(match code {
        "privacy_stopped" => Refusal::PrivacyStopped { since_ms: None },
        "helper_not_connected" | "helper_not_running" => Refusal::HelperNotRunning,
        // The helper's main thread held a full backlog, its general lane
        // already queues as many orders as it admits, or the order holding
        // that lane passed its deadline: a busy Mac.
        "busy" | "native_main_thread_busy" | "native_backpressure" | "native_command_stalled" => {
            Refusal::Busy {
                operation: Operation::Interaction,
                retry_after_ms: None,
            }
        }
        "engine_unavailable" | "action_unavailable" => Refusal::EngineUnavailable {
            cause: EngineCause::Unhealthy,
        },
        "window_not_found" | "capture_missing" => Refusal::NotFound {
            resource: Resource::Source,
        },
        "native_command_invalid" | "calendar_command_invalid" => Refusal::Internal {
            cause: InternalCause::NativeCommand,
        },
        "invalid_parameters" | "unknown_capability" => Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::ValidSelection,
        },
        "mac_disconnected" => Refusal::MacDisconnected { last_seen_ms: None },
        "source_inventory_failed" | "source_capture_failed" => Refusal::HelperReplyInvalid {
            field: "source.response".into(),
            expected: "source_result".into(),
            received_type: "absent".into(),
        },
        "capture_upload_failed" => Refusal::UpstreamFailed {
            service: orbit_core::refusal::Service::Hub,
            status: None,
        },
        "native_timeout" | "calendar_reply_timeout" => Refusal::HelperReplyInvalid {
            field: "response".into(),
            expected: "reply_before_deadline".into(),
            received_type: "absent".into(),
        },
        _ => return None,
    })
}

fn permission_code(code: &str) -> Option<Refusal> {
    Some(match code {
        "calendar_permission_required" => Refusal::PermissionDenied {
            permission: Permission::Calendar,
        },
        "calendar_permission_unknown" => Refusal::PermissionUnknown {
            permission: Permission::Calendar,
        },
        "accessibility_permission_required" => Refusal::PermissionDenied {
            permission: Permission::Accessibility,
        },
        "screen_recording_permission_required" | "screen_permission_required" => {
            Refusal::PermissionDenied {
                permission: Permission::ScreenRecording,
            }
        }
        _ => return None,
    })
}

fn interruption_code(code: &str) -> Option<Refusal> {
    let cause = match code {
        "calendar_helper_stopped" => ReadInterruption::CalendarHelperStopped,
        "calendar_read_restarted" => ReadInterruption::CalendarReadRestarted,
        "calendar_generation_changed" => ReadInterruption::CalendarGenerationChanged,
        "calendar_reply_replaced" => ReadInterruption::CalendarReplyReplaced,
        "conversation_changed" => ReadInterruption::ConversationChanged,
        "source_reply_replaced" => ReadInterruption::SourceReplyReplaced,
        "capture_stopped" => ReadInterruption::CaptureStopped,
        "cancelled" | "cancelled_no_replay" => ReadInterruption::Cancelled,
        "helper_restarted" => ReadInterruption::HelperRestarted {
            previous_generation: None,
            current_generation: None,
            previous_instance: None,
            current_instance: None,
        },
        "calendar_read_expired" => ReadInterruption::CalendarExpired {
            deadline_ms: orbit_core::calendar::READ_LIFETIME_NS / 1_000_000,
        },
        _ => return None,
    };
    Some(Refusal::ReadInterrupted { cause })
}

fn calendar_code(code: &str) -> Option<Refusal> {
    Some(match code {
        "calendar_busy" => Refusal::Busy {
            operation: Operation::Calendar,
            retry_after_ms: None,
        },
        "calendar_encode_failed" => Refusal::EncodeFailed {
            resource: Resource::Calendar,
        },
        "calendar_backend_unavailable" => Refusal::CalendarBackendUnavailable,
        "calendar_event_too_large" => Refusal::CalendarReadLimit {
            cause: orbit_core::refusal::CalendarReadLimit::Page {
                max_bytes: orbit_core::calendar::MAX_NATIVE_PAGE_BYTES as u64,
            },
        },
        "calendar_limit_exceeded" => Refusal::CalendarReadLimit {
            cause: orbit_core::refusal::CalendarReadLimit::Fields {
                max_title_bytes: orbit_core::calendar::MAX_TITLE_BYTES as u64,
                max_label_bytes: orbit_core::calendar::MAX_LABEL_BYTES as u64,
                max_timezone_bytes: orbit_core::calendar::MAX_TIMEZONE_BYTES as u64,
            },
        },
        _ => return None,
    })
}

/// A code the helper refuses with on its own: one it is known to send, or
/// one it is not, said with its bounded label.
fn helper_code(code: &str) -> Refusal {
    match serde_json::from_value::<orbit_core::refusal::HelperCode>(json!(code)) {
        Ok(code) => Refusal::HelperRefused { code },
        Err(_) => Refusal::HelperReplyInvalid {
            field: "error.code".into(),
            received_type: "string".into(),
            expected: Expected::KnownHelperCode {
                received: if code.len() <= MAX_BRIDGE_CODE_BYTES
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                {
                    code.into()
                } else {
                    "invalid_helper_code_label".into()
                },
                bytes: code.len() as u64,
                max_bytes: MAX_BRIDGE_CODE_BYTES as u64,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_command_failure_says_it_was_local_and_keeps_the_cause() -> anyhow::Result<()> {
        let Err(error) = crate::native::command("screen_sources", json!({"id":""})) else {
            anyhow::bail!("empty native command ID was accepted");
        };
        let failure = source_start_refusal(
            super::super::super::sources::SourceStartError::NativeCommand(error),
        );
        assert_eq!(failure.code().name(), "internal");
        let refusal = serde_json::to_value(&failure)?;
        assert_eq!(refusal["code"], "internal");
        assert_eq!(refusal["cause"]["kind"], "native_command");
        Ok(())
    }

    /// A read Deimos asked for that the helper refused because its main
    /// thread held a full backlog is a busy Mac, as the hub already says it.
    #[test]
    fn a_read_refused_behind_a_held_main_thread_is_a_busy_mac() {
        assert_eq!(
            Runtime::bridge_refusal_code("native_main_thread_busy"),
            Refusal::Busy {
                operation: Operation::Interaction,
                retry_after_ms: None,
            }
        );
    }

    /// A read the helper refused because its general lane already holds a
    /// full queue is a busy Mac too, never a reply the daemon could not read.
    #[test]
    fn a_read_refused_behind_a_full_general_lane_is_a_busy_mac() {
        assert_eq!(
            Runtime::bridge_refusal_code("native_backpressure"),
            Refusal::Busy {
                operation: Operation::Interaction,
                retry_after_ms: None,
            }
        );
    }

    /// A read the helper refused because the order holding its general lane
    /// passed its own deadline is a busy Mac as well: Deimos can ask again,
    /// and nothing in the reply was unreadable.
    #[test]
    fn a_read_refused_behind_a_stalled_general_lane_is_a_busy_mac() {
        assert_eq!(
            Runtime::bridge_refusal_code("native_command_stalled"),
            Refusal::Busy {
                operation: Operation::Interaction,
                retry_after_ms: None,
            }
        );
    }

    #[test]
    fn old_timeout_notice_keeps_the_measured_budget() {
        let notice = BridgeReleaseReason::Timeout {
            waited_ms: 10_317,
            deadline_ms: 10_000,
        }
        .refusal();
        let wire = serde_json::to_value(notice).unwrap_or_default();
        assert_eq!(wire["code"], "timeout");
        assert_eq!(wire["waited_ms"], 10_317);
        assert_eq!(wire["deadline_ms"], 10_000);
    }
}
