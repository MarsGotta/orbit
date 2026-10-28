"""Device capability payloads consumed and constructed by orbit-mcp."""

import json
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from pydantic_core import PydanticCustomError


def present_value(value):
    if value is None:
        raise ValueError("A present field cannot be null")
    return value


class WireModel(BaseModel):
    # Rust fixtures pin this boundary; additions require updating this consumer.
    model_config = ConfigDict(strict=True, extra="forbid")


# Retains the existing 1024-character filter metadata budget, independently
# of the PNG and the transport frame.
MAX_FILTER_CHARS = 1024
# Mac calendar read identities are UUIDs; 128 bytes bounds retained cursor metadata.
MAX_READ_ID_BYTES = 128
# Engine identifiers are ASCII; this matches ToolCall::validate on the Mac.
MAX_TOOL_NAME_BYTES = 128
# Bounds the complete canonical engine action, not the HTTP envelope.
MAX_ACTION_BYTES = 16_384
# Bounds sanitized Mac detail retained in a refusal, not conversation content.
MAX_BRIDGE_DETAIL_CHARS = 400
# Bounds the helper's diagnostic cause code, independently of its detail.
MAX_BRIDGE_CODE_BYTES = 64
# A path into the assistant's folders: macOS's PATH_MAX, the longest path its
# system calls accept. A copy of orbit_core::assistant_files::MAX_PATH_BYTES,
# compared by the guard of contracts/constants.json.
MAX_PATH_BYTES = 1024
# A search text: an APFS file name is at most 255 bytes, so a longer text
# matches none. A copy of orbit_core::assistant_files::MAX_QUERY_BYTES.
MAX_QUERY_BYTES = 256


class EmptyParameters(WireModel):
    pass


class ScreenshotParameters(WireModel):
    # None means the caller omitted a filter. A wrong type is never omission.
    application: str | None = Field(
        default=None, max_length=MAX_FILTER_CHARS, exclude_if=lambda value: value is None
    )
    title: str | None = Field(
        default=None, max_length=MAX_FILTER_CHARS, exclude_if=lambda value: value is None
    )
    full_screen: bool | None = Field(default=None, exclude_if=lambda value: value is None)

    _present_values = field_validator("application", "title", "full_screen", mode="before")(
        present_value
    )


class SelectedTextParameters(WireModel):
    application: str | None = Field(
        default=None, max_length=MAX_FILTER_CHARS, exclude_if=lambda value: value is None
    )
    title: str | None = Field(
        default=None, max_length=MAX_FILTER_CHARS, exclude_if=lambda value: value is None
    )

    _present_values = field_validator("application", "title", mode="before")(present_value)


class ControlToolsParameters(WireModel):
    after_index: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)

    _present_values = field_validator("after_index", mode="before")(present_value)


class ToolAction(WireModel):
    tool: str = Field(
        min_length=1,
        max_length=MAX_TOOL_NAME_BYTES,
        pattern=r"^[A-Za-z0-9_-]+$",
        description="Name from the orbit_control_tools catalogue; 1 to 128 ASCII letters, digits, _ or -.",
    )
    arguments: dict = Field(default={}, description="Tool arguments; empty by default.")


class PerformActionParameters(WireModel):
    action: ToolAction

    @model_validator(mode="after")
    def bounded_action(self):
        size = len(
            json.dumps(
                self.action.model_dump(), ensure_ascii=False, sort_keys=True, separators=(",", ":")
            ).encode("utf-8")
        )
        if size > MAX_ACTION_BYTES:
            raise PydanticCustomError(
                "action_size",
                "Action too large",
                {"field": "action", "bytes": size, "max_bytes": MAX_ACTION_BYTES},
            )
        return self


def bounded_read_id(value):
    if value is not None and not 0 < len(value.encode("utf-8")) <= MAX_READ_ID_BYTES:
        raise PydanticCustomError(
            "cursor_size",
            "Read identity exceeds the limit",
            {"bytes": len(value.encode("utf-8")), "max_bytes": MAX_READ_ID_BYTES},
        )
    return value


