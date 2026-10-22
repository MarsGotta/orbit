//! Remote device bridge requests and replies.
use super::ticketed::DeadlineKind;
use super::*;
use base64::Engine;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::bridge::{
    ActionReceipt, BridgeResult, ControlToolsResult, DeviceReplyParams, PerformActionResult,
    ScreenSourcesResult, ScreenshotResult, SelectedTextResult, WindowSummary,
};
use orbit_core::wire::native::{
    NativeResponseView, SelectedTextResult as NativeSelectedTextResult,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct EncodedCapture<'a> {
    data_base64: &'a str,
}

#[derive(Deserialize)]
struct ComputerWindows {
    structured: ComputerWindowRows,
}

#[derive(Deserialize)]
struct ComputerWindowRows {
    windows: Vec<ComputerWindow>,
}

#[derive(Deserialize)]
struct ComputerWindow {
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    application_name: Option<String>,
    #[serde(default)]
    application: Option<String>,
    title: String,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    window_id: Option<u64>,
    #[serde(default)]
    bundle_id: Option<String>,
    #[serde(default)]
    z_index: Option<i64>,
}

impl ComputerWindow {
    fn name(&self) -> Option<&str> {
        self.app_name
            .as_deref()
            .or(self.application_name.as_deref())
            .or(self.application.as_deref())
    }
}

#[derive(Deserialize)]
struct ComputerScreenshotSize {
    #[serde(default)]
    screenshot_width: Option<u64>,
    #[serde(default)]
    screenshot_height: Option<u64>,
}

#[derive(Deserialize)]
struct HubRequestHeader<'a> {
    request_id: &'a str,
    #[serde(default)]
    capability: Option<Value>,
}

