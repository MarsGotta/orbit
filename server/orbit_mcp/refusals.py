"""What the assistant is told when a tool cannot answer: one sentence per code.

The tables name codes; the Mac's own sentence and the refusal's numbers follow
them, so a refusal says its cause and, when it has them, its sizes, ceilings
and counts.
"""

from __future__ import annotations

import json
import re
import time
from datetime import UTC, datetime
from time import monotonic

from mcp.types import CallToolResult, TextContent

from orbit_hub.captures import CAPTURE_TTL_SECONDS
from orbit_hub.log import report

from . import files, meetings
from .devices import DeviceSelection, UnknownDevice, folded_name, seen_ago
from .hub import MAC_NOTICE_CHARS, HubClientError
from .refusal_messages import (
    ENGINE_CAUSE_MESSAGES,
    ERROR_MESSAGES,
    FIELD_LABELS,
    TYPED_PERMISSION_MESSAGES,
)

# The device capabilities of the assistant's folders (`files.py`).
FILES_CAPABILITIES = frozenset({"files_list", "files_search", "files_read"})


def mac_sentence(detail) -> str:
    """What the Mac itself said about this refusal, if it said anything.

    The tables in `refusal_messages.py` name codes; only the Mac knows which
    parameter it rejected, which permission is missing or what it was busy
    with. The sentence arrives bounded and on one line from `hub.py`, and is
    bounded again here: a message to a reader is not a place to trust a length.
    """
    if not isinstance(detail, dict):
        return ""
    notice = detail.get("mac_notice")
    if not isinstance(notice, str):
        return ""
    notice = " ".join("".join(c for c in notice if c.isprintable() or c.isspace()).split())
    return re.sub(r"(?:/Users/|/home/|\$HOME)[^\s,;)]*", "[local path]", notice[:MAC_NOTICE_CHARS])


def _refusal_sentence(detail) -> str:
    if not isinstance(detail, dict) or not isinstance(detail.get("refusal"), dict):
        return ""
    refusal = detail["refusal"]
    legacy_cause = refusal.get("text") if refusal.get("code") == "legacy" else None
    fields = []
    if isinstance(legacy_cause, str):
        fields.append(f"Mac cause: {mac_sentence({'mac_notice': legacy_cause})}")
    for key, label in FIELD_LABELS.items():
        value = refusal.get(key)
        if isinstance(value, (str, int, dict)) and not isinstance(value, bool):
            shown = json.dumps(value, ensure_ascii=False) if isinstance(value, dict) else str(value)
            if key == "detail":
                shown = mac_sentence({"mac_notice": shown})
            fields.append(f"{label}: {shown}")
    return "; ".join(fields) + "." if fields else ""


def _walk_sentence(detail) -> str:
    """What this translator counted while walking the Mac's pages, if anything.

    Built here, from numbers, never from what the Mac sent: it is how a refusal
    that ended a walk says how many times the walk was tried.
    """
    if not isinstance(detail, dict):
        return ""
    notice = detail.get("walk_notice")
    return notice if isinstance(notice, str) else ""


def _device_unavailable(code: str, selection: DeviceSelection) -> CallToolResult:
    """A Mac that is not connected, said with when it was seen or re-paired."""
    device = selection.device
    name = device.name if device is not None else selection.requested_name
    newer = [
        candidate
        for candidate in selection.same_name
        if candidate.id != selection.device_id
        and (device is None or candidate.paired_at > device.paired_at)
    ]
    if newer:
        latest = max(newer, key=lambda candidate: (candidate.paired_at, candidate.id))
        message = (
            f"The Mac is not connected to Orbit. {name} was paired again "
            f"{seen_ago(latest.paired_at)}; use the new identity."
        )
    elif device is not None:
        message = (
            f"The Mac is not connected to Orbit. {name} was last seen {seen_ago(device.last_seen)}."
        )
    else:
        message = f"The Mac is not connected to Orbit: {name}."
    return CallToolResult(content=[_text_content(f"{code}: {message}")], isError=True)