class CalendarReadParameters(WireModel):
    start_ms: int
    end_ms: int
    read_id: str | None = Field(default=None, exclude_if=lambda value: value is None)
    after_ms: int | None = Field(default=None, exclude_if=lambda value: value is None)
    after_index: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)
    after_invitee: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)

    _present_values = field_validator(
        "read_id", "after_ms", "after_index", "after_invitee", mode="before"
    )(present_value)

    _bounded_read_id = field_validator("read_id")(bounded_read_id)

    @model_validator(mode="after")
    def valid_range_and_cursor(self):
        if self.end_ms <= self.start_ms:
            raise PydanticCustomError(
                "calendar_range",
                "Invalid range",
                {
                    "field": "end_ms",
                    "expected": "later than start_ms",
                    "start_ms": self.start_ms,
                    "end_ms": self.end_ms,
                },
            )
        cursor = (self.read_id, self.after_ms, self.after_index)
        if any(value is not None for value in (*cursor, self.after_invitee)) and any(
            value is None for value in cursor
        ):
            for field, required in (
                ("read_id", "text"),
                ("after_ms", "integer milliseconds"),
                ("after_index", "nonnegative integer"),
            ):
                if getattr(self, field) is None:
                    raise PydanticCustomError(
                        "calendar_cursor",
                        "Incomplete cursor",
                        {"field": field, "expected": required},
                    )
        return self


def bounded_path(value):
    if value is None:
        return value
    size = len(value.encode("utf-8"))
    if not value.startswith("/") or size > MAX_PATH_BYTES or "\x00" in value:
        raise PydanticCustomError(
            "folder_path",
            "Invalid path",
            {
                "field": "path",
                "expected": f"absolute path of at most {MAX_PATH_BYTES} bytes",
                "bytes": size,
                "max_bytes": MAX_PATH_BYTES,
            },
        )
    return value


class FilesListParameters(WireModel):
    path: str | None = Field(default=None, exclude_if=lambda value: value is None)
    after_index: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)

    _present_values = field_validator("path", "after_index", mode="before")(present_value)
    _bounded_path = field_validator("path")(bounded_path)


class FilesSearchParameters(WireModel):
    query: str
    path: str | None = Field(default=None, exclude_if=lambda value: value is None)
    content: bool | None = Field(default=None, exclude_if=lambda value: value is None)

    _present_values = field_validator("path", "content", mode="before")(present_value)
    _bounded_path = field_validator("path")(bounded_path)

    @field_validator("query")
    @classmethod
    def bounded_query(cls, value):
        size = len(value.encode("utf-8"))
        if not value.strip() or size > MAX_QUERY_BYTES:
            raise PydanticCustomError(
                "search_query",
                "Invalid search",
                {
                    "field": "query",
                    "expected": f"text of 1 to {MAX_QUERY_BYTES} bytes",
                    "bytes": size,
                    "max_bytes": MAX_QUERY_BYTES,
                },
            )
        return value


class FilesReadParameters(WireModel):
    path: str
    after_index: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)
    after_page: int | None = Field(default=None, ge=0, exclude_if=lambda value: value is None)

    _present_values = field_validator("after_index", "after_page", mode="before")(present_value)
    _bounded_path = field_validator("path")(bounded_path)


PARAMETER_MODELS = {
    "screenshot": ScreenshotParameters,
    "screen_sources": EmptyParameters,
    "selected_text": SelectedTextParameters,
    "control_tools": ControlToolsParameters,
    "perform_action": PerformActionParameters,
    "calendar_read": CalendarReadParameters,
    "files_list": FilesListParameters,
    "files_search": FilesSearchParameters,
    "files_read": FilesReadParameters,
}


class WindowSummary(WireModel):
    application: str
    title: str


class ScreenSourcesResult(WireModel):
    windows: list[WindowSummary]


class ScreenshotMetadata(WireModel):
    application: str
    title: str
    width: int
    height: int
    mime_type: str
    sha256: str
    # On a full screen, the Mac's sentence on what its coordinates
    # are (the image is fitted; desktop actions are given on it).
    coordinates: str | None = None


class ScreenshotResult(ScreenshotMetadata):
    capture_id: str


class ScreenshotContentResult(ScreenshotMetadata):
    data_base64: str


class SelectedTextResult(WireModel):
    application: str
    title: str
    text: str


