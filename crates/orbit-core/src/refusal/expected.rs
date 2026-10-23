//! What a parameter or a reply should have been: the shape every
//! `invalid_parameter` and `helper_reply_invalid` refusal carries, with the
//! numbers of the rule it broke.
use super::SelectionFacts;
use serde::{Deserialize, Serialize};

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