def _spoken_code(code: str, detail) -> str:
    """The code a refusal is said with: a legacy refusal's own, when its text starts with one."""
    if code == "legacy" and isinstance(detail, dict):
        refusal = detail.get("refusal")
        legacy_text = refusal.get("text") if isinstance(refusal, dict) else None
        if isinstance(legacy_text, str):
            match = re.match(r"([a-z0-9_]{1,64})(?:: |$)", legacy_text)
            if match:
                return match.group(1)
    return code


def _upstream_failed(message, refusal: dict, detail: dict):
    # Capability models are checked by this consumer before rendering their fields.
    if refusal.get("service") == "hub" and detail.get("capability") == "screenshot":
        return "The Mac could not deliver the screenshot to Orbit. Check its connection before requesting it again."
    return message


def _internal(message, refusal: dict, detail: dict):
    cause = refusal.get("cause")
    if isinstance(cause, dict) and cause.get("kind") == "native_command":
        return ERROR_MESSAGES["native_command_invalid"]
    return message


def _helper_refused(message, refusal: dict, detail: dict):
    if isinstance(refusal.get("helper_code"), str):
        return ERROR_MESSAGES.get(refusal["helper_code"], message)
    return message


def _read_interrupted(message, refusal: dict, detail: dict):
    if isinstance(refusal.get("cause"), dict):
        reason = refusal["cause"].get("kind")
        if isinstance(reason, str):
            message = ERROR_MESSAGES.get(reason, message)
        if reason == "helper_restarted" and detail.get("capability") in FILES_CAPABILITIES:
            message = ERROR_MESSAGES["files_helper_restarted"]
    return message


def _timeout(message, refusal: dict, detail: dict):
    if detail.get("capability") in FILES_CAPABILITIES:
        return ERROR_MESSAGES["files_timeout"]
    return message


RECOVERY_STEPS = {
    "chunks": "chunks",
    "upload_jobs": "pending uploads",
    "snapshot": "snapshot",
    "environment": "service preparation",
    "operations": "pending operations",
    "call_preference": "call detection preference",
}


def _recovering(message, refusal: dict, detail: dict):
    step = RECOVERY_STEPS.get(refusal.get("step"), "unknown")
    return f"The store is recovering: step {step}; the command was not applied. Wait for recovery to finish."


def _skill_not_enabled(message, refusal: dict, detail: dict):
    if not isinstance(refusal.get("name"), str):
        return message
    enabled = refusal.get("enabled")
    listed = enabled.replace(",", ", ") if isinstance(enabled, str) and enabled else "none"
    return (
        f"This agent has no skill enabled in Orbit named «{refusal['name']}». "
        f"Enabled skills: {listed}."
    )


BUSY_OPERATIONS = {
    "chat": "Cannot switch conversations: a turn is in progress; interrupt it or wait for it to finish.",
    "audio": "Cannot switch conversations: a voice note is buffered; finish or interrupt it.",
    "voice_transcription": "Cannot switch conversations: voice transcription is still running; wait or interrupt it.",
    "audio_session": "Finish PTT or the current recording before switching to the call route.",
    "audio_drain": "Previous audio is still draining or waiting for durable storage; wait before continuing.",
}


def _busy(message, refusal: dict, detail: dict):
    operation = refusal.get("operation")
    return BUSY_OPERATIONS.get(operation, message) if isinstance(operation, str) else message


def _too_large(message, refusal: dict, detail: dict):
    if refusal.get("resource") == "ipc_response":
        return "The response exceeds the supported size. The command may have been applied; check the result before repeating it."
    return message


WIRE_FIELD_WORDS = {
    "object": "an object",
    "boolean": "true or false",
    "nonnegative_integer": "a nonnegative integer",
    "unix_milliseconds_integer": "integer milliseconds since 1970",
    "string": "text",
}


def _selection_message(expected: dict) -> str:
    message = (
        "No windows match the filters."
        if expected.get("reason") == "window_not_found"
        else "The matching windows cannot be selected."
    )
    return (
        message
        + f" {expected.get('windows')} windows, {expected.get('matching')} matches, {expected.get('selectable')} selectable and {expected.get('omitted')} omitted. List windows again before choosing another."
    )