class EngineTool(WireModel):
    name: str
    description: str = ""
    input_schema: object = None


class ControlToolsResult(WireModel):
    tools: list[EngineTool]
    after_index: int
    total: int
    next_after_index: int | None = None


class ActionReceipt(WireModel):
    operation_id: str
    tool: str
    state: str
    attempted: bool
    driver_receipt: object
    retry: str


class PerformActionResult(WireModel):
    receipt: ActionReceipt


class CalendarRequested(WireModel):
    start_unix_ms: int
    end_unix_ms: int


class CalendarOmission(WireModel):
    reason: str
    count: int


class CalendarReference(WireModel):
    calendar_id: str
    source_id: str


class CalendarInvitee(WireModel):
    name: str | None = None
    participant_url: str
    email: str | None = None
    role: str
    response_status: str
    participant_type: str
    is_current_user: bool


class CalendarFieldLeftOut(WireModel):
    bytes: int
    limit: int


class CalendarRecurrenceDay(WireModel):
    day: Literal["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]
    week: int | None = None


class CalendarRecurrenceEnd(WireModel):
    until_ms: int | None = None
    count: int | None = None


class CalendarRecurrence(WireModel):
    frequency: Literal["daily", "weekly", "monthly", "yearly"]
    interval: int
    days_of_week: list[CalendarRecurrenceDay] | None = None
    end: CalendarRecurrenceEnd | None = None
    rules_count: int | None = None


class CalendarEvent(WireModel):
    observation_id: str
    calendar: CalendarReference
    calendar_item_id: str
    event_id: str | None = None
    original_occurrence_unix_ms: int | None = None
    start_unix_ms: int
    end_unix_ms: int
    event_timezone: str | None = None
    interpreted_timezone: str
    is_floating: bool
    is_all_day: bool
    is_detached: bool
    title: str
    status: str
    last_modified_unix_ms: int | None = None
    invitees: list[CalendarInvitee] | None = None
    invitees_status: Literal["not_requested", "not_provided", "provided"]
    event_url: str | None = None
    event_url_status: Literal["not_requested", "not_provided", "provided"]
    invitees_continued: bool | None = None
    # Optional EventKit fields consumed by the agenda renderer.
    location: str | None = None
    notes: str | None = None
    notes_left_out: CalendarFieldLeftOut | None = None
    organizer: CalendarInvitee | None = None
    availability: Literal["busy", "free", "tentative", "unavailable", "not_supported"] | None = None
    recurrence: CalendarRecurrence | None = None
    can_edit: bool | None = None
    # The event's reminders, in minutes before its start, and how many
    # more it has that Orbit does not edit (absolute or by location).
    alarms_minutes: list[int] | None = None
    alarms_not_editable: int | None = None


class CalendarOversizedEvent(WireModel):
    calendar: CalendarReference
    calendar_item_id: str | None = None
    start_unix_ms: int
    end_unix_ms: int
    field: str
    bytes: int
    limit: int


class CalendarReadResult(WireModel):
    read_id: str
    requested: CalendarRequested
    read_at_unix_ms: int | None
    system_timezone: str | None
    provider_sync_status: str
    enumeration: str
    events_returned: int
    events: list[CalendarEvent]
    omitted: list[CalendarOmission]
    oversized: list[CalendarOversizedEvent] | None = None
    next_after_ms: int | None = None
    next_after_index: int | None = None
    next_after_invitee: int | None = None

    _bounded_read_id = field_validator("read_id")(bounded_read_id)


class FolderCause(WireModel):
    model_config = ConfigDict(strict=True, extra="allow")
    kind: str


class FileEntry(WireModel):
    name: str
    kind: Literal["file", "folder", "link", "other"]
    size_bytes: int | None = None
    modified_ms: int | None = None
    unavailable: FolderCause | None = None


class FilesListResult(WireModel):
    path: str | None = None
    entries: list[FileEntry]
    total: int
    after_index: int
    next_after_index: int | None = None


class SearchHit(WireModel):
    path: str
    kind: Literal["file", "folder", "link", "other"]
    size_bytes: int | None = None
    line: int | None = None
    snippet: str | None = None


class SearchStop(WireModel):
    cause: Literal["results", "visited", "files_read", "bytes_read", "deadline", "page_bytes"]
    limit: int


class FilesSearchResult(WireModel):
    query: str
    path: str | None = None
    content: bool
    results: list[SearchHit]
    visited: int
    files_read: int
    bytes_read: int
    partial_files: int
    stopped: SearchStop | None = None


class PdfPage(WireModel):
    page: int
    text: str


class FilesReadResult(WireModel):
    """One file as read; an image arrives with its bytes (`data_base64`),
    which the hub put in place of the Mac's `capture_id`."""

    path: str
    size_bytes: int
    modified_ms: int | None = None
    content: Literal["text", "pdf", "image"]
    text: str | None = None
    after_index: int | None = None
    next_after_index: int | None = None
    pages: list[PdfPage] | None = None
    page_count: int | None = None
    after_page: int | None = None
    next_after_page: int | None = None
    width: int | None = None
    height: int | None = None
    mime_type: str | None = None
    sha256: str | None = None
    source_width: int | None = None
    source_height: int | None = None
    capture_id: str | None = None
    data_base64: str | None = None

    @model_validator(mode="after")
    def complete_content(self):
        required = {
            "text": ("text", "after_index"),
            "pdf": ("pages", "page_count", "after_page", "after_index"),
            "image": ("width", "height", "mime_type", "source_width", "source_height"),
        }[self.content]
        for field in required:
            if getattr(self, field) is None:
                raise PydanticCustomError(
                    "file_content",
                    "Incomplete read",
                    {"field": field, "expected": f"field of {self.content}"},
                )
        return self


RESULT_MODELS = {
    "screenshot": ScreenshotResult,
    "screen_sources": ScreenSourcesResult,
    "selected_text": SelectedTextResult,
    "control_tools": ControlToolsResult,
    "perform_action": PerformActionResult,
    "calendar_read": CalendarReadResult,
    "files_list": FilesListResult,
    "files_search": FilesSearchResult,
    "files_read": FilesReadResult,
}


class RetryDisposition(WireModel):
    kind: Literal["after_ms", "after_fix", "check_outcome", "never"]
    ms: int | None = None


class BridgeRefusal(WireModel):
    model_config = ConfigDict(strict=True, extra="allow")
    code: str = Field(min_length=1, max_length=MAX_BRIDGE_CODE_BYTES, pattern=r"^[a-z0-9_]+$")
    retry: RetryDisposition
    detail: str | None = Field(default=None, max_length=MAX_BRIDGE_DETAIL_CHARS)


def validation_detail(error, prefix):
    """Describe a schema failure without echoing the untrusted payload."""
    first = error.errors()[0]
    location = ".".join(str(part) for part in first["loc"])
    field = ".".join(part for part in (prefix, location) if part)
    expected = {
        "list_type": "list",
        "string_type": "text",
        "int_type": "integer",
        "bool_type": "true or false",
        "dict_type": "object",
        "model_type": "object",
        "extra_forbidden": "no additional keys",
        "missing": "required field",
        "greater_than_equal": "nonnegative integer",
        "string_pattern_mismatch": f"ASCII name of 1 to {MAX_TOOL_NAME_BYTES} bytes with letters, digits, _ or -",
        "string_too_long": "text within the stated limit",
    }.get(first["type"], "valid contract value")
    context = first.get("ctx", {})
    numbers = {key: value for key, value in context.items() if type(value) is int}
    if first["type"] == "missing":
        expected = {
            "action": "object",
            "action.tool": "text",
            "start_ms": "integer",
            "end_ms": "integer",
        }.get(location, expected)
    if first["type"] == "string_too_long":
        numbers = {"chars": len(first["input"]), "max_chars": context["max_length"]}
        expected = f"text of at most {context['max_length']} characters"
    if first["type"] == "value_error" and location in {"application", "title", "full_screen"}:
        expected = "true or false" if location == "full_screen" else "text"
    if first["type"] in {
        "calendar_cursor",
        "calendar_range",
        "action_size",
        "folder_path",
        "search_query",
        "file_content",
    }:
        field = context["field"]
        expected = context.get("expected", expected)
    return {"field": field, "expected": expected, **numbers}
