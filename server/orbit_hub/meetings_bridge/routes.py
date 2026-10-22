"""Authenticated private meeting reads for Deimos."""

from functools import partial

from ..errors import render
from ..private_route import json_response, private_route
from .models import ATTACHMENT_PATH, LIST_PATH, NOTE_PATH, REQUEST_BYTES, TRANSCRIPT_PATH


def failure(exc):
    message = (
        "Solicitud fuera de contrato"
        if exc.code == "invalid_request"
        else "No se pudo completar la petición"
    )
    return json_response(render(exc.code, message, numbers=exc.numbers), exc.status)


def mount(app, service, transport_allowed):
    route = partial(
        private_route,
        app=app,
        transport_allowed=transport_allowed,
        failure=failure,
        strict_headers=True,
        methods=["POST", "GET", "PUT", "PATCH", "DELETE"],
    )
    route(LIST_PATH, service.list_meetings, REQUEST_BYTES)
    route(TRANSCRIPT_PATH, service.transcript, REQUEST_BYTES)
    route(NOTE_PATH, service.note, REQUEST_BYTES)
    route(ATTACHMENT_PATH, service.attachment, REQUEST_BYTES)
