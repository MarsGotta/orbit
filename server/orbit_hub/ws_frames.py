"""What a Mac's socket carries: its methods, its connection and its error frames."""

import asyncio
from dataclasses import dataclass, field
from enum import Enum

from fastapi import WebSocket, WebSocketDisconnect
from uvicorn.protocols.utils import ClientDisconnected

from .errors import render
from .log import current_op, report
from .protocol import emit
from .storage import is_storage_full


class MethodRoute(Enum):
    DEVICE_REPLY = "device_reply"
    RUNTIME = "runtime"
    SUBMIT_LANE = "submit_lane"


METHODS = {
    "chat.send": MethodRoute.SUBMIT_LANE,
    "chat.interrupt": MethodRoute.RUNTIME,
    "approval.respond": MethodRoute.RUNTIME,
    "clarify.respond": MethodRoute.RUNTIME,
    "ping": MethodRoute.RUNTIME,
    "device.reply": MethodRoute.DEVICE_REPLY,
}

# How a socket ends when the Mac goes away, not a failure to report: Starlette
# says so once accepted, uvicorn's own error says so before.
DISCONNECTED = (WebSocketDisconnect, ClientDisconnected)

# A version label is display metadata, not a frame or capture limit. The
# published "0.3.12" is 6 characters; 128 leaves room for a long build suffix
# while bounding each label retained per device and echoed in ready events.
MAX_DEVICE_VERSION_LENGTH = 128


@dataclass
class Socket:
    """One Mac's connection: what each frame read from it needs."""

    ws: WebSocket
    device: str
    device_version: str | None
    runtime: object
    queue: asyncio.Queue
    store: object
    settings: object
    bridge: object
    bridge_waiters: dict
    # Lanes outlive the viewer that opened them: their receipts are durable
    # and their prompts must not be cancelled mid-flight.
    in_flight: set[asyncio.Task] = field(default_factory=set)
    # The last control lane of each conversation, which the next one follows.
    controls: dict[str, asyncio.Task] = field(default_factory=dict)


def error_event(code, message, request_id, *, status=None, numbers=None):
    payload = {"op": current_op()}
    if status is not None:
        payload["status"] = status
    return emit("error", render(code, message, payload=payload, numbers=numbers), request_id)


def failed(socket: Socket, failure: Exception, request_id) -> dict:
    """The error for a failure no refusal named: the full disk with its
    numbers, or the generic one. Reported either way, never its text."""
    if is_storage_full(failure):
        full = socket.store.space.full()
        report(event="storage_full", code="storage_full", device=socket.device, session=request_id)
        return error_event(full.code, full.message, request_id, numbers=full.numbers)
    report(event="internal_error", code="internal_error", device=socket.device, session=request_id)
    return error_event(
        "internal_error", "No se pudo completar la operación; no se repetirá", request_id
    )
