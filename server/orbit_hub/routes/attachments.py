"""The files a device attaches to its turns: upload and read back."""

import hashlib
import re
from urllib.parse import quote, unquote

from fastapi import Request
from fastapi.responses import Response
from pydantic import Field, ValidationError

from ..attachments import MAX_ATTACHMENT_BYTES, too_large_file, valid_name
from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..protocol import StrictModel


class CaptureAttachment(StrictModel):
    """«Adjuntar ventana»: a capture the Mac uploaded (`PUT /v1/captures`)
    kept as an attachment under `name`."""

    capture_id: str = Field(min_length=1, max_length=80)
    request_id: str = Field(min_length=1, max_length=160)
    name: str = Field(min_length=1, max_length=1024)


# The header that carries the file's name, percent-encoded UTF-8: a header is
# Latin-1 and a name is anything the user called the file.
NAME_HEADER = "x-orbit-attachment-name"


def mount(app, store, identity, bounded_body, store_executor, now, captures):
    @app.post("/v1/attachments")
    async def upload(request: Request):
        device = identity(request)
        name, body = await read_upload(request, bounded_body)
        if not store.devices.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        attachment = await in_store(
            store_executor, store.attachments.put, device, name, body, round(now() * 1000)
        )
        return {"attachment": attachment}

    @app.post("/v1/attachments/capture")
    async def from_capture(request: Request):
        """The window the Mac captured for «Adjuntar ventana», taken once
        from the capture store (as a bridge reply would take it) and kept as
        an attachment: the PNG crosses the network once."""
        device = identity(request)
        try:
            # 4 KiB holds every valid request: two ids and a 1,024-character name.
            ask = CaptureAttachment.model_validate_json(
                await bounded_body(request, 4 * 1024, "attachment")
            )
        except ValidationError as exc:
            raise HubError(
                "invalid_attachment",
                "La captura que adjuntar no se describió bien",
                numbers={"field": "capture"},
            ) from exc
        body = captures.take(device, ask.capture_id, ask.request_id)
        if body is None:
            raise HubError(
                "capture_missing",
                "La captura ya no está en el hub (caduca a los cinco minutos o ya se usó)",
                http_status=404,
                numbers={"capture_id": ask.capture_id},
            )
        attachment = await in_store(
            store_executor, store.attachments.put, device, ask.name, body, round(now() * 1000)
        )
        return {"attachment": attachment}

    @app.get("/v1/attachments/{attachment_id}")
    async def download(attachment_id: str, request: Request):
        device = identity(request)
        try:
            attachment_id = canonical_uuid(attachment_id)
        except ValueError as exc:
            raise HubError(
                "attachment_not_found",
                "Ese adjunto no está en el hub",
                http_status=404,
                numbers={"attachment_id": attachment_id[:80]},
            ) from exc
        stored = await in_store(store_executor, store.attachments.read, device, attachment_id)
        return Response(
            content=stored.body,
            media_type=stored.mime,
            headers={NAME_HEADER: quote(stored.name, safe="")},
        )


async def read_upload(request: Request, bounded_body) -> tuple[str, bytes]:
    """An upload's name and bytes, refused when either does not hold."""
    try:
        name = valid_name(unquote(request.headers[NAME_HEADER], errors="strict"))
        digest = request.headers["x-orbit-sha256"]
        if not re.fullmatch(r"[a-f0-9]{64}", digest):
            raise ValueError("Invalid digest")
    except (KeyError, ValueError, UnicodeDecodeError) as exc:
        raise HubError(
            "invalid_attachment",
            "Faltan el nombre o el resumen SHA-256 del adjunto, o no valen",
            numbers={"field": "headers"},
        ) from exc
    declared = request.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > MAX_ATTACHMENT_BYTES:
        # Refused before reading a byte, with the size the Mac declared.
        raise too_large_file(name, int(declared))
    try:
        body = await bounded_body(request, MAX_ATTACHMENT_BYTES, "attachment")
    except HubError as exc:
        if exc.code == "too_large":
            raise too_large_file(name, exc.numbers["bytes"]) from exc
        raise
    if hashlib.sha256(body).hexdigest() != digest:
        raise HubError(
            "invalid_attachment",
            f"El resumen de «{name}» no coincide con lo recibido; vuelve a adjuntarlo",
            numbers={"field": "x-orbit-sha256", "bytes": len(body)},
        )
    return name, body