fn invalid_parameter_refusal(
    raw: &Value,
    capability: orbit_core::device_bridge::Capability,
) -> orbit_core::refusal::Refusal {
    use orbit_core::refusal::{Expected, Parameter, Refusal};
    use orbit_core::wire::bridge::{
        CalendarReadParameters, ControlToolsParameters, EmptyParameters, PerformActionParameters,
        ScreenshotParameters, SelectedTextParameters,
    };
    fn path<T: serde::de::DeserializeOwned>(value: &Value) -> Option<String> {
        let encoded = serde_json::to_vec(value).ok()?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        serde_path_to_error::deserialize::<_, T>(&mut decoder)
            .err()
            .map(|error| error.path().to_string())
    }
    let parameters = raw.get("parameters");
    let mut field = if let Some(parameters) = parameters.filter(|value| value.is_object()) {
        match capability {
            orbit_core::device_bridge::Capability::Screenshot => {
                path::<ScreenshotParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::ScreenSources => {
                path::<EmptyParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::SelectedText => {
                path::<SelectedTextParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::ControlTools => {
                path::<ControlToolsParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::PerformAction => {
                path::<PerformActionParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::CalendarRead => {
                path::<CalendarReadParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::FilesList => {
                path::<orbit_core::wire::bridge::FilesListParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::FilesSearch => {
                path::<orbit_core::wire::bridge::FilesSearchParameters>(parameters)
            }
            orbit_core::device_bridge::Capability::FilesRead => {
                path::<orbit_core::wire::bridge::FilesReadParameters>(parameters)
            }
        }
        .unwrap_or_else(|| "parameters".into())
    } else {
        "parameters".into()
    };
    if capability == orbit_core::device_bridge::Capability::CalendarRead
        && let Some(parameters) = parameters.filter(|value| value.is_object())
    {
        if parameters.get("start_ms").is_none() {
            field = "start_ms".into();
        } else if parameters.get("end_ms").is_none() {
            field = "end_ms".into();
        }
    }
    let observed = if field == "parameters" {
        parameters
    } else {
        parameters.and_then(|parameters| {
            field
                .split('.')
                .try_fold(parameters, |value, key| value.get(key))
        })
    };
    let received_type = match observed {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    };
    let expected =
        if field == "parameters" || field.ends_with("arguments") || field.ends_with("action") {
            "object"
        } else if field.ends_with("full_screen") || field.ends_with("content") {
            "boolean"
        } else if field.ends_with("after_index")
            || field.ends_with("after_invitee")
            || field.ends_with("after_page")
        {
            "nonnegative_integer"
        } else if field.ends_with("start_ms")
            || field.ends_with("end_ms")
            || field.ends_with("after_ms")
        {
            "unix_milliseconds_integer"
        } else {
            "string"
        };
    Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::WireField {
            field,
            expected: expected.into(),
            received_type: received_type.into(),
        },
    }
}

fn unknown_capture_filter(
    raw: &Value,
    capability: orbit_core::device_bridge::Capability,
) -> Option<orbit_core::refusal::Refusal> {
    let allowed: &[&str] = match capability {
        orbit_core::device_bridge::Capability::Screenshot => {
            &["application", "title", "full_screen"]
        }
        orbit_core::device_bridge::Capability::SelectedText => &["application", "title"],
        orbit_core::device_bridge::Capability::ScreenSources
        | orbit_core::device_bridge::Capability::ControlTools
        | orbit_core::device_bridge::Capability::PerformAction
        | orbit_core::device_bridge::Capability::CalendarRead
        | orbit_core::device_bridge::Capability::FilesList
        | orbit_core::device_bridge::Capability::FilesSearch
        | orbit_core::device_bridge::Capability::FilesRead => return None,
    };
    let parameters = raw.get("parameters")?.as_object()?;
    let key = parameters
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))?;
    Some(orbit_core::refusal::Refusal::InvalidParameter {
        parameter: orbit_core::refusal::Parameter::Request,
        expected: orbit_core::refusal::Expected::SelectionFilters {
            unknown: key.chars().take(MAX_BRIDGE_DETAIL_CHARS).collect(),
            allowed: allowed.iter().map(|value| (*value).to_owned()).collect(),
        },
    })
}

/// The windows a capture may take, of those that match what Deimos asked:
/// the titled ones when there are any. macOS draws its own overlays —
/// the cursor (CursorUIViewService, 128×128 in a real call on 2026-10-18),
/// menu-bar items, banners — as untitled windows in front of every other, so
/// the frontmost window is not the one the user is looking at. A person's
/// window has the title macOS shows in the Window menu and in Mission
/// Control. An untitled window
/// is still taken when no titled one matches, so an application whose window
/// has no title can be captured by its name. Neither inventory carries a
/// window's level, which is what macOS itself tells overlays apart by; size
/// is not used because a threshold would be a guess and the helper's rows
/// carry none, nor the owner, because a list of system services goes stale.
fn titled_first<T>(candidates: Vec<T>, title: impl Fn(&T) -> &str) -> Vec<T> {
    let titled = |candidate: &T| !title(candidate).trim().is_empty();
    if candidates.iter().any(titled) {
        candidates.into_iter().filter(titled).collect()
    } else {
        candidates
    }
}

fn result_payload(request_id: &str, result: BridgeResult) -> Value {
    json!(DeviceReplyParams::answered(request_id.to_owned(), result))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

/// How long either capture upload may take to reach the hub before the caller
/// is told that it did not. This bounds the bridge reply, not local sealing or
/// the HTTP client's general 90 s limit.
///
/// The hub gives a bridge request 30 s (`REQUEST_TIMEOUT_SECONDS`), so an
/// upload has to give up with enough margin for the refusal to travel. A
/// capture still uploading at 20 s would not have been useful anyway, and a
/// wait that outlives the caller's own is a reply that never leaves the Mac.
const CAPTURE_UPLOAD_TIMEOUT: DeadlineKind = DeadlineKind::new(
    "capture_upload",
    Duration::from_secs(orbit_core::limits::CAPTURE_UPLOAD_TIMEOUT_SECONDS),
    "One capture upload; 20 s leaves 10 s before the hub request limit; not local sealing, data size, or retry",
);

pub(super) enum PendingCapture {
    Engine {
        capture_id: String,
        application: String,
        title: String,
        width: u32,
        height: u32,
        mime_type: String,
    },
    Helper {
        source_id: String,
    },
    /// An image from the assistant's folders, fitted by the helper; the
    /// answer waits for the hub to hold it, as a screenshot's does.
    File {
        capture_id: String,
        result: Box<orbit_core::wire::bridge::FilesReadResult>,
    },
}

impl PendingCapture {
    fn id(&self) -> &str {
        match self {
            Self::Engine { capture_id, .. } | Self::File { capture_id, .. } => capture_id,
            Self::Helper { source_id } => source_id,
        }
    }
}

pub(super) enum CaptureUploadOutcome {
    Uploaded(String),
    Failed(anyhow::Error),
    TimedOut { waited_ms: u64 },
}

async fn bounded_capture_upload(
    hub_url: &str,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    bytes: Vec<u8>,
    op: &orbit_core::notice::Op,
) -> CaptureUploadOutcome {
    capture_upload_outcome(hub::upload_capture(
        hub_url,
        token,
        development,
        capture_id,
        request_id,
        bytes,
        op,
    ))
    .await
}

async fn capture_upload_outcome(
    upload: impl std::future::Future<Output = Result<String>>,
) -> CaptureUploadOutcome {
    let deadline = CAPTURE_UPLOAD_TIMEOUT.start();
    match tokio::time::timeout_at(deadline.at().into(), upload).await {
        Ok(Ok(sha256)) => CaptureUploadOutcome::Uploaded(sha256),
        Ok(Err(error)) => CaptureUploadOutcome::Failed(error),
        // Tokio's timer reached the deadline even when a paused test clock
        // leaves std::Instant stationary. A timeout therefore waited at least
        // its budget; a delayed poll may have waited longer on the real clock.
        Err(_) => CaptureUploadOutcome::TimedOut {
            waited_ms: deadline.waited_ms().max(deadline.budget_ms()),
        },
    }
}

/// Bounds a helper error-code label before it enters a typed refusal.
/// The 39 helper codes top out at 39 bytes (measured 2026-10-20); 64 retains
/// the existing cause-label budget. This is neither
/// the frame-size limit nor a filter for conversation content.
const MAX_BRIDGE_CODE_BYTES: usize = 64;

/// Bounds only fixed or validated parameter detail in a refusal frame.
/// Retains the former wire field's 400-character budget so retiring that
/// format does not enlarge these diagnostics. It does not bound conversation
/// content or the number of events in a calendar answer.
const MAX_BRIDGE_DETAIL_CHARS: usize = 400;

fn source_start_refusal(error: super::sources::SourceStartError) -> orbit_core::refusal::Refusal {
    use orbit_core::refusal::{InternalCause, Operation, Refusal};
    match error {
        super::sources::SourceStartError::PrivacyStopped => {
            Refusal::PrivacyStopped { since_ms: None }
        }
        super::sources::SourceStartError::HelperNotRunning
        | super::sources::SourceStartError::NativeChannelClosed => Refusal::HelperNotRunning,
        super::sources::SourceStartError::Busy
        | super::sources::SourceStartError::NativeChannelBusy => Refusal::Busy {
            operation: Operation::Capture,
            retry_after_ms: None,
        },
        super::sources::SourceStartError::NativeCommand(_) => Refusal::Internal {
            cause: InternalCause::NativeCommand,
        },
    }
}

/// The remote request being served, and what it asked for.
///
/// The capability travels with the id because the answer is assembled somewhere
/// else entirely, when the helper replies: without it, whatever came back would
/// be reported as whatever was asked for last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BridgeAsk {
    pub(super) id: String,
    pub(super) capability: orbit_core::device_bridge::Capability,
    pub(super) application: Option<String>,
    pub(super) title: Option<String>,
    pub(super) full_screen: bool,
    pub(super) source_id: Option<String>,
    pub(super) tool_call: Option<orbit_core::computer_control::ToolCall>,
    /// Where the catalogue page this request wants begins. Zero for every
    /// capability that is not `ControlTools`, and for the first page of one.
    pub(super) catalogue_after: usize,
    /// The dates a `CalendarRead` asked about, as they were asked for.
    ///
    /// Kept whole: the read covers the whole range, in pages, whatever its width.
    pub(super) calendar_window: Option<(i64, i64)>,
    /// Where a `CalendarRead` continues, when it is not the first page.
    pub(super) calendar_cursor: Option<super::calendar::Cursor>,
    /// What a `files_*` request asked, as it asked it.
    pub(super) files: Option<super::assistant_files::FilesAsk>,
    /// «Adjuntar ventana»: the user's own capture of the frontmost
    /// window. Its answer goes to the app as an attachment, never to the hub
    /// as a `device.reply` (`window_attachment`), and Orbit's own windows,
    /// in front while the user presses the button, are never the one taken.
    pub(super) for_attachment: bool,
}

/// The bundle identifiers of Orbit's own windows (Orbit, Orbit Pruebas, the
/// engine): what the helper also calls its own (`ownBundlePrefix`).
const ORBIT_BUNDLE_PREFIX: &str = "app.orbit.";

impl BridgeAsk {
    fn matches_window_filter(&self, application: &str, bundle: Option<&str>, title: &str) -> bool {
        if self.for_attachment
            && bundle.is_some_and(|bundle| bundle.starts_with(ORBIT_BUNDLE_PREFIX))
        {
            return false;
        }
        self.application.as_ref().is_none_or(|wanted| {
            application.eq_ignore_ascii_case(wanted)
                || bundle.is_some_and(|bundle| bundle.eq_ignore_ascii_case(wanted))
        }) && self
            .title
            .as_ref()
            .is_none_or(|wanted| title.contains(wanted))
    }
}

/// Why the bridge's single slot was freed outside the helper's own answer.
///
/// Each ending names the cause so the refusal the caller sees matches the same
/// wire shape the rejection path already uses, and the person can tell which
/// ending fired rather than only learning the request was abandoned.
#[derive(Debug, Clone, Copy)]
pub(super) enum BridgeReleaseReason<'a> {
    /// The helper did not reply before the operation's own deadline.
    Timeout { waited_ms: u64, deadline_ms: u64 },
    /// The hub did not confirm a capture upload within the bridge's margin.
    CaptureUploadTimeout { waited_ms: u64, deadline_ms: u64 },
    /// The operation ended before any answer arrived (helper restart, turn
    /// interrupt, new conversation, manual stop).
    Cancelled,
    /// A local control failure withdrew a remote action before its reply.
    ComputerRevoked(&'a orbit_core::refusal::Refusal),
    /// An authority fact withdrew computer control before its reply.
    Authority(orbit_core::authority::AuthorityChange),
    /// The person latched privacy while the request was still in flight.
    PrivacyStopped,
}

impl BridgeReleaseReason<'_> {
    fn refusal(self) -> orbit_core::refusal::Refusal {
        use orbit_core::refusal::{Operation, ReadInterruption, Refusal};
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
    pub(super) fn note_bridge_send(&mut self, result: &hub_link::SendResult, subject: &str) {
        if result.delivered_to_queue() {
            self.log("bridge_reply_queued", &[("subject", subject)]);
            self.say(NoticeCode::BridgeReplySent);
        } else {
            self.log(
                "bridge_reply_undelivered",
                &[("subject", subject), ("cause", &result.cause().to_string())],
            );
            self.say(NoticeCode::BridgeReplyUndelivered);
        }
    }
    pub(super) fn bridge_refusal_frame(
        &self,
        request_id: &str,
        refusal: &orbit_core::refusal::Refusal,
    ) -> Value {
        json!(DeviceReplyParams::refused(
            request_id.to_owned(),
            refusal.clone(),
            None
        ))
    }

    fn bridge_start_frame(
        &self,
        request_id: &str,
        refusal: &orbit_core::refusal::Refusal,
    ) -> Value {
        let mut reply = DeviceReplyParams::refused(request_id.to_owned(), refusal.clone(), None);
        if let orbit_core::computer_control::engine::Status::Failed { retry_in_ms, .. } =
            self.engine_status()
            && matches!(
                refusal,
                orbit_core::refusal::Refusal::EngineUnavailable { .. }
            )
            && let Some(ref mut fields) = reply.refusal
        {
            fields.retry_in_ms = Some(retry_in_ms);
        }
        json!(reply)
    }

    fn bridge_engine_refusal(&self) -> orbit_core::refusal::Refusal {
        use orbit_core::computer_control::engine::Status;
        use orbit_core::refusal::{
            EngineCause, Refusal, missing_permission_name, missing_system_permission_refusal,
        };
        match self.engine_status() {
            Status::Failed { .. } => Refusal::EngineUnavailable {
                cause: EngineCause::InitializationFailed,
            },
            Status::Starting => Refusal::EngineUnavailable {
                cause: EngineCause::NotRunning,
            },
            Status::Ready { .. } => Refusal::EngineUnavailable {
                cause: EngineCause::Unhealthy,
            },
            Status::PrivacyStopped => Refusal::PrivacyStopped { since_ms: None },
            Status::HelperNotConnected => Refusal::HelperNotRunning,
            Status::PermissionMissing { permissions } => missing_system_permission_refusal(
                permissions,
                self.state.capabilities().accessibility_state(),
                self.state.capabilities().screen_state(),
            ),
            Status::PermissionUnknown { permissions } => Refusal::PermissionUnknown {
                permission: missing_permission_name(permissions),
            },
        }
    }

    fn bridge_computer_start_failure(
        &self,
        error: super::computer_control::ComputerStartError,
    ) -> orbit_core::refusal::Refusal {
        use orbit_core::refusal::{Expected, Operation, Parameter, Refusal};
        match error {
            super::computer_control::ComputerStartError::Authority { refusal, .. } => refusal,
            super::computer_control::ComputerStartError::NotPaired => Refusal::NotPaired,
            super::computer_control::ComputerStartError::Busy(_) => Refusal::Busy {
                operation: Operation::ComputerAction,
                retry_after_ms: None,
            },
            super::computer_control::ComputerStartError::InvalidTool => Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::ValidSelection,
            },
            super::computer_control::ComputerStartError::UndeclaredTool(tool) => {
                Refusal::InvalidParameter {
                    parameter: Parameter::Request,
                    expected: Expected::DeclaredTool { received: tool },
                }
            }
            super::computer_control::ComputerStartError::InvalidHistory => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::ControlHistory,
            },
            super::computer_control::ComputerStartError::Storage(_) => Refusal::StorageFailed {
                operation: Operation::ComputerAction,
                cause: None,
            },
            super::computer_control::ComputerStartError::EngineUnavailable => {
                self.bridge_engine_refusal()
            }
            super::computer_control::ComputerStartError::CaptureFile(_) => Refusal::StorageFailed {
                operation: Operation::Capture,
                cause: None,
            },
            super::computer_control::ComputerStartError::DesktopCoordinates(cause) => cause,
            super::computer_control::ComputerStartError::Internal => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::ComputerStart,
            },
        }
    }

    pub(super) fn bridge_refusal_code(code: &str) -> orbit_core::refusal::Refusal {
        use orbit_core::refusal::{
            EngineCause, Expected, Operation, Parameter, Permission, ReadInterruption, Refusal,
            Resource,
        };
        match code {
            "privacy_stopped" => Refusal::PrivacyStopped { since_ms: None },
            "helper_not_connected" | "helper_not_running" => Refusal::HelperNotRunning,
            "busy" => Refusal::Busy {
                operation: Operation::Interaction,
                retry_after_ms: None,
            },
            "calendar_busy" => Refusal::Busy {
                operation: Operation::Calendar,
                retry_after_ms: None,
            },
            "engine_unavailable" | "action_unavailable" => Refusal::EngineUnavailable {
                cause: EngineCause::Unhealthy,
            },
            "window_not_found" | "capture_missing" => Refusal::NotFound {
                resource: Resource::Source,
            },
            "native_command_invalid" | "calendar_command_invalid" => Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::NativeCommand,
            },
            "calendar_encode_failed" => Refusal::EncodeFailed {
                resource: orbit_core::refusal::Resource::Calendar,
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
            "calendar_backend_unavailable" => Refusal::CalendarBackendUnavailable,
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
            "calendar_helper_stopped" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarHelperStopped,
            },
            "calendar_read_restarted" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarReadRestarted,
            },
            "calendar_generation_changed" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarGenerationChanged,
            },
            "calendar_reply_replaced" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarReplyReplaced,
            },
            "conversation_changed" => Refusal::ReadInterrupted {
                cause: ReadInterruption::ConversationChanged,
            },
            "source_reply_replaced" => Refusal::ReadInterrupted {
                cause: ReadInterruption::SourceReplyReplaced,
            },
            "capture_stopped" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CaptureStopped,
            },
            "cancelled" | "cancelled_no_replay" => Refusal::ReadInterrupted {
                cause: ReadInterruption::Cancelled,
            },
            "helper_restarted" => Refusal::ReadInterrupted {
                cause: ReadInterruption::HelperRestarted {
                    previous_generation: None,
                    current_generation: None,
                    previous_instance: None,
                    current_instance: None,
                },
            },
            "calendar_read_expired" => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarExpired {
                    deadline_ms: orbit_core::calendar::READ_LIFETIME_NS / 1_000_000,
                },
            },
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
            "capture_upload_failed" => Refusal::UpstreamFailed {
                service: orbit_core::refusal::Service::Hub,
                status: None,
            },
            "native_timeout" | "calendar_reply_timeout" => Refusal::HelperReplyInvalid {
                field: "response".into(),
                expected: "reply_before_deadline".into(),
                received_type: "absent".into(),
            },
            _ => match serde_json::from_value::<orbit_core::refusal::HelperCode>(json!(code)) {
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
            },
        }
    }

    /// Carries out a remote request once it has been admitted, and answers it.
    ///
    /// Only admitted capabilities arrive here, so a request cannot
    /// be accepted and then quietly dropped.
    pub(super) async fn serve_bridge_request(
        &mut self,
        ask: &BridgeAsk,
    ) -> std::result::Result<(), orbit_core::refusal::Refusal> {
        match ask.capability {
            orbit_core::device_bridge::Capability::ScreenSources => {
                if self.computer_connection_epoch().is_some()
                    && self.read_computer_windows().is_ok()
                {
                    return Ok(());
                }
                self.source_windows().await.map_err(source_start_refusal)
            }
            orbit_core::device_bridge::Capability::Screenshot => {
                if ask.full_screen {
                    if ask.application.is_some() || ask.title.is_some() {
                        return Err(orbit_core::refusal::Refusal::InvalidParameter {
                            parameter: orbit_core::refusal::Parameter::Request,
                            expected: orbit_core::refusal::Expected::FullScreenFilters {
                                application: ask.application.is_some(),
                                title: ask.title.is_some(),
                            },
                        });
                    }
                    return self
                        .read_bridge_screenshot(
                            None,
                            None,
                            "Escritorio".into(),
                            "Pantalla completa".into(),
                        )
                        .map_err(|error| self.bridge_computer_start_failure(error));
                }
                if self.computer_connection_epoch().is_some()
                    && self.read_computer_windows().is_ok()
                {
                    return Ok(());
                }
                self.source_windows().await.map_err(source_start_refusal)
            }
            orbit_core::device_bridge::Capability::SelectedText => {
                // The same path the person's own window listing takes. A second
                // route to the helper would be a second set of rules about when
                // capture is allowed, and only one of them would be maintained.
                self.source_windows().await.map_err(source_start_refusal)
            }
            // The catalogue comes from the engine, asked of it, not from
            // a list Orbit keeps. A newer engine's tool appears here on its own.
            orbit_core::device_bridge::Capability::ControlTools => {
                if !self.engine_is_ready() {
                    return Err(self.bridge_engine_refusal());
                }
                // One page, never the whole catalogue. The engine's is
                // 147 909 bytes and the link refuses anything over 128 KiB, so
                // a reply that asked for a whole frame was a reply that never
                // left the Mac.
                let catalogue = self.engine_tools();
                let page =
                    orbit_core::device_bridge::catalogue_page(&catalogue, ask.catalogue_after);
                let result = BridgeResult::ControlTools(ControlToolsResult {
                    tools: page.tools,
                    after_index: page.after_index,
                    total: page.total,
                    next_after_index: page.next_after_index,
                });
                self.conversation.bridge_request = None;
                self.send(
                    hub::command(&ask.id, "device.reply", result_payload(&ask.id, result))
                        .map_err(|_| orbit_core::refusal::Refusal::HubUnreachable {
                            since_ms: None,
                        })?,
                )
                .await
                .map_err(|_| orbit_core::refusal::Refusal::HubUnreachable { since_ms: None })
            }
            // The engine or nothing. There is no adapter fallback
            // left to divert into, and an absent engine is said out loud.
            orbit_core::device_bridge::Capability::PerformAction => {
                let call = ask.tool_call.clone().ok_or({
                    orbit_core::refusal::Refusal::InvalidParameter {
                        parameter: orbit_core::refusal::Parameter::Request,
                        expected: orbit_core::refusal::Expected::ValidSelection,
                    }
                })?;
                self.call_engine_tool(call, "Deimos (canal no indicado)")
                    .map_err(|error| self.bridge_computer_start_failure(error))
            }
            // EventKit, for these dates, now, for
            // every calendar, answered in pages. Every way this can fail is
            // named here and now rather than left to the caller's timeout,
            // because "no answer" about a calendar reads as "nothing in your
            // diary" and that is the one wrong answer.
            orbit_core::device_bridge::Capability::CalendarRead => {
                let (start_ms, end_ms) = ask.calendar_window.ok_or({
                    orbit_core::refusal::Refusal::InvalidParameter {
                        parameter: orbit_core::refusal::Parameter::Request,
                        expected: orbit_core::refusal::Expected::ValidSelection,
                    }
                })?;
                match self.calendar_bridge_read(
                    &ask.id,
                    start_ms,
                    end_ms,
                    ask.calendar_cursor.clone(),
                ) {
                    Ok(()) => Ok(()),
                    Err(refusal) => {
                        self.conversation.bridge_request = None;
                        self.refuse_bridge_admission(&ask.id, &refusal)
                            .await
                            .map_err(|_| orbit_core::refusal::Refusal::HubUnreachable {
                                since_ms: None,
                            })
                    }
                }
            }
            // The folders the user chose in the macOS panel, resolved by
            // the helper and decided in `orbit_core::assistant_files`.
            orbit_core::device_bridge::Capability::FilesList
            | orbit_core::device_bridge::Capability::FilesSearch
            | orbit_core::device_bridge::Capability::FilesRead => self.files_bridge_start(ask),
        }
    }

    pub(super) async fn answer_bridge_from_computer_windows(
        &mut self,
        observation: &Value,
    ) -> Result<bool> {
        let Some(ask) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == orbit_core::device_bridge::Capability::ScreenSources)
            .cloned()
        else {
            return Ok(true);
        };
        let Ok(listing) = ComputerWindows::deserialize(observation) else {
            return Ok(false);
        };
        let mut windows = Vec::with_capacity(listing.structured.windows.len());
        for row in listing.structured.windows {
            let Some(application) = row.name() else {
                return Ok(false);
            };
            windows.push(WindowSummary {
                application: application.to_owned(),
                title: row.title,
            });
        }
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(
                &ask.id,
                BridgeResult::ScreenSources(ScreenSourcesResult { windows }),
            ),
        ));
        self.note_bridge_send(&sent, "the engine's windows reply");
        self.conversation.bridge_request = None;
        Ok(true)
    }

    pub(super) fn bridge_screenshot_target(
        &self,
        observation: &Value,
    ) -> Option<(i64, u64, String, String)> {
        let ask =
            self.conversation.bridge_request.as_ref().filter(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::Screenshot
            })?;
        let listing = ComputerWindows::deserialize(observation).ok()?;
        let matching = listing
            .structured
            .windows
            .iter()
            .filter_map(|row| {
                let pid = row.pid?;
                let window_id = row.window_id?;
                let application = row.name()?;
                let title = row.title.as_str();
                if pid <= 0
                    || window_id == 0
                    || !ask.matches_window_filter(application, row.bundle_id.as_deref(), title)
                {
                    return None;
                }
                Some((
                    row.z_index.unwrap_or(i64::MIN),
                    pid,
                    window_id,
                    application.to_owned(),
                    title.to_owned(),
                ))
            })
            .collect();
        titled_first(matching, |candidate| candidate.4.as_str())
            .into_iter()
            .max_by_key(|candidate| candidate.0)
            .map(|(_, pid, window_id, application, title)| (pid, window_id, application, title))
    }

    pub(super) async fn answer_bridge_from_computer_screenshot(
        &mut self,
        observation: &crate::cua::NativeObservation,
        application: &str,
        title: &str,
    ) -> Result<bool> {
        let Some(ask) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == orbit_core::device_bridge::Capability::Screenshot)
            .cloned()
        else {
            return Ok(true);
        };
        let Some(image) = observation
            .images
            .first()
            .filter(|_| observation.images.len() == 1)
        else {
            return Ok(false);
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&image.data) else {
            return Ok(false);
        };
        let Some((width, height)) = png_dimensions(&bytes) else {
            return Ok(false);
        };
        if orbit_core::sources::validate_png(&bytes, width, height).is_err() {
            return Ok(false);
        }
        let structured = observation
            .structured
            .as_ref()
            .and_then(|value| ComputerScreenshotSize::deserialize(value).ok());
        if structured
            .as_ref()
            .and_then(|value| value.screenshot_width)
            .is_some_and(|value| value != u64::from(width))
            || structured
                .as_ref()
                .and_then(|value| value.screenshot_height)
                .is_some_and(|value| value != u64::from(height))
        {
            return Ok(false);
        }
        let mime_type = image.mime_type.clone();
        self.upload_engine_capture(&ask, bytes, (width, height), mime_type, application, title)
            .await
            .map(|_| true)
    }

    /// The full screen, fitted, goes out as any engine capture does;
    /// its scale is kept with the upload and becomes the one desktop actions
    /// are mapped with once Deimos has been sent this image.
    pub(super) async fn answer_bridge_with_desktop(
        &mut self,
        fitted: orbit_core::sources::FittedDesktop,
    ) -> Result<()> {
        let Some(ask) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| ask.capability == orbit_core::device_bridge::Capability::Screenshot)
            .cloned()
        else {
            return Ok(());
        };
        let scale = fitted.scale;
        self.log(
            "desktop_capture_fitted",
            &[
                (
                    "screen",
                    &format!("{}x{}", scale.screen_width, scale.screen_height),
                ),
                (
                    "image",
                    &format!("{}x{}", scale.image_width, scale.image_height),
                ),
                ("bytes", &fitted.png.len().to_string()),
            ],
        );
        let uploading = self
            .upload_engine_capture(
                &ask,
                fitted.png,
                (scale.image_width, scale.image_height),
                "image/png".into(),
                "Escritorio",
                "Pantalla completa",
            )
            .await?;
        self.features.computer.pending_desktop = uploading.map(|capture_id| (capture_id, scale));
        Ok(())
    }

    /// The bytes go out of band, exactly as the helper's do. Inlining them
    /// put a 933 KiB frame on a link that admits 131 072, so "Deimos ve mi
    /// pantalla" failed on every capture worth taking. The capture's id when
    /// the upload started; `None` when it was refused at once.
    async fn upload_engine_capture(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        (width, height): (u32, u32),
        mime_type: String,
        application: &str,
        title: &str,
    ) -> Result<Option<String>> {
        let capture_id = uuid::Uuid::new_v4().to_string();
        let capture = PendingCapture::Engine {
            capture_id: capture_id.clone(),
            application: application.to_owned(),
            title: title.to_owned(),
            width,
            height,
            mime_type,
        };
        self.start_capture_upload(ask, bytes, capture_id, capture)
            .await
    }

    /// An image read from the assistant's folders goes up the way a
    /// screenshot does; `partial` is its answer without the capture's id and
    /// digest, which the upload supplies.
    pub(super) async fn upload_file_capture(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        partial: orbit_core::wire::bridge::FilesReadResult,
    ) -> Result<Option<String>> {
        let capture_id = uuid::Uuid::new_v4().to_string();
        let capture = PendingCapture::File {
            capture_id: capture_id.clone(),
            result: Box::new(partial),
        };
        self.start_capture_upload(ask, bytes, capture_id, capture)
            .await
    }

    async fn start_capture_upload(
        &mut self,
        ask: &BridgeAsk,
        bytes: Vec<u8>,
        capture_id: String,
        capture: PendingCapture,
    ) -> Result<Option<String>> {
        let (Some(hub_url), Some(token)) = (self.settings.hub_url.clone(), self.token.clone())
        else {
            return self.refuse_capture_upload(ask, None).await.map(|()| None);
        };
        let upload_id = capture_id.clone();
        let request_id = ask.id.clone();
        let generation = self.state.generation();
        let development = self.development;
        let work = self.work.work_events.clone();
        // A second capture replaces the first: its upload is aborted here, not
        // left running to be discarded later as `upload_replaced`.
        if let Some(mut previous) = self.features.sources.bridge_upload.take() {
            previous.work.cancel();
        }
        let started = capture_id.clone();
        let mut upload_work = super::ticketed::Ticketed::logical();
        upload_work.replace_logical(generation, self.turn.op.clone(), (), move |ticket| {
            tokio::spawn(async move {
                let outcome = bounded_capture_upload(
                    &hub_url,
                    &token,
                    development,
                    &capture_id,
                    &request_id,
                    bytes,
                    ticket.op(),
                )
                .await;
                deliver_work(
                    &work,
                    WorkResult::BridgeCaptureUpload {
                        ticket,
                        request_id,
                        capture,
                        outcome,
                    },
                )
                .await;
            })
        });
        self.features.sources.bridge_upload = Some(super::sources::BridgeUpload {
            capture_id: upload_id,
            screenshot: None,
            work: upload_work,
        });
        Ok(Some(started))
    }

    /// A capture Deimos asked for, refused with its cause and its numbers,
    /// as fields the translator can act on.
    pub(super) fn refuse_bridge_capture(&mut self, refusal: orbit_core::refusal::Refusal) {
        let Some(ask) = self.conversation.bridge_request.take() else {
            return;
        };
        let mut said = bridge_refused(&refusal);
        if let orbit_core::refusal::Refusal::TooLarge {
            bytes, max_bytes, ..
        } = &refusal
        {
            said = said
                .with(NoticeKey::Bytes, *bytes)
                .with(NoticeKey::MaxBytes, *max_bytes);
        }
        self.say(said);
        let code = refusal.code().name();
        let frame = json!(DeviceReplyParams::refused(ask.id.clone(), refusal, None,));
        let sent = self.send_frame(hub::command(&ask.id, "device.reply", frame));
        self.note_bridge_send(&sent, &format!("the capture refusal ({code})"));
    }

    /// Sends the answer to a remote request once the helper has replied.
    ///
    /// Called after every source response rather than from inside the source
    /// code itself, so that the bridge cannot alter what the local path does.
    pub(super) async fn answer_bridge_from_sources(&mut self, event: &Value) -> Result<()> {
        let Some(ask) = self.conversation.bridge_request.clone() else {
            return Ok(());
        };
        if ask.capability == orbit_core::device_bridge::Capability::ScreenSources {
            let payload = match self.state.views.source_inventory.as_ref() {
                Some(inventory) => {
                    let windows: Vec<WindowSummary> =
                        orbit_core::device_bridge::window_summary(inventory)
                            .into_iter()
                            .map(|(application, title)| WindowSummary { application, title })
                            .collect();
                    result_payload(
                        &ask.id,
                        BridgeResult::ScreenSources(ScreenSourcesResult { windows }),
                    )
                }
                // Still waiting: say nothing yet rather than reporting an empty Mac.
                None if self.source_operation_pending() => return Ok(()),
                None => {
                    let failure = self.features.sources.last_failure.as_ref();
                    let refusal = failure
                        .map(|failure| failure.refusal.clone())
                        .unwrap_or_else(|| Self::bridge_refusal_code("source_inventory_failed"));
                    self.bridge_refusal_frame(&ask.id, &refusal)
                }
            };
            let result = self.send_frame(hub::command(&ask.id, "device.reply", payload));
            self.note_bridge_send(&result, "the windows reply");
            self.conversation.bridge_request = None;
            return Ok(());
        }

        let response = NativeResponseView::from_value(event)?;
        if ask.source_id.is_none() {
            if !response.ok {
                return self
                    .refuse_bridge_request(&ask, "source_inventory_failed")
                    .await;
            }
            let Some(inventory) = self.state.views.source_inventory.as_ref() else {
                if self.source_operation_pending() {
                    return Ok(());
                }
                return self
                    .refuse_bridge_request(&ask, "source_inventory_failed")
                    .await;
            };
            let matching: Vec<_> = inventory
                .windows
                .iter()
                .filter(|window| {
                    ask.matches_window_filter(
                        &window.application_name,
                        Some(&window.bundle_id),
                        &window.title,
                    )
                })
                .collect();
            let selectable = matching
                .iter()
                .copied()
                .filter(|window| window.binding.is_some() && window.unavailable_reason.is_none())
                .collect();
            let token = titled_first(selectable, |window| window.title.as_str())
                .first()
                .and_then(|window| window.binding.as_ref())
                .map(|binding| binding.selection_token.clone());
            let Some(token) = token else {
                let code = if matching.is_empty() {
                    "window_not_found"
                } else {
                    "screen_selection_unavailable"
                };
                return self.refuse_bridge_request(&ask, code).await;
            };
            let format = match ask.capability {
                orbit_core::device_bridge::Capability::Screenshot => {
                    orbit_core::sources::SourceFormat::Png
                }
                orbit_core::device_bridge::Capability::SelectedText => {
                    orbit_core::sources::SourceFormat::SelectedText
                }
                orbit_core::device_bridge::Capability::ScreenSources
                | orbit_core::device_bridge::Capability::ControlTools
                | orbit_core::device_bridge::Capability::PerformAction
                | orbit_core::device_bridge::Capability::CalendarRead
                | orbit_core::device_bridge::Capability::FilesList
                | orbit_core::device_bridge::Capability::FilesSearch
                | orbit_core::device_bridge::Capability::FilesRead => return Ok(()),
            };
            if self
                .source_capture_as(&token, "Deimos".to_owned(), format)
                .await
                .is_err()
            {
                return self
                    .refuse_bridge_request(&ask, "source_capture_failed")
                    .await;
            }
            let Some(source_id) = self
                .state
                .views
                .source_request
                .as_ref()
                .map(|progress| progress.id.clone())
            else {
                return self
                    .refuse_bridge_request(&ask, "source_capture_failed")
                    .await;
            };
            if let Some(current) = self.conversation.bridge_request.as_mut() {
                current.source_id = Some(source_id);
            }
            return Ok(());
        }

        if self.source_operation_pending() {
            return Ok(());
        }
        if !response.ok {
            return self
                .refuse_bridge_request(&ask, "source_capture_failed")
                .await;
        }
        let Some(receipt) = self.state.views.source_receipt.clone().filter(|receipt| {
            ask.source_id.as_deref() == Some(receipt.intent.id.as_str())
                && receipt.intent.annotation == "Deimos"
        }) else {
            return self
                .refuse_bridge_request(&ask, "source_capture_failed")
                .await;
        };
        let (application, title, binding) = match &receipt.intent.target {
            orbit_core::sources::SourceTarget::NativeWindow {
                application_name,
                title,
                binding,
                ..
            } => (application_name.clone(), title.clone(), binding),
        };
        if ask.for_attachment {
            self.window_captured_by_helper(binding.application_pid, binding.window_id);
        }
        let result =
            match ask.capability {
                orbit_core::device_bridge::Capability::Screenshot => {
                    let (width, height) = match receipt.metadata.media {
                        orbit_core::sources::SourceMedia::Png { width, height } => (width, height),
                        orbit_core::sources::SourceMedia::SelectedText { .. } => {
                            return self
                                .refuse_bridge_request(&ask, "source_capture_failed")
                                .await;
                        }
                    };
                    self.prepare_bridge_capture_upload(event);
                    let Some(upload) = self.features.sources.bridge_upload.as_mut() else {
                        return self.refuse_capture_upload(&ask, None).await;
                    };
                    upload.screenshot = Some(ScreenshotResult {
                        capture_id: receipt.intent.id,
                        application,
                        title,
                        width,
                        height,
                        mime_type: "image/png".into(),
                        sha256: receipt.sha256,
                        coordinates: None,
                    });
                    return Ok(());
                }
                orbit_core::device_bridge::Capability::SelectedText => {
                    let body = response
                        .result
                        .context("response selected_text field=result type=absent")?;
                    let text = orbit_core::wire::native::decode_result_body::<
                        NativeSelectedTextResult,
                    >("selected_text", body)?
                    .text;
                    BridgeResult::SelectedText(SelectedTextResult {
                        application: application.clone(),
                        title: title.clone(),
                        text,
                    })
                }
                orbit_core::device_bridge::Capability::ScreenSources
                | orbit_core::device_bridge::Capability::ControlTools
                | orbit_core::device_bridge::Capability::PerformAction
                | orbit_core::device_bridge::Capability::CalendarRead
                | orbit_core::device_bridge::Capability::FilesList
                | orbit_core::device_bridge::Capability::FilesSearch
                | orbit_core::device_bridge::Capability::FilesRead => return Ok(()),
            };
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(&ask.id, result),
        ));
        self.note_bridge_send(&sent, &format!("the screen read: {application} · {title}"));
        self.conversation.bridge_request = None;
        Ok(())
    }

    pub(super) fn prepare_bridge_capture_upload(&mut self, event: &Value) {
        let Some(ask) = self.conversation.bridge_request.as_ref() else {
            return;
        };
        let Ok(response) = NativeResponseView::from_value(event) else {
            return;
        };
        if ask.capability != orbit_core::device_bridge::Capability::Screenshot
            || ask.source_id.is_none()
            || !response.ok
            || self.features.sources.bridge_upload.is_some()
        {
            return;
        }
        let Some(encoded) = response
            .result
            .and_then(|body| EncodedCapture::deserialize(body).ok())
            .map(|body| body.data_base64)
        else {
            return;
        };
        let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        else {
            return;
        };
        let Some(hub_url) = self.settings.hub_url.clone() else {
            return;
        };
        let Some(token) = self.token.clone() else {
            return;
        };
        let capture_id = ask.source_id.clone().unwrap_or_default();
        let upload_id = capture_id.clone();
        let request_id = ask.id.clone();
        let generation = self.state.generation();
        let development = self.development;
        let work = self.work.work_events.clone();
        let mut upload_work = super::ticketed::Ticketed::logical();
        upload_work.replace_logical(generation, self.turn.op.clone(), (), move |ticket| {
            tokio::spawn(async move {
                let outcome = bounded_capture_upload(
                    &hub_url,
                    &token,
                    development,
                    &capture_id,
                    &request_id,
                    bytes,
                    ticket.op(),
                )
                .await;
                deliver_work(
                    &work,
                    WorkResult::BridgeCaptureUpload {
                        ticket,
                        request_id,
                        capture: PendingCapture::Helper {
                            source_id: capture_id,
                        },
                        outcome,
                    },
                )
                .await;
            })
        });
        self.features.sources.bridge_upload = Some(super::sources::BridgeUpload {
            capture_id: upload_id,
            screenshot: None,
            work: upload_work,
        });
    }

    pub(super) async fn on_bridge_capture_upload(
        &mut self,
        ticket: super::ticketed::WorkTicket,
        request_id: String,
        capture: PendingCapture,
        outcome: CaptureUploadOutcome,
    ) -> Result<()> {
        let Some(upload) = self.features.sources.bridge_upload.as_mut() else {
            return Ok(());
        };
        match upload.work.settle(&ticket, self.state.generation()) {
            super::ticketed::Settlement::Current(()) => {}
            super::ticketed::Settlement::Foreign => return Ok(()),
            super::ticketed::Settlement::Expired {
                started_generation,
                current_generation,
            } => {
                if let Some(log) = &self.diagnostics {
                    let started = started_generation.to_string();
                    let current = current_generation.to_string();
                    log.record(
                        "bridge_capture_upload_stale",
                        &[
                            ("cause", "generation_changed"),
                            ("started_generation", &started),
                            ("current_generation", &current),
                        ],
                    );
                }
                return Ok(());
            }
        }
        let stale_cause = match self.conversation.bridge_request.as_ref() {
            None => Some("request_released"),
            Some(ask) if ask.id != request_id => Some("request_replaced"),
            Some(ask)
                if ask.capability
                    != if matches!(capture, PendingCapture::File { .. }) {
                        orbit_core::device_bridge::Capability::FilesRead
                    } else {
                        orbit_core::device_bridge::Capability::Screenshot
                    } =>
            {
                Some("capability_changed")
            }
            Some(_)
                if self
                    .features
                    .sources
                    .bridge_upload
                    .as_ref()
                    .is_none_or(|upload| upload.capture_id != capture.id()) =>
            {
                Some("upload_replaced")
            }
            Some(_) => None,
        };
        if let Some(cause) = stale_cause {
            if let Some(log) = &self.diagnostics {
                log.record("bridge_capture_upload_stale", &[("cause", cause)]);
            }
            return Ok(());
        }
        let Some(ask) = self.conversation.bridge_request.clone() else {
            return Ok(());
        };
        let from_engine = matches!(&capture, PendingCapture::Engine { .. });
        if let (
            PendingCapture::File { capture_id, result },
            CaptureUploadOutcome::Uploaded(sha256),
        ) = (&capture, &outcome)
        {
            let mut result = (**result).clone();
            if let orbit_core::wire::bridge::FileContent::Image {
                capture_id: id,
                sha256: digest,
                ..
            } = &mut result.content
            {
                id.clone_from(capture_id);
                digest.clone_from(sha256);
            }
            self.features.sources.bridge_upload.take();
            let sent = self.send_frame(hub::command(
                &ask.id,
                "device.reply",
                result_payload(&ask.id, BridgeResult::FilesRead(result)),
            ));
            self.note_bridge_send(&sent, "the folder image");
            self.conversation.bridge_request = None;
            return Ok(());
        }
        let screenshot = match (capture, outcome) {
            (_, CaptureUploadOutcome::TimedOut { waited_ms }) => {
                self.release_bridge_request(BridgeReleaseReason::CaptureUploadTimeout {
                    waited_ms,
                    deadline_ms: CAPTURE_UPLOAD_TIMEOUT.duration().as_millis() as u64,
                });
                return Ok(());
            }
            (_, CaptureUploadOutcome::Failed(error)) => {
                return self.refuse_capture_upload(&ask, Some(&error)).await;
            }
            (
                PendingCapture::Engine {
                    capture_id,
                    application,
                    title,
                    width,
                    height,
                    mime_type,
                },
                CaptureUploadOutcome::Uploaded(sha256),
            ) => {
                // This full screen is what Deimos now sees, so its
                // scale is the one its desktop actions are mapped with.
                let desktop = self
                    .features
                    .computer
                    .pending_desktop
                    .take_if(|(pending, _)| *pending == capture_id)
                    .map(|(_, scale)| scale);
                if let Some(scale) = desktop {
                    self.features.computer.desktop_scale = Some(scale);
                }
                ScreenshotResult {
                    capture_id,
                    application,
                    title,
                    width,
                    height,
                    mime_type,
                    sha256,
                    coordinates: desktop.map(|scale| scale.describe()),
                }
            }
            (PendingCapture::File { .. }, CaptureUploadOutcome::Uploaded(_)) => return Ok(()),
            (PendingCapture::Helper { source_id }, CaptureUploadOutcome::Uploaded(sha256)) => {
                let Some(screenshot) = self
                    .features
                    .sources
                    .bridge_upload
                    .as_ref()
                    .and_then(|upload| upload.screenshot.as_ref())
                    .filter(|screenshot| {
                        ask.source_id.as_deref() == Some(source_id.as_str())
                            && screenshot.capture_id == source_id
                            && screenshot.sha256 == sha256
                    })
                    .cloned()
                else {
                    return self
                        .refuse_bridge_request(&ask, "source_capture_failed")
                        .await;
                };
                screenshot
            }
        };
        self.features.sources.bridge_upload.take();
        let subject = if from_engine {
            format!(
                "the engine observation: {} · {}",
                screenshot.application, screenshot.title
            )
        } else {
            format!(
                "the screen read: {} · {}",
                screenshot.application, screenshot.title
            )
        };
        let sent = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(&ask.id, BridgeResult::Screenshot(screenshot)),
        ));
        self.note_bridge_send(&sent, &subject);
        self.conversation.bridge_request = None;
        Ok(())
    }

    pub(super) fn finish_remote_computer_action(&mut self, operation_id: &str, receipt: &Value) {
        let Some(ask) = self
            .conversation
            .bridge_request
            .as_ref()
            .filter(|ask| {
                ask.capability == orbit_core::device_bridge::Capability::PerformAction
                    && ask.tool_call.is_some()
            })
            .cloned()
        else {
            return;
        };
        let tool = ask
            .tool_call
            .as_ref()
            .map_or("tool_call", |call| call.tool.as_str());
        // The action was attempted once and is never repeated when its receipt
        // cannot enter the queue. Queue admission is not a Deimos receipt.
        let result = self.send_frame(hub::command(
            &ask.id,
            "device.reply",
            result_payload(
                &ask.id,
                BridgeResult::PerformAction(PerformActionResult {
                    receipt: ActionReceipt {
                        operation_id: operation_id.to_owned(),
                        tool: tool.to_owned(),
                        state: "performed".into(),
                        attempted: true,
                        driver_receipt: receipt.clone(),
                        retry: "no_repetir".into(),
                    },
                }),
            ),
        ));
        self.note_bridge_send(&result, "the receipt of the engine's single attempt");
        self.conversation.bridge_request = None;
    }

    async fn refuse_capture_upload(
        &mut self,
        ask: &BridgeAsk,
        error: Option<&anyhow::Error>,
    ) -> Result<()> {
        if let Some(mut upload) = self.features.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        let refusal = error
            .and_then(|error| error.downcast_ref::<orbit_core::refusal::Refusal>())
            .cloned()
            .unwrap_or_else(|| Self::bridge_refusal_code("capture_upload_failed"));
        self.say(bridge_refused(&refusal));
        self.conversation.bridge_request = None;
        self.send(hub::command(
            &ask.id,
            "device.reply",
            self.bridge_refusal_frame(&ask.id, &refusal),
        )?)
        .await
    }

    async fn refuse_bridge_request(&mut self, ask: &BridgeAsk, code: &str) -> Result<()> {
        if let Some(mut upload) = self.features.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        let source_failure = self
            .features
            .sources
            .last_failure
            .clone()
            .filter(|failure| {
                (code == "source_inventory_failed" && ask.source_id.is_none())
                    || (code == "source_capture_failed"
                        && ask.source_id.as_deref() == Some(failure.operation_id.as_str()))
            });
        let selection = (code == "window_not_found" || code == "screen_selection_unavailable")
            .then_some(self.state.views.source_inventory.as_ref())
            .flatten()
            .map(|inventory| {
                use orbit_core::refusal::{
                    Expected, Parameter, Refusal, SelectionFacts, SelectionFailure, SelectionWindow,
                };
                let matching = inventory
                    .windows
                    .iter()
                    .filter(|window| {
                        ask.matches_window_filter(
                            &window.application_name,
                            Some(&window.bundle_id),
                            &window.title,
                        )
                    })
                    .count();
                let mut sample = Vec::new();
                // Preserve the former 400-character inventory detail budget:
                // it bounds only the sample; the complete counts travel separately.
                for window in &inventory.windows {
                    sample.push(SelectionWindow {
                        application: window.application_name.clone(),
                        title: window.title.clone(),
                        unavailable_reason: window.unavailable_reason.clone(),
                        has_binding: window.binding.is_some(),
                    });
                    if serde_json::to_string(&sample)
                        .map_or(true, |text| text.chars().count() > MAX_BRIDGE_DETAIL_CHARS)
                    {
                        sample.pop();
                        break;
                    }
                }
                // Filter copies share the former detail's character budget.
                // Original byte counts remain exact even when the copy is short.
                let mut remaining = MAX_BRIDGE_DETAIL_CHARS;
                let mut shortened = false;
                let mut bounded_filter = |filter: &Option<String>| {
                    filter.as_ref().map(|text| {
                        let kept: String = text.chars().take(remaining).collect();
                        remaining -= kept.chars().count();
                        shortened |= kept.len() != text.len();
                        kept
                    })
                };
                let application = bounded_filter(&ask.application);
                let title = bounded_filter(&ask.title);
                Refusal::InvalidParameter {
                    parameter: Parameter::Selection,
                    expected: Expected::Selection(Box::new(SelectionFacts {
                        reason: if code == "window_not_found" {
                            SelectionFailure::WindowNotFound
                        } else {
                            SelectionFailure::Unavailable
                        },
                        application_bytes: ask.application.as_ref().map(|s| s.len() as u64),
                        title_bytes: ask.title.as_ref().map(|s| s.len() as u64),
                        filters_truncated: shortened,
                        application,
                        title,
                        windows: inventory.windows.len() as u64,
                        matching: matching as u64,
                        selectable: 0,
                        omitted: inventory.omitted_count,
                        inventory: sample,
                    })),
                }
            });
        let refusal = source_failure
            .as_ref()
            .map(|failure| failure.refusal.clone())
            .or(selection)
            .unwrap_or_else(|| Self::bridge_refusal_code(code));
        self.say(bridge_refused(&refusal));
        self.conversation.bridge_request = None;
        let payload = self.bridge_refusal_frame(&ask.id, &refusal);
        self.send(hub::command(&ask.id, "device.reply", payload)?)
            .await
    }

    /// Frees the bridge slot when the operation that served it ended outside
    /// the helper's own answer.
    ///
    /// Every ending listed by the spec calls this: timeout, cancellation,
    /// privacy stop, hub disconnect, generation change. If the hub is still
    /// reachable the caller is told why with the same wire shape the
    /// rejection path already uses; if the hub is gone, the local notice names
    /// the failed delivery. Either way the slot is left as `None` so the next remote
    /// request is admitted rather than told the bridge is busy.
    pub(super) fn release_bridge_request(&mut self, reason: BridgeReleaseReason<'_>) {
        if let Some(mut upload) = self.features.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        let Some(ask) = self.conversation.bridge_request.take() else {
            return;
        };
        let attempted_action = (ask.capability
            == orbit_core::device_bridge::Capability::PerformAction)
            .then(|| {
                self.features
                    .computer
                    .active_general
                    .as_ref()
                    .map(|entry| entry.operation_id.clone())
            })
            .flatten();
        // «Adjuntar ventana» is answered to the app, not to the hub (its reply
        // is diverted in `send_frame`): without the hub it is told why here.
        if ask.for_attachment && self.state.connection() == Connection::Offline {
            self.refuse_window(if self.state.privacy_stopped() {
                orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None }
            } else {
                orbit_core::refusal::Refusal::HubUnreachable { since_ms: None }
            });
            return;
        }
        if self.state.privacy_stopped()
            && self.state.connection() == Connection::Offline
            && !self.hub.is_some()
        {
            self.note_bridge_send(
                &hub_link::SendResult::Retired(hub_link::Retirement::PrivacyStop),
                "the remote request's refusal",
            );
            return;
        }
        // The disconnect handler sets Offline before releasing its request.
        // The user's privacy stop also sets Offline, but after cutting capture
        // it may still deliver this final refusal on the open link before
        // retiring it.
        if self.state.connection() == Connection::Offline
            && !(matches!(reason, BridgeReleaseReason::PrivacyStopped) && self.hub.is_some())
        {
            self.note_bridge_send(
                &hub_link::SendResult::Disconnected,
                "the remote request's refusal",
            );
            return;
        }
        let source_failure = self
            .features
            .sources
            .last_failure
            .as_ref()
            .filter(|failure| {
                matches!(reason, BridgeReleaseReason::Cancelled)
                    && (ask.source_id.as_deref() == Some(failure.operation_id.as_str())
                        || (ask.source_id.is_none()
                            && self
                                .state
                                .views
                                .source_request
                                .as_ref()
                                .is_some_and(|progress| progress.id == failure.operation_id)))
            });
        let interruption = reason.refusal();
        let attempted = attempted_action.is_some();
        let refusal = if let Some(operation_id) = attempted_action {
            orbit_core::refusal::Refusal::OutcomeUnknown { operation_id }
        } else {
            source_failure.map_or_else(|| interruption.clone(), |failure| failure.refusal.clone())
        };
        let mut reply = DeviceReplyParams::refused(ask.id.clone(), refusal, None);
        if attempted && let Some(ref mut fields) = reply.refusal {
            fields.interruption = Some(Box::new(interruption));
        }
        let frame = hub::command(&ask.id, "device.reply", json!(reply));
        // `send_or_report` rather than `self.send`: this path runs after
        // `privacy_stopped` is already true, and a refusal is the answer the
        // caller is owed rather than a request that should be rebuffed.
        let result = self.send_frame(frame);
        let subject = if matches!(reason, BridgeReleaseReason::PrivacyStopped) {
            "the remote request's refusal (privacy_stopped)"
        } else {
            "the remote request's refusal"
        };
        self.note_bridge_send(&result, subject);
        if matches!(reason, BridgeReleaseReason::CaptureUploadTimeout { .. }) {
            self.say(
                Notice::new(NoticeCode::BridgeRefused)
                    .with_token(NoticeKey::Code, "capture_upload_timeout"),
            );
        }
    }

    /// Answers a remote assistant asking this Mac to observe or act.
    ///
    /// Every reachable path replies: a request that is silently dropped leaves
    /// the assistant waiting on a Mac that already decided, and "no answer"
    /// reads as a broken bridge rather than as the refusal it is.
    ///
    /// One case is deliberately unreachable. While privacy is latched
    /// `hub_event` returns before dispatching anything at all, so a stopped Mac
    /// says nothing rather than saying no. That invariant is older and stronger
    /// than this bridge — a latched Mac is silent to the hub, full stop — and
    /// punching a hole in it for a politer error would be a bad trade. Naming
    /// the caller's timeout as the answer is the hub's job, not this one's.
    pub(super) async fn device_request_raw(&mut self, raw: &Value) -> Result<()> {
        use orbit_core::wire::bridge::DeviceRequestData;

        let Ok(header) = HubRequestHeader::deserialize(raw) else {
            // Nothing to reply to: without an id the answer has no destination.
            return Ok(());
        };
        let request_id = header.request_id;
        let capability = match orbit_core::device_bridge::admits(
            header
                .capability
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or(""),
            self.bridge_conditions(),
        ) {
            Ok(capability) => capability,
            Err(refusal) => {
                self.refuse_bridge_admission(request_id, &refusal).await?;
                return Ok(());
            }
        };
        let encoded = serde_json::to_vec(raw)?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        let typed: DeviceRequestData = match serde_path_to_error::deserialize(&mut decoder) {
            Ok(request) => request,
            Err(_) => {
                let refusal = unknown_capture_filter(raw, capability)
                    .unwrap_or_else(|| invalid_parameter_refusal(raw, capability));
                self.refuse_bridge_admission(request_id, &refusal).await?;
                return Ok(());
            }
        };
        self.device_request(&typed).await
    }

    fn bridge_conditions(&self) -> orbit_core::device_bridge::Conditions {
        orbit_core::device_bridge::Conditions {
            privacy_stopped: self.state.privacy_stopped(),
            // What macOS grants, as the helper last reported it and
            // as the daemon asks it again every few seconds. Nothing in Orbit's
            // settings is read: there is no switch left to consult.
            helper_connected: self.state.capabilities().native_helper(),
            accessibility: self.state.capabilities().accessibility_state(),
            screen_recording: self.state.capabilities().screen_state(),
            // Read from the live permission the Mac reports, not from
            // a switch in Orbit. This is the only decision the user makes about
            // their calendar, so it is the only thing consulted here. In
            // three values — a permission the Mac has not reported yet is
            // neither granted nor refused.
            calendar: orbit_core::device_bridge::CalendarPermission::from_authorization(
                self.features.calendar.view.authorization,
            ),
        }
    }

    async fn refuse_bridge_admission(
        &mut self,
        request_id: &str,
        refusal: &orbit_core::refusal::Refusal,
    ) -> Result<()> {
        self.say(bridge_refused(refusal));
        self.send(hub::command(
            request_id,
            "device.reply",
            self.bridge_refusal_frame(request_id, refusal),
        )?)
        .await
    }

    pub(super) async fn device_request(
        &mut self,
        data: &orbit_core::wire::bridge::DeviceRequestData,
    ) -> Result<()> {
        use orbit_core::wire::bridge::BridgeRequest;

        let request_id = data.request_id.clone();
        let name = match &data.request {
            BridgeRequest::Screenshot { .. } => "screenshot",
            BridgeRequest::ScreenSources { .. } => "screen_sources",
            BridgeRequest::SelectedText { .. } => "selected_text",
            BridgeRequest::ControlTools { .. } => "control_tools",
            BridgeRequest::PerformAction { .. } => "perform_action",
            BridgeRequest::CalendarRead { .. } => "calendar_read",
            BridgeRequest::FilesList { .. } => "files_list",
            BridgeRequest::FilesSearch { .. } => "files_search",
            BridgeRequest::FilesRead { .. } => "files_read",
        };
        let capability = match orbit_core::device_bridge::admits(name, self.bridge_conditions()) {
            Err(refusal) => {
                self.refuse_bridge_admission(&request_id, &refusal).await?;
                return Ok(());
            }
            Ok(capability) => capability,
        };
        let files = match super::assistant_files::FilesAsk::from_request(&data.request) {
            Ok(files) => files,
            Err(refusal) => {
                self.refuse_bridge_admission(&request_id, &refusal).await?;
                return Ok(());
            }
        };
        let (
            application,
            title,
            full_screen,
            tool_call,
            catalogue_after,
            calendar_window,
            calendar_cursor,
        ) = match data.request.clone() {
            BridgeRequest::Screenshot { parameters } => (
                parameters.application,
                parameters.title,
                parameters.full_screen.unwrap_or(false),
                None,
                0,
                None,
                None,
            ),
            BridgeRequest::ScreenSources { .. } => (None, None, false, None, 0, None, None),
            BridgeRequest::SelectedText { parameters } => (
                parameters.application,
                parameters.title,
                false,
                None,
                0,
                None,
                None,
            ),
            BridgeRequest::ControlTools { parameters } => (
                None,
                None,
                false,
                None,
                parameters.after_index.unwrap_or(0),
                None,
                None,
            ),
            BridgeRequest::PerformAction { parameters } => {
                if parameters.action.validate().is_err() {
                    let action = &parameters.action;
                    let arguments_type = match &action.arguments {
                        Value::Null => "null",
                        Value::Bool(_) => "boolean",
                        Value::Number(_) => "number",
                        Value::String(_) => "string",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                    };
                    let refusal = orbit_core::refusal::Refusal::InvalidParameter {
                        parameter: orbit_core::refusal::Parameter::Request,
                        expected: orbit_core::refusal::Expected::ToolCall {
                            tool_bytes: action.tool.len() as u64,
                            max_tool_bytes: orbit_core::computer_control::MAX_TOOL_NAME_BYTES
                                as u64,
                            arguments_type: arguments_type.into(),
                            bytes: serde_json::to_vec(action).ok().map(|v| v.len() as u64),
                            max_bytes: orbit_core::limits::MAX_ACTION_BYTES as u64,
                        },
                    };
                    self.refuse_bridge_admission(&request_id, &refusal).await?;
                    return Ok(());
                }
                (None, None, false, Some(parameters.action), 0, None, None)
            }
            BridgeRequest::CalendarRead { parameters } => {
                if parameters.end_ms <= parameters.start_ms {
                    let refusal = orbit_core::refusal::Refusal::InvalidParameter {
                        parameter: orbit_core::refusal::Parameter::Request,
                        expected: orbit_core::refusal::Expected::CalendarRange {
                            start_ms: parameters.start_ms,
                            end_ms: parameters.end_ms,
                            min_ms: orbit_core::calendar::MIN_UNIX_MS,
                            max_ms: orbit_core::calendar::MAX_UNIX_MS,
                        },
                    };
                    self.refuse_bridge_admission(&request_id, &refusal).await?;
                    return Ok(());
                }
                let cursor = match super::calendar::Cursor::from_parameters(&parameters) {
                    Ok(cursor) => cursor,
                    Err(refusal) => {
                        self.refuse_bridge_admission(&request_id, &refusal).await?;
                        return Ok(());
                    }
                };
                (
                    None,
                    None,
                    false,
                    None,
                    0,
                    Some((parameters.start_ms, parameters.end_ms)),
                    cursor,
                )
            }
            BridgeRequest::FilesList { .. }
            | BridgeRequest::FilesSearch { .. }
            | BridgeRequest::FilesRead { .. } => (None, None, false, None, 0, None, None),
        };
        if self.conversation.bridge_request.is_some() {
            let refusal = orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::Interaction,
                retry_after_ms: None,
            };
            self.say(bridge_refused(&refusal));
            self.send(hub::command(
                &request_id,
                "device.reply",
                self.bridge_refusal_frame(&request_id, &refusal),
            )?)
            .await?;
            return Ok(());
        }
        let ask = BridgeAsk {
            for_attachment: false,
            id: request_id.clone(),
            capability,
            application,
            title,
            full_screen,
            source_id: None,
            tool_call,
            catalogue_after,
            calendar_window,
            calendar_cursor,
            files,
        };
        self.conversation.bridge_request = Some(ask.clone());
        // What Deimos asked for, said while it is served: Deimos never looks
        // at this Mac quietly.
        let asked = match capability {
            orbit_core::device_bridge::Capability::Screenshot if ask.full_screen => {
                "screenshot_full"
            }
            orbit_core::device_bridge::Capability::Screenshot => "screenshot_window",
            orbit_core::device_bridge::Capability::ScreenSources => "screen_sources",
            orbit_core::device_bridge::Capability::SelectedText => "selected_text",
            orbit_core::device_bridge::Capability::ControlTools => "control_tools",
            orbit_core::device_bridge::Capability::PerformAction => "perform_action",
            orbit_core::device_bridge::Capability::CalendarRead => "calendar_read",
            orbit_core::device_bridge::Capability::FilesList => "files_list",
            orbit_core::device_bridge::Capability::FilesSearch => "files_search",
            orbit_core::device_bridge::Capability::FilesRead => "files_read",
        };
        // A start that fails is answered here and now. Leaving the request open
        // would spend the caller's whole timeout on a Mac that already knows it
        // will not be answering.
        let started = self.serve_bridge_request(&ask).await;
        // Written AFTER starting the work, never before. The local operation may
        // set this same field while it starts.
        match started {
            Ok(()) => {
                self.say(Notice::new(NoticeCode::BridgeServing).with_token(NoticeKey::Code, asked));
            }
            Err(failure) => {
                self.conversation.bridge_request = None;
                self.log("bridge_start_failed", &[("code", failure.code().name())]);
                self.say(
                    Notice::new(NoticeCode::BridgeRefused)
                        .with_token(NoticeKey::Code, failure.code().name()),
                );
                self.send(hub::command(
                    &request_id,
                    "device.reply",
                    self.bridge_start_frame(&request_id, &failure),
                )?)
                .await?;
            }
        }
        Ok(())
    }
}

