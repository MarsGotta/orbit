//! Shared, typed reasons for refusing an operation. Decisions depend on facts
//! supplied by callers; this module never reads the clock or performs effects.
use crate::{CapturePhase, PermissionState, computer_control::engine::SystemPermissions};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "ms", rename_all = "snake_case")]
pub enum Retry {
    Never,
    AfterFix,
    AfterMs(u64),
    CheckOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceReplyRetry {
    AfterFix,
    CheckOutcome,
}

macro_rules! parameter_enum {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export_to = "../../apps/desktop/src/bridge/generated/"))]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $(Self::$variant => $label),+ })
            }
        }
    };
}
parameter_enum!(Permission { Microphone => "microphone", ScreenRecording => "screen_recording", Accessibility => "accessibility", ScreenAndAccessibility => "screen_and_accessibility", Calendar => "calendar", Automation => "automation" });

/// The one name for a set of missing system permissions, shared by the bridge
/// and the control topic. `grant` never reports an empty set; «both» is the
/// remaining real combination and the honest name for anything else built.
pub fn missing_permission_name(missing: SystemPermissions) -> Permission {
    match (missing.accessibility, missing.screen_recording) {
        (true, false) => Permission::Accessibility,
        (false, true) => Permission::ScreenRecording,
        (true, true) | (false, false) => Permission::ScreenAndAccessibility,
    }
}