def _invalid_parameter(message, refusal: dict, detail: dict):
    """What the Mac expected instead, when it said."""
    expected = refusal.get("expected")
    if not isinstance(expected, dict):
        return message
    kind = expected.get("kind")
    if kind == "open_recording":
        return "Start another recording; the previous one is durably closed and cannot be resumed."
    if kind == "wire_field":
        field, required = expected.get("field"), expected.get("expected")
        return f"{field} must be {WIRE_FIELD_WORDS.get(required, required)}."
    if kind == "selection_filters":
        return f"Filter {expected.get('unknown')} is unknown; allowed values: {', '.join(expected.get('allowed', []))}."
    if kind == "desktop_frame":
        return "Request a screenshot with full_screen=true first: desktop coordinates require that image."
    if kind == "desktop_coordinate":
        return f"Coordinate {expected.get('axis')}={expected.get('received')} is outside the screenshot of {expected.get('image_width')} × {expected.get('image_height')} pixels; correct it before retrying."
    if kind == "selection":
        return _selection_message(expected)
    if kind == "calendar_range":
        return f"Invalid calendar range: requires {expected.get('min_ms')} ≤ start_ms={expected.get('start_ms')} < end_ms={expected.get('end_ms')} ≤ {expected.get('max_ms')}."
    if kind == "calendar_cursor":
        return "Continuing an agenda read requires a nonempty read_id, after_ms and after_index; correct the cursor using the received data."
    if kind == "full_screen_filters":
        return "full_screen=true does not support application or title filters."
    if kind == "tool_call":
        return "The engine action violates the contract: check the name, argument object and their sizes before retrying."
    if kind == "title_length":
        return f"The title has {expected.get('characters')} characters; maximum {expected.get('max_characters')}."
    return message


def _transcription_cause(refusal: dict) -> str:
    cause = refusal.get("cause")
    provider_status = refusal.get("provider_status")
    timeout_ms = refusal.get("timeout_ms")
    if type(provider_status) is int:
        return f"The service returned HTTP {provider_status}."
    if cause == "transcription_connection_timeout" and type(timeout_ms) is int:
        return f"The service connection took longer than {timeout_ms} ms."
    if type(timeout_ms) is int:
        return f"The service took longer than {timeout_ms} ms."
    causes = {
        "transcription_protocol_error": "The service cut off the response before completing it.",
        "transcription_queued": "The audio group is queued.",
        "transcription_in_progress": "The audio group is being transcribed.",
        "transcription_group_not_ready": "The audio group is not ready yet.",
    }
    if isinstance(cause, str) and cause in causes:
        return causes[cause]
    return f"Cause: {cause}." if isinstance(cause, str) else "Cause missing from the hub."


