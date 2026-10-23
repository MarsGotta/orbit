"""«Recordatorio personal»: `GET` and `PUT /v1/reminders/{key}` (hub-v1)."""

import json

from fastapi import Request
from fastapi.responses import JSONResponse

from ..errors import HubError
from ..log import in_store
from ..storage.reminders import MAX_REMINDER_CHARS

# The body: the text at its ceiling, every character escaped as six bytes, and
# the time. It bounds what is read before it is parsed.
MAX_REMINDER_BODY_BYTES = MAX_REMINDER_CHARS * 6 + 256


def _key(value: str) -> str:
    """A meeting's `title_digest`: 64 lowercase hexadecimal characters."""
    if len(value) != 64 or any(character not in "0123456789abcdef" for character in value):
        raise HubError(
            "invalid_parameters",
            "La clave del recordatorio no es válida: es el SHA-256 en hexadecimal del título",
            numbers={"field": "key", "chars": len(value), "expected_chars": 64},
        )
    return value


def _edit(body: bytes) -> tuple[str, int]:
    try:
        edit = json.loads(body)
    except ValueError:
        edit = None
    text = edit.get("text") if isinstance(edit, dict) else None
    edited = edit.get("edited_at_ms") if isinstance(edit, dict) else None
    if not isinstance(text, str):
        raise HubError("invalid_parameters", "Falta el texto", numbers={"field": "text"})
    if len(text) > MAX_REMINDER_CHARS:
        raise HubError(
            "invalid_parameters",
            f"El recordatorio tiene {len(text)} caracteres y el máximo es {MAX_REMINDER_CHARS}",
            numbers={"field": "text", "chars": len(text), "max_chars": MAX_REMINDER_CHARS},
        )
    if type(edited) is not int or not 0 <= edited < 2**63:
        raise HubError(
            "invalid_parameters",
            "edited_at_ms no válido: un instante en milisegundos",
            numbers={"field": "edited_at_ms"},
        )
    return text, edited


def mount(app, store, identity, bounded_body, store_executor):
    @app.get("/v1/reminders/{key}")
    async def read_reminder(key: str, request: Request):
        """The meeting's reminder; `reminder: null` when nobody wrote one."""
        identity(request)
        key = _key(key)
        found = await in_store(store_executor, lambda: store.reminders.get(key))
        return JSONResponse({"key": key, "reminder": found})

    @app.put("/v1/reminders/{key}")
    async def save_reminder(key: str, request: Request):
        """An edit of the reminder; the answer is the one kept, the latest."""
        identity(request)
        key = _key(key)
        text, edited = _edit(await bounded_body(request, MAX_REMINDER_BODY_BYTES))
        kept = await in_store(store_executor, lambda: store.reminders.save(key, text, edited))
        return JSONResponse({"key": key, "reminder": kept})
