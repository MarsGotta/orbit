//! Typed payloads carried inside hub-v1 device requests and replies.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Bounds retained calendar cursor metadata to its existing 128-byte budget;
/// generated UUIDs fit in 36 bytes.
/// This is not the calendar page or event limit.
pub const MAX_READ_ID_BYTES: usize = 128;

fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRequestData {
    pub request_id: String,
    #[serde(flatten)]
    pub request: BridgeRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum BridgeRequest {
    Screenshot {
        #[serde(default)]
        parameters: ScreenshotParameters,
    },
    ScreenSources {
        #[serde(default)]
        parameters: EmptyParameters,
    },
    SelectedText {
        #[serde(default)]
        parameters: SelectedTextParameters,
    },
    ControlTools {
        #[serde(default)]
        parameters: ControlToolsParameters,
    },
    PerformAction {
        parameters: PerformActionParameters,
    },
    CalendarRead {
        parameters: CalendarReadParameters,
    },
    FilesList {
        #[serde(default)]
        parameters: FilesListParameters,
    },
    FilesSearch {
        parameters: FilesSearchParameters,
    },
    FilesRead {
        parameters: FilesReadParameters,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyParameters {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotParameters {
    /// `None` means the caller did not filter by application. A present value
    /// must be a string; a mistyped filter must never broaden a screenshot.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub application: Option<String>,
    /// `None` means the caller did not filter by window title.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub full_screen: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedTextParameters {
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub application: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlToolsParameters {
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformActionParameters {
    pub action: crate::computer_control::ToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarReadParameters {
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub read_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_index: Option<usize>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_invitee: Option<usize>,
}

/// Without `path`, the folders themselves; with it, one folder inside them,
/// from `after_index`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesListParameters {
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub path: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_index: Option<u64>,
}

/// A name, or with `content` also the text of plain-text files, under `path`
/// or every folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesSearchParameters {
    pub query: String,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub path: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub content: Option<bool>,
}

/// One file, from the position the previous answer gave: `after_index` in a
/// text file, `after_page` and `after_index` in a PDF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesReadParameters {
    pub path: String,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_index: Option<u64>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceReplyParams {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<BridgeResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<BridgeRefusal>,
}

#[derive(Deserialize)]
struct UncheckedDeviceReply {
    request_id: String,
    #[serde(default)]
    result: Option<BridgeResult>,
    #[serde(default)]
    refusal: Option<BridgeRefusal>,
}

impl<'de> Deserialize<'de> for DeviceReplyParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = UncheckedDeviceReply::deserialize(deserializer)?;
        if usize::from(raw.result.is_some()) + usize::from(raw.refusal.is_some()) != 1 {
            return Err(serde::de::Error::custom(
                "device reply needs exactly one result or refusal",
            ));
        }
        Ok(Self {
            request_id: raw.request_id,
            result: raw.result,
            refusal: raw.refusal,
        })
    }
}

/// A refusal plus the bounded, sanitized detail used for a bad input.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRefusal {
    pub reason: crate::refusal::Refusal,
    pub detail: Option<String>,
    pub retry_in_ms: Option<u64>,
    /// Why an attempted action lost its reply; its outcome remains uncertain.
    pub interruption: Option<Box<crate::refusal::Refusal>>,
}

impl Serialize for BridgeRefusal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.reason).map_err(serde::ser::Error::custom)?;
        if let (Some(detail), Some(object)) = (&self.detail, value.as_object_mut()) {
            object.insert("detail".into(), Value::String(detail.clone()));
        }
        if let (Some(retry_in_ms), Some(object)) = (self.retry_in_ms, value.as_object_mut()) {
            object.insert("retry_in_ms".into(), Value::from(retry_in_ms));
        }
        if let (Some(interruption), Some(object)) = (&self.interruption, value.as_object_mut()) {
            object.insert(
                "interruption".into(),
                serde_json::to_value(interruption).map_err(serde::ser::Error::custom)?,
            );
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BridgeRefusal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        let detail = value
            .as_object_mut()
            .and_then(|object| object.remove("detail"))
            .map(|detail| serde_json::from_value(detail).map_err(serde::de::Error::custom))
            .transpose()?;
        let retry_in_ms = value
            .as_object_mut()
            .and_then(|object| object.remove("retry_in_ms"))
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .transpose()?;
        let interruption = value
            .as_object_mut()
            .and_then(|object| object.remove("interruption"))
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .transpose()?;
        let reason = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            reason,
            detail,
            retry_in_ms,
            interruption,
        })
    }
}

