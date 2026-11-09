"""What Hermes's refusals of the hub's RPCs mean, by method and code."""

from __future__ import annotations

import re

from ..protocol import HubError
from .hermes_events import RESUME_HISTORY_ERROR_PREFIX

# What Hermes's `image.attach_bytes` refusals mean (tui_gateway/
# prompt_attachments.py and methods_prompt.py, read 2026-10-22).
HERMES_IMAGE_REFUSALS = {
    4016: "does not support that image type",
    4017: "could not decode it",
    4018: "exceeds its 25 MiB per-image ceiling",
}
# JSON-RPC's "method not found": a Hermes older than the method.
METHOD_NOT_FOUND = -32601


def rpc_failure(method: str, error: dict) -> HubError:
    """What a Hermes RPC error means, by method and code."""
    code = error.get("code")
    if method == "client.capabilities":
        return HubError(
            "hermes_capabilities_refused",
            f"Hermes refused Orbit's declaration that it answers its questions (code {code}); "
            "its approvals could not reach the Mac, so the connection was closed",
            numbers={"upstream_code": code} if type(code) is int else None,
        )
    if method == "clarify.lock" and code == 4002:
        return HubError(
            "clarify_question_unknown",
            "Hermes does not know that question of the clarification (code 4002); nothing was answered",
        )
    if method == "image.attach_bytes" and code in {4016, 4017, 4018}:
        # 4016 an extension it does not take, 4017 bytes it cannot
        # decode, 4018 over its 25 MiB ceiling.
        return HubError(
            "attachment_refused",
            f"Hermes did not accept a message image (code {code})",
            numbers={"upstream_code": code},
        )
    if method in {"prompt.submit", "image.attach_bytes"} and code == 4001:
        return HubError(
            "session_runtime_not_found",
            "Hermes could not find the active session (code 4001); the message was not accepted and can be resent after resuming",
        )
    if method == "session.resume":
        refusal = _resume_failure(code, error.get("message"))
        if refusal is not None:
            return refusal
    return HubError("upstream_rejected", f"Hermes refused the operation (code {code})")


def _resume_failure(code: object, reason: object) -> HubError | None:
    """Why Hermes refused a `session.resume`, when its code and message say."""
    if code == METHOD_NOT_FOUND:
        return HubError(
            "session_resume_unsupported",
            "This Hermes version cannot resume stored sessions (code -32601)",
        )
    if code == 4007 and reason == "session not found":
        return HubError(
            "session_resume_not_found",
            "Hermes no longer retains the stored conversation (code 4007); local history is still available",
        )
    if code == 4007 and reason == "session no longer live; retry resume":
        return HubError(
            "session_resume_race",
            "Hermes closed the session while resuming (code 4007); try again",
        )
    if code == 4130:
        numbers = (
            re.search(r"\((\d+) messages; limit (\d+)\)", reason)
            if isinstance(reason, str)
            else None
        )
        if numbers is not None:
            return HubError(
                "session_resume_too_large",
                f"Hermes cannot resume {numbers[1]} messages; limit {numbers[2]} (code 4130)",
            )
        return HubError(
            "session_resume_too_large",
            "Hermes refused to resume due to size (code 4130); no count or limit was supplied",
        )
    if code == 4009 and reason == "session disconnect interrupt settling":
        return HubError(
            "session_resume_busy",
            "Hermes is still closing the previous turn (code 4009); wait",
        )
    if code == 5000 and isinstance(reason, str):
        if reason.startswith("Session storage is unavailable:"):
            return HubError(
                "session_resume_storage_unavailable",
                "Hermes cannot open its session store (code 5000)",
            )
        if reason.startswith(RESUME_HISTORY_ERROR_PREFIX):
            return HubError(
                "session_resume_history_unreadable",
                "Hermes could not read stored history (code 5000); the message was not sent",
            )
    return None