def _transcription_retry(refusal: dict) -> str:
    retry_at_ms = refusal.get("retry_at_ms")
    if type(retry_at_ms) is not int or refusal.get("phase") != "pending":
        return ""
    try:
        retry_time = datetime.fromtimestamp(retry_at_ms // 1000, tz=UTC)
        return f" Next attempt scheduled for {retry_time:%Y-%m-%d %H:%M:%S} UTC."
    except (OverflowError, OSError, ValueError):
        remaining = max(0, (retry_at_ms - int(time.time() * 1000)) // 1000)
        return f" Next attempt scheduled in {remaining} seconds."


def _transcription(message, refusal: dict, detail: dict):
    """A transcript not there yet or failed: its cause, its attempts and the next one."""
    attempts = refusal.get("attempts")
    if type(attempts) is int and refusal.get("phase") == "running":
        count_text = f" Attempt {attempts} is in progress."
    else:
        count_text = f" {attempts} attempts." if type(attempts) is int else ""
    return f"{message} {_transcription_cause(refusal)}{count_text}{_transcription_retry(refusal)}"


def _permission(code: str):
    def message_for(message, refusal: dict, detail: dict):
        permission = refusal.get("permission")
        if isinstance(permission, str):
            return TYPED_PERMISSION_MESSAGES.get((code, permission), message)
        return message

    return message_for


def _engine_unavailable(message, refusal: dict, detail: dict):
    cause = refusal.get("cause")
    return ENGINE_CAUSE_MESSAGES.get(cause, message) if isinstance(cause, str) else message


def _too_many(message, refusal: dict, detail: dict):
    if refusal.get("resource") != "capture":
        return message
    count, maximum = refusal.get("count"), refusal.get("max")
    if type(count) is int and type(maximum) is int:
        return (
            f"The hub already holds {maximum} pending screenshots from this Mac; this upload "
            f"would be number {count}. Wait for them to be consumed or expire: each "
            f"is kept for at most {int(CAPTURE_TTL_SECONDS)} s."
        )
    return message


# The codes whose sentence depends on what their refusal carries: each turns
# the table's sentence (or None) into the one this refusal is said with.
REFINED_MESSAGES = {
    "upstream_failed": _upstream_failed,
    "internal": _internal,
    "helper_refused": _helper_refused,
    "read_interrupted": _read_interrupted,
    "timeout": _timeout,
    "recovering": _recovering,
    "skill_not_enabled": _skill_not_enabled,
    "busy": _busy,
    "too_large": _too_large,
    "invalid_parameter": _invalid_parameter,
    "transcription_failed": _transcription,
    "transcription_pending": _transcription,
    "permission_denied": _permission("permission_denied"),
    "permission_unknown": _permission("permission_unknown"),
    "permission_not_granted": _permission("permission_not_granted"),
    "engine_unavailable": _engine_unavailable,
    "too_many": _too_many,
}


def _numbers(code: str, refusal, detail) -> str:
    if code in meetings.NOTE_REFUSALS or code == "after_index_out_of_range":
        # Their numbers are already words: when, why and how many times.
        return meetings.refusal_sentence(code, refusal)
    return _refusal_sentence(detail)


def _known_devices(error: UnknownDevice) -> str:
    known = []
    seen = set()
    for device in error.devices:
        normalized = folded_name(device.name)
        if normalized in seen:
            continue
        seen.add(normalized)
        marker = " (default)" if device.id == error.default_device_id else ""
        known.append(f"{device.name}{marker}")
    return f"Known Macs: {', '.join(known) if known else 'none'}."


def error_result(
    error: str | HubClientError, selection: DeviceSelection | None = None
) -> CallToolResult:
    code = error.code if isinstance(error, HubClientError) else error
    if code == "device_unavailable" and selection is not None:
        return _device_unavailable(code, selection)
    detail = getattr(error, "detail", None)
    spoken_code = _spoken_code(code, detail)
    message = ERROR_MESSAGES.get(spoken_code)
    refusal = detail.get("refusal") if isinstance(detail, dict) else None
    if isinstance(refusal, dict) and code in REFINED_MESSAGES:
        message = REFINED_MESSAGES[code](message, refusal, detail)
    if message is None:
        message = f"Code {spoken_code} has no sentence in this translator."
    if code in files.REFUSALS and isinstance(refusal, dict):
        message = files.refusal_message(code, refusal)
    else:
        numbers = _numbers(code, refusal, detail)
        if numbers:
            message = f"{message} {numbers}"
    if isinstance(error, UnknownDevice):
        message = f"{message} {_known_devices(error)}"
    for said in (mac_sentence(detail), _walk_sentence(detail)):
        if said:
            message = f"{message} {said}"
    text = f"{spoken_code}: {message}"
    return CallToolResult(content=[_text_content(text)], isError=True)


def unexpected_result(device: str | None, started: float) -> CallToolResult:
    report(
        event="orbit_mcp_tool_failed",
        code="internal_error",
        device=device,
        elapsed_ms=max(0, int((monotonic() - started) * 1000)),
    )
    return error_result("internal_error")


def _text_content(value: str) -> TextContent:
    return TextContent(type="text", text=value)
