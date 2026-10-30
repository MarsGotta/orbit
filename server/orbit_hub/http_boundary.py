"""The hub's HTTP boundary: bounded input, transport and the one error handler.

Every HTTP error body gets its request's op and a typed refusal; whatever a
route did not foresee is answered here without its text (a full disk with
its numbers, anything else as `internal_error`).
"""

import io
import json
import wave

from fastapi import Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from starlette.requests import ClientDisconnect

from .config import Settings
from .errors import http_response, render
from .log import bind_op, get_logger, report
from .protocol import HubError
from .storage import is_storage_full
from .urls import LOOPBACK_HOSTS

logger = get_logger(__name__)


def bearer(headers) -> str:
    value = headers.get("authorization", "")
    if not value.startswith("Bearer "):
        raise HubError("unauthorized", "Device credential required")
    return value[7:]


def allowed_transport(scope, settings: Settings) -> bool:
    if scope["scheme"] in {"https", "wss"}:
        return True
    host = scope.get("client", ("",))[0]
    return settings.allow_insecure_localhost and host in LOOPBACK_HOSTS


async def bounded_body(request: Request, maximum: int, resource: str = "hub_request") -> bytes:
    """The request's body, refused past `maximum` bytes with what it was.

    `resource` names the body in the refusal: `capture`, `recording` or `audio`
    for a route of one of those (a chunk, a manifest, a WAV or a text to speak),
    `hub_request` for any other body.
    `bytes` is what had arrived when the ceiling was crossed, not the whole body.
    """
    body = bytearray()
    async for piece in request.stream():
        if len(body) + len(piece) > maximum:
            raise HubError(
                "too_large",
                "Payload too large",
                numbers={
                    "resource": resource,
                    "bytes": len(body) + len(piece),
                    "max_bytes": maximum,
                },
            )
        body.extend(piece)
    if not body:
        raise HubError("invalid_payload", "Empty payload")
    return bytes(body)


def validate_wav(body: bytes) -> None:
    try:
        with wave.open(io.BytesIO(body), "rb") as audio:
            channels, width, rate = audio.getnchannels(), audio.getsampwidth(), audio.getframerate()
            if channels not in {1, 2} or width != 2 or not 8000 <= rate <= 192000:
                raise ValueError("Unsupported PCM")
            expected = audio.getnframes() * channels * width
            if expected == 0 or len(audio.readframes(audio.getnframes())) != expected:
                raise ValueError("Truncated WAV")
    except (wave.Error, EOFError, ValueError) as exc:
        raise HubError("invalid_audio", "Valid 16-bit PCM WAV required") from exc


def install(app, settings: Settings, store) -> None:
    """The error handlers and the boundary every HTTP request goes through."""

    @app.exception_handler(HubError)
    async def hub_error(_request, exc):
        return http_response(
            exc.code,
            exc.message,
            exc.http_status,
            numbers=exc.numbers,
            payload=exc.display_metadata(),
        )

    @app.exception_handler(RequestValidationError)
    async def invalid_request(_request, _exc):
        # Pydantic's default errors echo submitted values (including pairing codes).
        return http_response("invalid_request", "Request violates the contract", 422)

    @app.middleware("http")
    async def boundary(request, call_next):
        # The one handler for whatever a route did not foresee, and the place
        # every HTTP error body gets the op of its request.
        op = bind_op(request.headers.get("x-orbit-op"))
        if not allowed_transport(request.scope, settings):
            return http_response("tls_required", "HTTPS required", 403, payload={"op": op})
        if request.headers.get("origin"):
            return http_response(
                "origin_forbidden", "API exclusive to the Orbit client", 403, payload={"op": op}
            )
        try:
            response = await call_next(request)
        except Exception as failure:  # noqa: BLE001 - redact arbitrary adapter/storage exceptions at HTTP boundary
            return unforeseen(request, failure, store, op)
        if response.status_code >= 400 and response.headers.get("content-type", "").startswith(
            "application/json"
        ):
            response = await with_op(response, op)
        response.headers["Cache-Control"] = "no-store"
        response.headers["X-Content-Type-Options"] = "nosniff"
        return response


def unforeseen(request, failure: Exception, store, op: str):
    """What a route did not foresee, without its text."""
    # The template, never the path: a path holds the ids of one request.
    route = getattr(request.scope.get("route"), "path", None)
    if isinstance(failure, ClientDisconnect):
        # The client hung up mid-body (a privacy stop cuts an upload): nobody
        # reads an answer, and it is not a hub failure.
        logger.info(
            "client_disconnected",
            extra={"event": "client_disconnected", "op": op, "route": route},
        )
        return Response(status_code=499)
    if is_storage_full(failure):
        # The disk is full: a cause someone can fix, said with its numbers.
        full = store.space.full()
        report("storage_full", "storage_full", route=route, status=full.http_status)
        return http_response(
            full.code, full.message, full.http_status, payload={"op": op}, numbers=full.numbers
        )
    report("internal_error", "internal_error", route=route, status=500)
    return http_response(
        "internal_error", "Server error; keep the local copy", 500, payload={"op": op}
    )


async def with_op(response, op: str):
    """An error body as a typed refusal, with its request's op."""
    body = b"".join([chunk async for chunk in response.body_iterator])
    headers = {
        key: value for key, value in response.headers.items() if key.lower() != "content-length"
    }
    try:
        existing = json.loads(body)
    except (ValueError, UnicodeError):
        existing = None
    code = existing.get("code") if isinstance(existing, dict) else None
    if code is None and response.status_code in {405, 422}:
        code = "invalid_request"
    if not (isinstance(existing, dict) and ("refusal" in existing or isinstance(code, str))):
        return Response(content=body, status_code=response.status_code, headers=headers)
    payload = existing
    if "refusal" not in existing:
        message = existing.get("message") or existing.get("notice") or code
        if not isinstance(message, str):
            message = code
        if "message" not in existing and "notice" not in existing:
            message = "Request violates the contract"
        payload = render(code, message, payload=existing)
    return JSONResponse({**payload, "op": op}, status_code=response.status_code, headers=headers)
