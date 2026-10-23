"""The authenticated device WebSocket: `/v1/ws`.

Composition only. The socket is accepted and its frames are read here
(`ws_reader`), answered in lanes (`ws_dispatch`) and a Mac's answers to the
assistants' questions routed (`ws_replies`); the frames the runtime queues for
the device are written by `outgoing`.
"""

import asyncio
from collections.abc import Callable
from contextlib import suppress
from dataclasses import dataclass

from fastapi import WebSocket

from .attachments import limits as attachment_limits
from .errors import HubError
from .log import report
from .protocol import HUB_PROTOCOL, emit
from .ws_frames import DISCONNECTED, MAX_DEVICE_VERSION_LENGTH, Socket
from .ws_reader import read_frames

# Four connections per device: the Mac keeps one, and a reconnection can open
# the next before the hub has seen the last one end (a send waits up to 10 s).
# Each costs a queue of up to 256 frames (runtime.subscribe). It protects the
# hub from one device opening sockets without end; why four, and not two, was
# not measured. It does not limit devices, turns or requests over HTTP.
MAX_SOCKETS_PER_DEVICE = 4
# How long one frame may take to reach the Mac before its slot is freed.
SEND_TIMEOUT_SECONDS = 10


@dataclass(frozen=True)
class Services:
    """What every socket of the hub uses."""

    settings: object
    agents: object
    identity: Callable
    allowed_transport: Callable
    runtime_for: Callable
    store: object
    now: Callable[[], float]
    bridge: object
    bridge_waiters: dict
    hub_version: Callable[[], str]


def mount(
    app,
    settings,
    agents,
    identity,
    allowed_transport,
    runtime_for,
    store,
    now,
    bridge,
    bridge_waiters,
    hub_version,
):
    services = Services(
        settings,
        agents,
        identity,
        allowed_transport,
        runtime_for,
        store,
        now,
        bridge,
        bridge_waiters,
        hub_version,
    )

    @app.websocket("/v1/ws")
    async def websocket(ws: WebSocket):
        # The socket's share of the global handler. A frame's failures are
        # answered in the reader; this catches what is left of the connection.
        try:
            socket = await accept(services, ws)
            if socket is not None:
                await serve(services, socket)
        except DISCONNECTED:
            return
        except Exception:  # noqa: BLE001 - never expose secret-bearing exceptions
            report("internal_error", "internal_error", route="/v1/ws")

    return websocket


async def accept(services: Services, ws: WebSocket) -> Socket | None:
    """The authenticated device's socket, or None when it was refused and closed."""
    try:
        if not services.allowed_transport(ws.scope, services.settings) or ws.headers.get("origin"):
            await ws.close(code=4403)
            return None
        device = services.identity(ws)
    except HubError:
        await ws.close(code=4401)
        return None
    await ws.accept()
    runtime = services.runtime_for(device)
    version = device_version(ws, device)
    if len(runtime.listeners) >= MAX_SOCKETS_PER_DEVICE:
        await ws.close(code=4429)
        return None
    queue = runtime.subscribe()
    services.store.devices.mark_seen(device, services.now())
    return Socket(
        ws,
        device,
        version,
        runtime,
        queue,
        services.store,
        services.settings,
        services.bridge,
        services.bridge_waiters,
    )


def ready(services: Services, socket: Socket) -> dict:
    agents = services.agents
    return emit(
        "ready",
        {
            "device_id": socket.device,
            "hub_protocol": HUB_PROTOCOL,
            "hub_version": services.hub_version(),
            # The primary's, for Macs from before agents (0.9.0), whose turns
            # name none and so go to it.
            "assistant_name": agents.by_id[agents.primary_id].name,
            # `agents`, `default_agent_id` (the primary) and `addable_agents`,
            # as `agents.changed` repeats them.
            **agents.listing(),
            # Absent from a 0.9.0 hub, which has no attachments: the Mac
            # offers none to it (contracts/hub-v1.md).
            "attachments": attachment_limits(),
            "device_version": socket.device_version,
            # Absent from a 0.10.1 hub: what the hub's storage uses, its limit
            # and what is left (contracts/hub-v1.md).
            "storage": services.store.space.status(),
        },
    )


async def serve(services: Services, socket: Socket) -> None:
    tasks = [
        asyncio.create_task(read_frames(socket)),
        asyncio.create_task(outgoing(socket, ready(services, socket))),
    ]
    try:
        with suppress(asyncio.CancelledError):
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    finally:
        socket.runtime.listeners.discard(socket.queue)
        services.store.devices.mark_seen(socket.device, services.now())
        if not socket.runtime.listeners:
            abandon(socket)
        await finish(socket, tasks)


def abandon(socket: Socket) -> None:
    """The Mac is gone. Its outstanding questions will never be answered, and
    letting them survive a reconnection would deliver a reply about a screen
    from before the gap."""
    for abandoned in socket.bridge.abandon(socket.device):
        waiter = socket.bridge_waiters.pop(abandoned.request_id, None)
        if waiter is not None and not waiter.done():
            waiter.set_result(
                {
                    "refusal": {
                        "code": "mac_disconnected",
                        "last_seen_ms": None,
                        "retry": {"kind": "check_outcome"},
                    },
                    "device_version": socket.device_version,
                }
            )


def device_version(ws: WebSocket, device: str) -> str | None:
    version = ws.headers.get("x-orbit-device-version")
    if version is not None and len(version) > MAX_DEVICE_VERSION_LENGTH:
        report(
            event="device_version_ignored",
            code="device_version_too_long",
            device=device,
            exc_info=False,
        )
        return None
    return version


async def outgoing(socket: Socket, ready: dict) -> None:
    await send_or_evict(socket, ready)
    while True:
        frame = await socket.queue.get()
        if frame is None:
            await socket.ws.close(code=4408)
            return
        await send_or_evict(socket, frame)
        if frame["event"] == "revoked":
            await socket.ws.close(code=4403)
            return


async def send_or_evict(socket: Socket, frame: dict) -> None:
    """Send one frame with a deadline; on timeout free the slot."""
    try:
        await asyncio.wait_for(socket.ws.send_json(frame), timeout=SEND_TIMEOUT_SECONDS)
    except TimeoutError:
        # A client that stops reading from the socket would hold its slot
        # forever. Free the slot and log the eviction; message bodies never
        # appear in the log line.
        report(event="slow_client_evicted", code="slow_client_evicted", device=socket.device)
        await socket.ws.close(code=4429)
        raise


async def finish(socket: Socket, tasks: list[asyncio.Task]) -> None:
    """Stop the socket's reader and writer; the lanes they started go on."""
    for task in tasks:
        task.cancel()
    with suppress(asyncio.CancelledError):
        await asyncio.gather(*tasks, return_exceptions=True)
    for task in tasks:
        if task.cancelled():
            continue
        failure = task.exception()
        if failure is not None and not isinstance(failure, (*DISCONNECTED, TimeoutError)):
            # TimeoutError is the eviction `send_or_evict` already reported.
            report("websocket_task_failed", "internal_error", device=socket.device, exc=failure)