impl DeviceReplyParams {
    pub fn answered(request_id: String, result: BridgeResult) -> Self {
        Self {
            request_id,
            result: Some(result),
            refusal: None,
        }
    }

    pub fn refused(
        request_id: String,
        reason: crate::refusal::Refusal,
        detail: Option<String>,
    ) -> Self {
        Self {
            request_id,
            result: None,
            refusal: Some(BridgeRefusal {
                reason,
                detail,
                retry_in_ms: None,
                interruption: None,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BridgeResult {
    Screenshot(ScreenshotResult),
    ScreenSources(ScreenSourcesResult),
    SelectedText(SelectedTextResult),
    ControlTools(ControlToolsResult),
    PerformAction(PerformActionResult),
    CalendarRead(CalendarReadResult),
    FilesList(FilesListResult),
    FilesSearch(FilesSearchResult),
    FilesRead(FilesReadResult),
}

/// A page of a folder, or of the folders themselves when `path` is absent
/// (their entries are then named by their whole path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesListResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub entries: Vec<crate::assistant_files::Entry>,
    pub total: u64,
    pub after_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_index: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesSearchResult {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub content: bool,
    pub results: Vec<crate::assistant_files::SearchHit>,
    pub visited: u64,
    pub files_read: u64,
    pub bytes_read: u64,
    pub partial_files: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<crate::assistant_files::SearchStop>,
}

/// One file as read: `path` is where it resolved on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesReadResult {
    pub path: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<i64>,
    #[serde(flatten)]
    pub content: FileContent,
}

/// What a read carries. An image travels as a capture, like a screenshot:
/// the hub swaps `capture_id` for the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "content", rename_all = "snake_case")]
pub enum FileContent {
    Text {
        text: String,
        after_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_index: Option<u64>,
    },
    Pdf {
        pages: Vec<crate::assistant_files::PdfPage>,
        page_count: u32,
        after_page: u32,
        after_index: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_page: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_after_index: Option<u64>,
    },
    Image {
        capture_id: String,
        width: u32,
        height: u32,
        mime_type: String,
        sha256: String,
        source_width: u32,
        source_height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub capture_id: String,
    pub application: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
    pub sha256: String,
    /// On a full screen, what its coordinates are, in plain words
    /// with both sizes: the image Deimos sees is fitted, and a desktop action
    /// is given on it. Absent on a window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSummary {
    pub application: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSourcesResult {
    pub windows: Vec<WindowSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedTextResult {
    pub application: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlToolsResult {
    pub tools: Vec<crate::computer_control::EngineTool>,
    pub after_index: usize,
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformActionResult {
    pub receipt: ActionReceipt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub operation_id: String,
    pub tool: String,
    pub state: String,
    pub attempted: bool,
    pub driver_receipt: Value,
    pub retry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarRequested {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarReadResult {
    pub read_id: String,
    pub requested: CalendarRequested,
    pub read_at_unix_ms: Option<i64>,
    pub system_timezone: Option<String>,
    pub provider_sync_status: String,
    pub enumeration: String,
    pub events_returned: usize,
    pub events: Vec<CalendarBridgeEvent>,
    pub omitted: Vec<CalendarOmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oversized: Option<Vec<crate::calendar::CalendarOversizedEvent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_invitee: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarOmission {
    pub reason: String,
    pub count: u32,
}

/// A calendar occurrence, with the one paging marker that is not part of an
/// EventKit occurrence itself.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarBridgeEvent {
    pub occurrence: crate::calendar::CalendarOccurrence,
    pub invitees_continued: Option<bool>,
}

impl Serialize for CalendarBridgeEvent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value =
            serde_json::to_value(&self.occurrence).map_err(serde::ser::Error::custom)?;
        if let (Some(continued), Some(object)) = (self.invitees_continued, value.as_object_mut()) {
            object.insert("invitees_continued".into(), Value::Bool(continued));
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CalendarBridgeEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        let invitees_continued = value
            .as_object_mut()
            .and_then(|object| object.remove("invitees_continued"))
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .transpose()?;
        let occurrence = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            occurrence,
            invitees_continued,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_capture_filter_does_not_become_unfiltered_capture() {
        for capability in ["screenshot", "selected_text"] {
            let raw = json!({"request_id":"fixture", "capability":capability,
                "parameters":{"window":"Safari"}});
            assert!(
                serde_json::from_value::<DeviceRequestData>(raw).is_err(),
                "{capability} accepted an unknown capture filter"
            );
        }
    }
}
