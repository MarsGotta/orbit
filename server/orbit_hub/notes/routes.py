"""Authenticated HTTP for the note of a recording and the note templates."""

import re

from fastapi import Request
from fastapi.responses import JSONResponse

from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..meetings.ownership import device_for
from .custom_templates import LinkRequest, TemplateDraft, UnlinkRequest
from .http import parse
from .models import MAX_TEMPLATE_ID_CHARS, TEMPLATE_ID_PATTERN, EditRequest, GenerateRequest

# A request carries the inputs of one note: private notes (≤ 64 Ki
# characters, up to four UTF-8 bytes each), names and the event. 512 KiB holds
# the largest valid one; anything bigger is refused with its size.
MAX_REQUEST_BYTES = 512 * 1024


def mount(app, worker, identity, bounded_body, store_executor, now, recording_changed):
    repository = worker.repository

    async def run(function, *args):
        return await in_store(store_executor, function, *args)

    def recording_of(value: str) -> str:
        try:
            return canonical_uuid(value)
        except ValueError:
            raise HubError("invalid_note_request", "Invalid recording identifier") from None

    async def recording_device(request, recording):
        caller = identity(request)

        def reading():
            with repository.store.read_snapshot() as db:
                return device_for(db, recording, caller)

        return await run(reading)

    async def parsed(request: Request, model):
        body = await bounded_body(request, MAX_REQUEST_BYTES)
        return parse(body, model, "invalid_note_request", "Note request")

    @app.post("/v1/recordings/{recording_id}/note")
    async def request_note(recording_id: str, request: Request):
        recording = recording_of(recording_id)
        device = await recording_device(request, recording)
        payload = await parsed(request, GenerateRequest)
        document = await run(
            lambda: repository.request(
                device, recording, now(), worker.configured, template_id=payload.template_id
            )
        )
        worker.signal.set()
        recording_changed(device, recording)
        return JSONResponse(document, status_code=202)

    @app.get("/v1/recordings/{recording_id}/note")
    async def read_note(recording_id: str, request: Request):
        recording = recording_of(recording_id)
        device = await recording_device(request, recording)
        return JSONResponse(await run(repository.document, device, recording))

    @app.put("/v1/recordings/{recording_id}/note/edits")
    async def edit_note(recording_id: str, request: Request):
        recording = recording_of(recording_id)
        device = await recording_device(request, recording)
        payload = await parsed(request, EditRequest)
        document = await run(repository.edit, device, recording, payload, now())
        recording_changed(device, recording)
        return JSONResponse(document)

    template_routes(app, worker, identity, bounded_body, run)


def template_id_of(value: str) -> str:
    if len(value) > MAX_TEMPLATE_ID_CHARS or re.fullmatch(TEMPLATE_ID_PATTERN, value) is None:
        raise HubError(
            "invalid_note_template",
            f"Invalid template identifier: lowercase letters, digits, «-» or «_», up to {MAX_TEMPLATE_ID_CHARS} caracteres",
            numbers={"field": "id", "max": MAX_TEMPLATE_ID_CHARS},
        )
    return value


def template_routes(app, worker, identity, bounded_body, run):
    """The user's own templates and their assignment to a series."""
    templates = worker.templates

    async def body_of(request: Request, model):
        body = await bounded_body(request, MAX_REQUEST_BYTES)
        return parse(body, model, "invalid_note_template", "Plantilla")

    @app.get("/v1/note-templates")
    async def list_templates(request: Request):
        return JSONResponse(await run(templates.listing, identity(request)))

    @app.put("/v1/note-templates/{template_id}")
    async def save_template(template_id: str, request: Request):
        device, chosen = identity(request), template_id_of(template_id)
        draft = await body_of(request, TemplateDraft)
        template, created = await run(templates.put, device, chosen, draft, worker.now())
        return JSONResponse(template, status_code=201 if created else 200)

    @app.delete("/v1/note-templates/{template_id}")
    async def delete_template(template_id: str, request: Request):
        device = identity(request)
        return JSONResponse(await run(templates.delete, device, template_id_of(template_id)))

    @app.get("/v1/note-template-links")
    async def list_links(request: Request):
        return JSONResponse(await run(templates.links, identity(request)))

    @app.put("/v1/note-template-links")
    async def save_link(request: Request):
        device = identity(request)
        link = await body_of(request, LinkRequest)
        return JSONResponse(await run(templates.link, device, link, worker.now()))

    @app.delete("/v1/note-template-links")
    async def delete_link(request: Request):
        device = identity(request)
        unlink = await body_of(request, UnlinkRequest)
        return JSONResponse(await run(templates.unlink, device, unlink.calendar_item_id))
