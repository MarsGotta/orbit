"""One transport-aware, redacted error contract for every hub boundary."""

from fastapi.responses import JSONResponse

_STATUS_CODES = {401: 'unauthorized', 403: 'revoked invalid_pairing', 404: 'session_not_found meeting_not_found recording_not_found note_not_requested note_template_not_found agent_not_found', 409: 'chunk_conflict note_not_ready note_revision_changed stale_revision recording_finalized manifest_conflict manifest_mismatch', 413: 'too_large session_message_too_large', 429: 'rate_limited', 507: 'quota_exceeded'}  # fmt: skip


def _http_status(code: str) -> int:
    return next((status for status, codes in _STATUS_CODES.items() if code in codes.split()), 400)


class HubError(Exception):
    def __init__(
        self,
        code: str,
        message: str,
        http_status: int | None = None,
        numbers: dict[str, int] | None = None,
    ):
        super().__init__(message)
        self.code, self.message = code, message
        self.numbers = numbers
        self.http_status = _http_status(code) if http_status is None else http_status


_CHECK_OUTCOME = {
    "outcome_unknown",
    "no_answer",
    "connection_lost",
    "receipt_lost",
    "device_unavailable",
    "transcription_pending",
    "device_reply_timeout",
}
# Work that is under way: asking again later succeeds without anyone fixing
# anything, so the advice is a wait, never "do not retry".
_IN_PROGRESS = {"note_queued", "note_generating", "note_waiting_transcript"}
_AFTER_FIX = {
    "invalid_parameters",
    "unauthorized",
    "tls_required",
    "origin_forbidden",
    "revoked",
    "rate_limited",
    "device_reply_invalid",
    "agent_mismatch",
    # Settings › Agents: the user fixes the file or removes a skill first.
    "skill_invalid",
    "too_many_skills",
}

_OPERATIONS = {
    "capture": "capture",
    "session": "chat",
}
_RESOURCES = {
    "recording": "recording",
    "session": "session",
    "speech": "audio",
}
# `hub_request` is the body of an HTTP request to the hub that no narrower
# resource names (pairing, notes, templates, dictionaries, meeting edits);
# `ipc_request`/`ipc_response` are the Mac's own IPC lines.
_KNOWN_SIZE_RESOURCES = frozenset(_RESOURCES.values()) | {
    "capture",
    "hub_request",
    "ipc_request",
    "ipc_response",
}


def _wire_u64(value: object) -> bool:
    # This checks Rust's u64 wire type, not a new payload-size ceiling.
    return type(value) is int and 0 <= value <= 2**64 - 1


def _family(code: str | None, numbers: dict) -> tuple[str | None, dict]:
    if code is None:
        return None, {}
    if code == "device_unavailable":
        return "mac_disconnected", {"last_seen_ms": None}
    if code == "too_many":
        resource = numbers.get("resource")
        count = numbers.get("count")
        maximum = numbers.get("max")
        if resource == "capture" and _wire_u64(count) and _wire_u64(maximum):
            return code, {"resource": resource, "count": count, "max": maximum}
    if code == "busy" or code.endswith(("_busy", "_capacity")):
        prefix = code.removesuffix("_busy").removesuffix("_capacity")
        return "busy", {
            "operation": _OPERATIONS.get(prefix, "interaction"),
            "retry_after_ms": numbers.get("retry_after_ms"),
        }
    if code == "too_large" or code.endswith("_too_large"):
        prefix = code.removesuffix("_too_large")
        size = numbers.get("bytes", numbers.get("received_bytes"))
        maximum = numbers.get("max_bytes")
        resource = numbers.get("resource", _RESOURCES.get(prefix))
        if _wire_u64(size) and _wire_u64(maximum) and resource in _KNOWN_SIZE_RESOURCES:
            return "too_large", {"resource": resource, "bytes": size, "max_bytes": maximum}
        fields = {
            key: value
            for key, value in numbers.items()
            if key not in {"bytes", "received_bytes", "max_bytes", "resource"}
        }
        if _wire_u64(size):
            fields["bytes"] = size
        if _wire_u64(maximum):
            fields["max_bytes"] = maximum
        return code, fields
    if code.endswith("_not_found") and code.removesuffix("_not_found") in _RESOURCES:
        return "not_found", {"resource": _RESOURCES[code.removesuffix("_not_found")]}
    if (
        code == "no_answer"
        and _wire_u64(numbers.get("deadline_ms"))
        and _wire_u64(numbers.get("waited_ms"))
    ):
        return "timeout", {
            "operation": numbers.get("operation", "interaction"),
            "waited_ms": numbers["waited_ms"],
            "deadline_ms": numbers["deadline_ms"],
        }
    return code, numbers


def render(
    code: str | None,
    message: str,
    *,
    payload: dict | None = None,
    numbers: dict | None = None,
) -> dict:
    """Add one refusal to an existing HTTP, WebSocket or private-route payload."""
    payload = dict(payload or {})
    payload["message"] = message
    if code is not None:
        payload["code"] = code
    if isinstance(payload.get("refusal"), dict):
        return payload
    details = numbers or {}
    family, fields = _family(code, details)
    busy = family == "busy"
    retry_after = details.get("retry_after_ms")
    if (busy or code == "rate_limited" or code in _IN_PROGRESS) and isinstance(retry_after, int):
        disposition = {"kind": "after_ms", "ms": retry_after}
    elif (
        busy
        or family in {"too_large", "too_many", "not_found"}
        or code in _AFTER_FIX
        or (code is not None and code.endswith(("_too_large", "_not_found")))
    ):
        disposition = {"kind": "after_fix"}
    elif family == "timeout" or code in _CHECK_OUTCOME:
        disposition = {"kind": "check_outcome"}
    else:
        disposition = {"kind": "never"}
    payload["refusal"] = {"code": family, **fields, "retry": disposition}
    return payload


def http_response(
    code: str | None,
    message: str,
    status: int,
    *,
    payload: dict | None = None,
    numbers: dict[str, int] | None = None,
) -> JSONResponse:
    return JSONResponse(
        render(code, message, payload=payload, numbers=numbers),
        status_code=status,
    )


class PrivateError(ValueError):
    """Only fixed, non-sensitive public codes cross the HTTP boundary."""

    def __init__(
        self,
        status: int,
        code: str,
        numbers: dict[str, int | str] | None = None,
        detail: dict[str, str] | None = None,
    ):
        self.status, self.code = status, code
        self.numbers = numbers
        self.detail = detail
        super().__init__(code)