/// Both the remote bridge and the control topic name the same macOS decision.
/// A permission still awaiting its first grant is not one macOS denied.
pub fn missing_system_permission_refusal(
    missing: SystemPermissions,
    accessibility: PermissionState,
    screen_recording: PermissionState,
) -> Refusal {
    let permission = missing_permission_name(missing);
    if (missing.accessibility && accessibility == PermissionState::NotGranted)
        || (missing.screen_recording && screen_recording == PermissionState::NotGranted)
    {
        Refusal::PermissionNotGranted { permission }
    } else {
        Refusal::PermissionDenied { permission }
    }
}
parameter_enum!(Resource { Subscriptions => "subscriptions", IpcResponse => "ipc_response", IpcRequest => "ipc_request", HubResponse => "hub_response", Message => "message", Recording => "recording", Session => "session", Audio => "audio", Calendar => "calendar", Source => "source", Capture => "capture", ComputerAction => "computer_action", Storage => "storage", Folder => "folder", File => "file" });
parameter_enum!(Operation { IpcRequest => "ipc_request", Subscription => "subscription", Chat => "chat", SessionList => "session_list", Interaction => "interaction", Recording => "recording", Audio => "audio", AudioSession => "audio_session", AudioDrain => "audio_drain", Capture => "capture", Playback => "playback", Call => "call", Calendar => "calendar", ComputerAction => "computer_action", Storage => "storage", Pairing => "pairing", VoiceTranscription => "voice_transcription", PrivacyStop => "privacy_stop", Files => "files" });
parameter_enum!(Parameter { Text => "text", Duration => "duration", Title => "title", ApplicationPid => "application_pid", Request => "request", Selection => "selection", Identifier => "identifier", Meeting => "meeting" });
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expected {
    WireType {
        name: String,
    },
    CalendarEvent {
        position: u64,
        rows: u64,
    },
    DeclaredTool {
        received: String,
    },
    DesktopFrame,
    DesktopCoordinate {
        axis: String,
        received: Option<serde_json::Number>,
        image: u32,
        screen: u32,
        image_width: u32,
        image_height: u32,
        upper_inclusive: bool,
    },
    DesktopFit {
        cause: crate::sources::DesktopFitError,
    },
    SignedRange {
        received: i64,
        min: i64,
        max: i64,
    },
    ReadDuration {
        read_ns: u64,
        max_exclusive_ns: u64,
    },
    IpcPayload {
        bytes: u64,
        line: u64,
        column: u64,
        syntax: bool,
        error: String,
    },
    KnownHelperCode {
        received: String,
        bytes: u64,
        max_bytes: u64,
    },
    UnsignedRange {
        received: u64,
        min: u64,
        max: u64,
    },
    ReadTimes {
        started_ns: u64,
        completed_ns: u64,
    },
    ClockDomain {
        received: String,
    },
    CurrentGeneration {
        received: u64,
        required: Option<u64>,
        instance_matches: bool,
    },
    WireField {
        field: String,
        expected: String,
        received_type: String,
    },
    SelectionFilters {
        unknown: String,
        allowed: Vec<String>,
    },
    Selection(Box<SelectionFacts>),
    FullScreenFilters {
        application: bool,
        title: bool,
    },
    ToolCall {
        tool_bytes: u64,
        max_tool_bytes: u64,
        arguments_type: String,
        bytes: Option<u64>,
        max_bytes: u64,
    },
    CalendarRange {
        start_ms: i64,
        end_ms: i64,
        min_ms: i64,
        max_ms: i64,
    },
    /// A path for the assistant's folders: absolute, without NUL, at most
    /// `assistant_files::MAX_PATH_BYTES`.
    AbsolutePath {
        bytes: u64,
        max_bytes: u64,
    },
    /// A search text: at least one character, at most
    /// `assistant_files::MAX_QUERY_BYTES`.
    SearchQuery {
        bytes: u64,
        max_bytes: u64,
    },
    CalendarCursor {
        read_id_bytes: Option<u64>,
        max_read_id_bytes: u64,
        after_ms: bool,
        after_index: bool,
        after_invitee: bool,
    },
    NonEmpty,
    ValidIdentifier,
    ValidSelection,
    ValidApplication,
    TextLength {
        bytes: u64,
        max_bytes: u64,
    },
    /// A meeting title over the manifest maximum. Counted in
    /// characters, the unit `MAX_MANIFEST_TITLE_CHARACTERS` is defined in,
    /// unlike the byte-based `TextLength` above.
    TitleLength {
        characters: u64,
        max_characters: u64,
    },
    DurationRange {
        actual_ms: i64,
        min_ms: i64,
        max_ms: i64,
    },
    OpenRecording,
    CorrectCaptureState,
    ValidMeeting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionFacts {
    pub reason: SelectionFailure,
    pub application_bytes: Option<u64>,
    pub title_bytes: Option<u64>,
    pub filters_truncated: bool,
    pub application: Option<String>,
    pub title: Option<String>,
    pub windows: u64,
    pub matching: u64,
    pub selectable: u64,
    pub omitted: u64,
    pub inventory: Vec<SelectionWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFailure {
    WindowNotFound,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionWindow {
    pub application: String,
    pub title: String,
    pub unavailable_reason: Option<String>,
    pub has_binding: bool,
}

parameter_enum!(Service { Hub => "hub", Provider => "provider", NativeHelper => "native_helper" });
parameter_enum!(HubSessionField { Id => "id", Status => "status" });
parameter_enum!(EngineCause { NotInstalled => "not_installed", NotRunning => "not_running", Unhealthy => "unhealthy", Unsupported => "unsupported", VoiceProcessingUnsupported => "voice_processing_unsupported", InitializationFailed => "initialization_failed" });

/// The existing size, upstream, timeout and parameter families cannot carry a
/// download URL, declared-versus-observed size, available manifest keys and
/// network cause together. Updates therefore have one typed family; the CLI
/// maps each cause to the corresponding exit-code family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateSizeSource {
    Declared,
    Observed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateNetworkCause {
    Dns,
    ConnectionRefused,
    Tls,
    Other { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallLogIssue {
    TooLarge,
    MissingResult { started_at: Option<String> },
    Io { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateCause {
    TooLarge {
        file: String,
        bytes: u64,
        max_bytes: u64,
        source: UpdateSizeSource,
    },
    HttpStatus {
        url: String,
        status: u16,
    },
    Deadline {
        url: String,
        waited_ms: u64,
        deadline_ms: u64,
    },
    NoResponse {
        url: String,
        cause: UpdateNetworkCause,
    },
    InvalidUtf8 {
        file: String,
        byte: usize,
    },
    MissingKey {
        key: String,
        available_keys: Vec<String>,
    },
    InvalidVersion {
        value: String,
    },
    InvalidFile {
        key: String,
        value: String,
    },
    InvalidDigest {
        key: String,
        value: String,
    },
    InstallerDigestMismatch {
        expected: String,
        actual: String,
    },
    InstallLog {
        bytes: Option<u64>,
        max_bytes: u64,
        reason: InstallLogIssue,
    },
    InvalidOrigin {
        bytes: usize,
    },
    InvalidRedirect {
        url: String,
        cause: String,
    },
    RedirectOutsideOrigin {
        from_origin: String,
        target_origin: String,
    },
    RedirectLimit {
        url: String,
        followed: usize,
        max_redirects: usize,
    },
    ClientBuild {
        cause: String,
    },
    UnsupportedArchitecture {
        value: String,
    },
    InstallerFailed {
        status: Option<i32>,
    },
    InstallerIo {
        cause: String,
    },
}

/// Which of the attachment ceilings a refusal names (`attachments`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentCeiling {
    File,
    Message,
    Session,
}

/// Why a file cannot be attached or a turn cannot carry its files. Each
/// carries the file's name and the numbers of the rule that fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttachmentCause {
    /// The hub has no attachments (0.9.0): nothing is read or sent.
    Unsupported,
    TooLarge {
        ceiling: AttachmentCeiling,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<u64>,
        bytes: u64,
        /// What the conversation already holds, for the session ceiling.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_bytes: Option<u64>,
        max_bytes: u64,
    },
    TooMany {
        files: u64,
        max_files: u64,
    },
    /// No agent reads this type; `mime` is what its content looks like.
    TypeUnsupported {
        name: String,
        mime: String,
    },
    /// The file could not be read: on this Mac (`missing`, `permission`,
    /// `not_a_file`, `io`), or by the hub (`pdf_locked`, `pdf_damaged`,
    /// `render_failed`).
    Unreadable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        reason: String,
    },
    /// The agent refused it (Hermes's `image.attach_bytes` code).
    Refused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        upstream_code: Option<i64>,
    },
    /// The hub no longer has it: never sent within the hour, or sent already.
    NotFound {
        attachment_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalendarReadLimit {
    Page {
        max_bytes: u64,
    },
    Fields {
        max_title_bytes: u64,
        max_label_bytes: u64,
        max_timezone_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InternalCause {
    NativeCommand,
    ControlHistory,
    ComputerStart,
    DaemonOperation { error: String },
    AppOperation { code: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StorageCause {
    Detail(String),
    PrivacyStop {
        error: String,
        step: crate::RecoveryStep,
        elapsed_ms: u64,
    },
}
impl From<String> for StorageCause {
    fn from(value: String) -> Self {
        Self::Detail(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadInterruption {
    Authority {
        change: crate::authority::AuthorityChange,
    },
    Cancelled,
    InventoryDispatch {
        sent_reads: u64,
        total_reads: u64,
    },
    GenerationChanged {
        started_generation: u64,
        current_generation: u64,
    },
    ConversationChanged,
    HelperRestarted {
        previous_generation: Option<u64>,
        current_generation: Option<u64>,
        previous_instance: Option<String>,
        current_instance: Option<String>,
    },
    CalendarChanged {
        changes: u64,
        max_restarts: u64,
    },
    CalendarHelperStopped,
    CalendarReadRestarted,
    CalendarGenerationChanged,
    CalendarReplyReplaced,
    SourceReplyReplaced,
    CaptureStopped,
    ComputerRevoked,
    CalendarExpired {
        deadline_ms: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonReplyIssue {
    Missing,
    Decode { error: String },
    CursorRepeated,
    CountMismatch { delivered: u64, total: u64 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum HelperCode {
    AccessibilityPermissionRequired,
    CalendarBusy,
    CalendarClockUnavailable,
    CalendarEventTooLarge,
    CalendarInvalidated,
    CalendarLimitExceeded,
    CalendarPermissionRequired,
    CalendarReadExpired,
    CalendarReadFailed,
    CalendarSelectionUnavailable,
    CaptureClockUnavailable,
    InvalidCalendarRequest,
    InvalidScreenInventory,
    InvalidScreenshotDimensions,
    InvalidWindowBinding,
    InvalidWindowDimensions,
    PrivacyStopped,
    ScreenInventoryBusy,
    ScreenInventoryTooLarge,
    ScreenPermissionRequired,
    ScreenSelectionExpired,
    ScreenSelectionUnavailable,
    ScreenshotBusy,
    ScreenshotExpired,
    ScreenshotSdkFailed,
    ScreenshotTooLarge,
    SelectedTextChanged,
    SelectedTextClassificationIncomplete,
    SelectedTextEmpty,
    SelectedTextExpired,
    SelectedTextIdentityChanged,
    SelectedTextInvalidRange,
    SelectedTextInvalidText,
    SelectedTextLimit,
    SelectedTextNotExposed,
    SelectedTextSecureField,
    SelectedTextUnavailable,
    SelectedTextWindowAmbiguous,
    WindowIdentityChanged,
}

macro_rules! refusals {
    ($($variant:ident => $code:literal $( { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? } )? ,)*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(tag = "code", rename_all = "snake_case")]
        pub enum Refusal { $($variant $( { $($(#[$attr])* $field: $ty),* } )?,)* }
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export_to = "../../apps/desktop/src/bridge/generated/"))]
        #[serde(rename_all = "snake_case")]
        pub enum RefusalCode { $($variant,)* }
        impl RefusalCode {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
            pub fn name(self) -> &'static str { match self { $(Self::$variant => $code,)* } }
        }
        impl Refusal {
            pub fn code(&self) -> RefusalCode { match self { $(Self::$variant { .. } => RefusalCode::$variant,)* } }
        }
    };
}

refusals! {
    Attachment => "attachment" { cause: AttachmentCause },
    CalendarReadLimit => "calendar_read_limit" { cause: CalendarReadLimit },
    EncodeFailed => "encode_failed" { resource: Resource },
    HelperReplyInvalid => "helper_reply_invalid" { field: String, expected: Expected, received_type: String },
    ReadInterrupted => "read_interrupted" { cause: ReadInterruption },
    HelperRefused => "helper_refused" { #[serde(rename = "helper_code")] code: HelperCode },
    Internal => "internal" { cause: InternalCause },
    ProtocolVersion => "protocol_version" { received: u64, required: u64 },
    HubReplyInvalid => "hub_reply_invalid" { field: String, received_bytes: Option<u64> },
    DaemonReplyInvalid => "daemon_reply_invalid" { field: String, issue: DaemonReplyIssue },
    Recovering => "recovering" {
        step: crate::RecoveryStep,
        elapsed_ms: u64,
    },
    PrivacyStopped => "privacy_stopped" {
        since_ms: Option<i64>,
    },
    PermissionDenied => "permission_denied" {
        permission: Permission,
    },
    PermissionNotGranted => "permission_not_granted" {
        permission: Permission,
    },
    PermissionUnknown => "permission_unknown" {
        permission: Permission,
    },
    HelperNotRunning => "helper_not_running",
    CalendarBackendUnavailable => "calendar_backend_unavailable",
    DeviceReplyInvalid => "device_reply_invalid" {
        field: String,
        expected: String,
        device_version: String,
        retry: DeviceReplyRetry,
    },
    DeviceReplyTimeout => "device_reply_timeout" {
        expected: String,
        deadline_ms: u64,
    },
    NotPaired => "not_paired",
    MacDisconnected => "mac_disconnected" {
        last_seen_ms: Option<i64>,
    },
    HubUnreachable => "hub_unreachable" {
        since_ms: Option<i64>,
    },
    HubSessionsUnpaged => "hub_sessions_unpaged" {
        delivered: u64,
        total: u64,
    },
    HubSessionInvalidRow => "hub_session_invalid_row" {
        position: u64,
        rows: u64,
        field: HubSessionField,
        bytes: u64,
        max_bytes: u64,
    },
    HubSessionsCursorMismatch => "hub_sessions_cursor_mismatch" {
        rows: u64,
    },
    EngineUnavailable => "engine_unavailable" {
        cause: EngineCause,
    },
    Busy => "busy" {
        operation: Operation,
        retry_after_ms: Option<u64>,
    },
    PendingRecording => "pending_recording" {
        id: String,
        title: String,
        phase: CapturePhase,
        sealed_chunks: u64,
        gaps: u64,
        expires_at_ms: i64,
    },
    WorkInFlight => "work_in_flight" {
        in_flight: u64,
        max: u64,
    },
    PhysicalWorkFinishing => "physical_work_finishing" {
        in_flight: u64,
        max: u64,
    },
    TooLarge => "too_large" {
        resource: Resource,
        bytes: u64,
        max_bytes: u64,
    },
    TooMany => "too_many" {
        resource: Resource,
        count: u64,
        max: u64,
    },
    InvalidParameter => "invalid_parameter" {
        parameter: Parameter,
        expected: Expected,
    },
    RevisionChanged => "revision_changed" {
        expected_revision: u64,
        actual_revision: u64,
        expected_state_sha256: String,
        actual_state_sha256: String,
    },
    Timeout => "timeout" {
        operation: Operation,
        waited_ms: u64,
        deadline_ms: u64,
    },
    Expired => "expired" {
        resource: Resource,
        expired_at_ms: Option<i64>,
    },
    NotFound => "not_found" {
        resource: Resource,
    },
    OutcomeUnknown => "outcome_unknown" {
        operation_id: String,
    },
    WorkExpired => "work_expired" {
        operation: Operation,
        started_generation: u64,
        current_generation: u64,
    },
    UpstreamFailed => "upstream_failed" {
        service: Service,
        status: Option<u16>,
    },
    StorageFailed => "storage_failed" {
        operation: Operation,
        #[serde(default)]
        cause: Option<StorageCause>,
    },
    Update => "update" {
        cause: UpdateCause,
    },
    Legacy => "legacy" {
        text: String,
    },
    // The assistant asked for a file and the user has given it no folder.
    NoFolders => "no_folders",
    // The path, as asked and as it resolves on the Mac (links and `..`
    // followed), is in none of the folders. Names them all so the caller can
    // correct its path.
    OutsideFolders => "outside_folders" {
        path: String,
        resolved: Option<String>,
        folders: Vec<String>,
    },
    // The folder's bookmark does not open it, and why.
    FolderUnavailable => "folder_unavailable" {
        path: String,
        cause: crate::assistant_files::FolderCause,
    },
    // A path inside a folder that cannot be read as asked, and why.
    FileUnavailable => "file_unavailable" {
        path: String,
        cause: crate::assistant_files::FileCause,
    },
    // A PDF or an image over the size Orbit reads, with both sizes.
    FileTooLarge => "file_too_large" {
        path: String,
        content: crate::assistant_files::ContentKind,
        bytes: u64,
        max_bytes: u64,
    },
}

impl Refusal {
    /// The hub cannot be asked now: this Mac is not paired, has no link to
    /// it, or the user's privacy stop is on. What the hub keeps (notes,
    /// templates) is then unknown here, never a failure of the read: the
    /// connection and the stop say themselves elsewhere (seen in a run of the
    /// real test on the Mac).
    pub fn needs_the_hub(&self) -> bool {
        matches!(
            self,
            Self::NotPaired | Self::HubUnreachable { .. } | Self::PrivacyStopped { .. }
        )
    }

    pub fn retry(&self) -> Retry {
        match self {
            Self::PrivacyStopped { .. }
            | Self::PermissionDenied { .. }
            | Self::PermissionNotGranted { .. }
            | Self::PermissionUnknown { .. }
            | Self::HelperNotRunning
            | Self::CalendarBackendUnavailable
            | Self::DeviceReplyInvalid {
                retry: DeviceReplyRetry::AfterFix,
                ..
            }
            | Self::NotPaired
            | Self::HubSessionsUnpaged { .. }
            | Self::HubSessionInvalidRow { .. }
            | Self::HubSessionsCursorMismatch { .. }
            | Self::EngineUnavailable { .. }
            | Self::InvalidParameter { .. }
            | Self::RevisionChanged { .. }
            | Self::Expired { .. }
            | Self::NotFound { .. }
            | Self::PendingRecording { .. }
            | Self::TooMany { .. }
            | Self::Attachment { .. }
            | Self::NoFolders
            | Self::OutsideFolders { .. }
            | Self::FolderUnavailable { .. }
            | Self::FileUnavailable { .. }
            | Self::FileTooLarge { .. } => Retry::AfterFix,
            Self::Update { cause } => match cause {
                UpdateCause::InstallerFailed { .. } | UpdateCause::InstallerIo { .. } => {
                    Retry::CheckOutcome
                }
                UpdateCause::Deadline { .. }
                | UpdateCause::NoResponse { .. }
                | UpdateCause::HttpStatus { .. } => Retry::AfterFix,
                UpdateCause::TooLarge { .. }
                | UpdateCause::InvalidUtf8 { .. }
                | UpdateCause::MissingKey { .. }
                | UpdateCause::InvalidVersion { .. }
                | UpdateCause::InvalidFile { .. }
                | UpdateCause::InvalidDigest { .. }
                | UpdateCause::InstallerDigestMismatch { .. }
                | UpdateCause::InstallLog { .. }
                | UpdateCause::InvalidOrigin { .. }
                | UpdateCause::InvalidRedirect { .. }
                | UpdateCause::RedirectOutsideOrigin { .. }
                | UpdateCause::RedirectLimit { .. }
                | UpdateCause::ClientBuild { .. }
                | UpdateCause::UnsupportedArchitecture { .. } => Retry::AfterFix,
            },
            Self::MacDisconnected { .. }
            | Self::DeviceReplyInvalid {
                retry: DeviceReplyRetry::CheckOutcome,
                ..
            }
            | Self::DeviceReplyTimeout { .. }
            | Self::HubUnreachable { .. }
            | Self::UpstreamFailed { .. }
            | Self::StorageFailed { .. } => Retry::CheckOutcome,
            Self::TooLarge {
                resource: Resource::IpcResponse,
                ..
            } => Retry::CheckOutcome,
            Self::TooLarge {
                resource:
                    Resource::Subscriptions
                    | Resource::IpcRequest
                    | Resource::HubResponse
                    | Resource::Message
                    | Resource::Recording
                    | Resource::Session
                    | Resource::Audio
                    | Resource::Calendar
                    | Resource::Source
                    | Resource::Capture
                    | Resource::ComputerAction
                    | Resource::Storage
                    | Resource::Folder
                    | Resource::File,
                ..
            } => Retry::AfterFix,
            Self::Busy {
                retry_after_ms: Some(ms),
                ..
            } => Retry::AfterMs(*ms),
            Self::Busy {
                retry_after_ms: None,
                ..
            } => Retry::AfterFix,
            Self::Timeout { .. }
            | Self::OutcomeUnknown { .. }
            | Self::WorkExpired { .. }
            | Self::WorkInFlight { .. }
            | Self::PhysicalWorkFinishing { .. } => Retry::CheckOutcome,
            Self::EncodeFailed { .. }
            | Self::Internal { .. }
            | Self::Recovering { .. }
            | Self::Legacy { .. } => Retry::Never,
            Self::CalendarReadLimit { .. }
            | Self::HelperReplyInvalid { .. }
            | Self::HelperRefused { .. }
            | Self::ProtocolVersion { .. } => Retry::AfterFix,
            Self::HubReplyInvalid { .. }
            | Self::DaemonReplyInvalid { .. }
            | Self::ReadInterrupted { .. } => Retry::CheckOutcome,
        }
    }
}
impl Serialize for Refusal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = match self {
            Self::PrivacyStopped { since_ms } => {
                json!({"code":"privacy_stopped","since_ms":since_ms})
            }
            Self::PermissionDenied { permission } => {
                json!({"code":"permission_denied","permission":permission})
            }
            Self::PermissionNotGranted { permission } => {
                json!({"code":"permission_not_granted","permission":permission})
            }
            Self::PermissionUnknown { permission } => {
                json!({"code":"permission_unknown","permission":permission})
            }
            Self::HelperNotRunning => json!({"code":"helper_not_running"}),
            Self::CalendarBackendUnavailable => json!({"code":"calendar_backend_unavailable"}),
            Self::DeviceReplyInvalid {
                field,
                expected,
                device_version,
                ..
            } => {
                json!({"code":"device_reply_invalid","field":field,"expected":expected,"device_version":device_version})
            }
            Self::DeviceReplyTimeout {
                expected,
                deadline_ms,
            } => {
                json!({"code":"device_reply_timeout","expected":expected,"deadline_ms":deadline_ms})
            }
            Self::NotPaired => json!({"code":"not_paired"}),
            Self::MacDisconnected { last_seen_ms } => {
                json!({"code":"mac_disconnected","last_seen_ms":last_seen_ms})
            }
            Self::HubUnreachable { since_ms } => {
                json!({"code":"hub_unreachable","since_ms":since_ms})
            }
            Self::HubSessionsUnpaged { delivered, total } => {
                json!({"code":"hub_sessions_unpaged","delivered":delivered,"total":total})
            }
            Self::HubSessionInvalidRow {
                position,
                rows,
                field,
                bytes,
                max_bytes,
            } => {
                json!({"code":"hub_session_invalid_row","position":position,"rows":rows,"field":field,"bytes":bytes,"max_bytes":max_bytes})
            }
            Self::HubSessionsCursorMismatch { rows } => {
                json!({"code":"hub_sessions_cursor_mismatch","rows":rows})
            }
            Self::EngineUnavailable { cause } => json!({"code":"engine_unavailable","cause":cause}),
            Self::Busy {
                operation,
                retry_after_ms,
            } => json!({"code":"busy","operation":operation,"retry_after_ms":retry_after_ms}),
            Self::PendingRecording {
                id,
                title,
                phase,
                sealed_chunks,
                gaps,
                expires_at_ms,
            } => json!({"code":"pending_recording","id":id,"title":title,"phase":phase,
                    "sealed_chunks":sealed_chunks,"gaps":gaps,"expires_at_ms":expires_at_ms}),
            Self::WorkInFlight { in_flight, max } => {
                json!({"code":"work_in_flight","in_flight":in_flight,"max":max})
            }
            Self::PhysicalWorkFinishing { in_flight, max } => {
                json!({"code":"physical_work_finishing","in_flight":in_flight,"max":max})
            }
            Self::TooLarge {
                resource,
                bytes,
                max_bytes,
            } => {
                json!({"code":"too_large","resource":resource,"bytes":bytes,"max_bytes":max_bytes})
            }
            Self::TooMany {
                resource,
                count,
                max,
            } => json!({"code":"too_many","resource":resource,"count":count,"max":max}),
            Self::InvalidParameter {
                parameter,
                expected,
            } => json!({"code":"invalid_parameter","parameter":parameter,"expected":expected}),
            Self::RevisionChanged {
                expected_revision,
                actual_revision,
                expected_state_sha256,
                actual_state_sha256,
            } => {
                json!({"code":"revision_changed","expected_revision":expected_revision,"actual_revision":actual_revision,"expected_state_sha256":expected_state_sha256,"actual_state_sha256":actual_state_sha256})
            }
            Self::Timeout {
                operation,
                waited_ms,
                deadline_ms,
            } => {
                json!({"code":"timeout","operation":operation,"waited_ms":waited_ms,"deadline_ms":deadline_ms})
            }
            Self::Expired {
                resource,
                expired_at_ms,
            } => json!({"code":"expired","resource":resource,"expired_at_ms":expired_at_ms}),
            Self::NotFound { resource } => json!({"code":"not_found","resource":resource}),
            Self::OutcomeUnknown { operation_id } => {
                json!({"code":"outcome_unknown","operation_id":operation_id})
            }
            Self::WorkExpired {
                operation,
                started_generation,
                current_generation,
            } => {
                json!({"code":"work_expired","operation":operation,"started_generation":started_generation,"current_generation":current_generation})
            }
            Self::UpstreamFailed { service, status } => {
                json!({"code":"upstream_failed","service":service,"status":status})
            }
            Self::StorageFailed { operation, cause } => match cause {
                Some(cause) => json!({"code":"storage_failed","operation":operation,"cause":cause}),
                None => json!({"code":"storage_failed","operation":operation}),
            },
            Self::Update { cause } => json!({"code":"update","cause":cause}),
            Self::Recovering { step, elapsed_ms } => {
                json!({"code":"recovering","step":step,"elapsed_ms":elapsed_ms})
            }
            Self::Legacy { text } => json!({"code":"legacy","text":text}),
            Self::CalendarReadLimit { cause } => {
                json!({"code":"calendar_read_limit","cause":cause})
            }
            Self::Attachment { cause } => json!({"code":"attachment","cause":cause}),
            Self::EncodeFailed { resource } => json!({"code":"encode_failed","resource":resource}),
            Self::HelperReplyInvalid {
                field,
                expected,
                received_type,
            } => {
                json!({"code":"helper_reply_invalid","field":field,"expected":expected,"received_type":received_type})
            }
            Self::ReadInterrupted { cause } => json!({"code":"read_interrupted","cause":cause}),
            Self::HelperRefused { code } => json!({"code":"helper_refused","helper_code":code}),
            Self::Internal { cause } => json!({"code":"internal","cause":cause}),
            Self::ProtocolVersion { received, required } => {
                json!({"code":"protocol_version","received":received,"required":required})
            }
            Self::HubReplyInvalid {
                field,
                received_bytes,
            } => json!({"code":"hub_reply_invalid","field":field,"received_bytes":received_bytes}),
            Self::DaemonReplyInvalid { field, issue } => {
                json!({"code":"daemon_reply_invalid","field":field,"issue":issue})
            }
            Self::NoFolders => json!({"code":"no_folders"}),
            Self::OutsideFolders {
                path,
                resolved,
                folders,
            } => {
                json!({"code":"outside_folders","path":path,"resolved":resolved,"folders":folders})
            }
            Self::FolderUnavailable { path, cause } => {
                json!({"code":"folder_unavailable","path":path,"cause":cause})
            }
            Self::FileUnavailable { path, cause } => {
                json!({"code":"file_unavailable","path":path,"cause":cause})
            }
            Self::FileTooLarge {
                path,
                content,
                bytes,
                max_bytes,
            } => {
                json!({"code":"file_too_large","path":path,"content":content,"bytes":bytes,"max_bytes":max_bytes})
            }
        };
        if let Value::Object(ref mut fields) = value {
            fields.insert(
                "retry".into(),
                serde_json::to_value(self.retry()).map_err(serde::ser::Error::custom)?,
            );
        }
        value.serialize(serializer)
    }
}

// Error adapters require Display. Diagnostics retain the wire code and fields;
// the interface and MCP own the user-facing sentences.
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_string(self) {
            Ok(value) => f.write_str(&value),
            Err(_) => f.write_str(self.code().name()),
        }
    }
}
impl std::error::Error for Refusal {}

/// The recovery phase in the messages emitted by startup and typed refusals.
pub fn recovery_step_name(step: crate::RecoveryStep) -> &'static str {
    match step {
        crate::RecoveryStep::Chunks => "chunks",
        crate::RecoveryStep::UploadJobs => "upload_jobs",
        crate::RecoveryStep::Snapshot => "snapshot",
        crate::RecoveryStep::Environment => "environment",
        crate::RecoveryStep::Operations => "operations",
        crate::RecoveryStep::CallPreference => "call_preference",
    }
}

impl Refusal {
    pub fn helper_reply_invalid(problem: crate::wire::native::NativeDecodeProblem) -> Self {
        Self::HelperReplyInvalid {
            field: problem.field,
            expected: format!("native_v1/{}", problem.command).into(),
            received_type: problem.observed_type.into(),
        }
    }
}

impl From<String> for Expected {
    fn from(name: String) -> Self {
        Self::WireType { name }
    }
}
impl From<&str> for Expected {
    fn from(name: &str) -> Self {
        Self::WireType { name: name.into() }
    }
}
