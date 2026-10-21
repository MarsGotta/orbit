"""Pure, method-specific interaction decoding and bounded public representations."""

import json

from .protocol import HubError

TERMINAL_ERRORS = {'Context injection refused.', 'Turn cancelled before the agent was ready', 'Session no longer running before the agent was ready'}  # fmt: skip


def pending_data(request_id: str, kind: str, payload: dict) -> dict:
    result = {"request_id": request_id, "kind": kind, "state": "pending", "expires_at": None}
    if kind == "approval":
        result["description"] = str(
            payload.get("description") or payload.get("command") or "Hermes solicita autorización"
        )[:2048]
    else:
        result["question"] = str(payload.get("question") or "Responde a las preguntas de Hermes")[
            :2048
        ]
        questions = payload.get("questions")
        if isinstance(questions, list) and len(json.dumps(questions).encode()) <= 8192:
            result["questions"] = questions[:16]
    choices = payload.get("choices")
    if isinstance(choices, list):
        result["choices"] = [choice[:256] for choice in choices[:32] if isinstance(choice, str)]
    return result


def decode_response(kind: str, result: dict) -> tuple[bool, dict]:
    """Return whether still pending and a truthful public acknowledgement."""
    if kind == "approval":
        resolved = result.get("resolved")
        if type(resolved) is not int or resolved not in {0, 1}:
            raise HubError(
                "upstream_protocol", "Hermes no confirmó una aprobación individual; no se repetirá"
            )
        return False, {
            "status": "ok" if resolved == 1 else "expired",
            "resolved": resolved,
            "applied": resolved == 1,
        }
    status, remaining = result.get("status"), result.get("remaining", [])
    if (
        status not in {"ok", "expired"}
        or not isinstance(remaining, list)
        or len(remaining) > 32
        or not all(isinstance(item, str) and 1 <= len(item) <= 128 for item in remaining)
    ):
        raise HubError("upstream_protocol", "Hermes no confirmó la aclaración; no se repetirá")
    return status == "ok" and bool(remaining), {
        "status": status,
        "remaining": remaining,
        "applied": status == "ok",
    }