/// A refusal of what Deimos asked, said on the Mac with its code only:
/// Deimos reads the full sentence in its reply and tells the user.
fn bridge_refused(refusal: &orbit_core::refusal::Refusal) -> Notice {
    Notice::new(NoticeCode::BridgeRefused).with_token(NoticeKey::Code, refusal.code().name())
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn both_upload_paths_stop_at_the_bridge_deadline() {
        let outcome = capture_upload_outcome(std::future::pending::<Result<String>>()).await;
        let CaptureUploadOutcome::TimedOut { waited_ms } = outcome else {
            panic!("the upload did not expire with the bridge deadline");
        };
        assert_eq!(waited_ms, 20_000);
    }

    #[test]
    fn source_command_failure_says_it_was_local_and_keeps_the_cause() -> anyhow::Result<()> {
        let Err(error) = crate::native::command("screen_sources", json!({"id":""})) else {
            anyhow::bail!("empty native command ID was accepted");
        };
        let failure = source_start_refusal(super::super::sources::SourceStartError::NativeCommand(
            error,
        ));
        assert_eq!(failure.code().name(), "internal");
        let refusal = serde_json::to_value(&failure)?;
        assert_eq!(refusal["code"], "internal");
        assert_eq!(refusal["cause"]["kind"], "native_command");
        Ok(())
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
