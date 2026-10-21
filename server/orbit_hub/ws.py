"""Authenticated device WebSocket and method dispatch."""

import asyncio
import json
from contextlib import suppress
from enum import Enum

from fastapi import WebSocket, WebSocketDisconnect
from pydantic import ValidationError
from uvicorn.protocols.utils import ClientDisconnected

from . import safety
from .errors import HubError, render
from .log import bind_op, current_op, report
from .protocol import HUB_PROTOCOL, PARAMETERS, DeviceReplyParams, Frame, emit
from .turns import TurnState, can_start_turn


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


def error_event(code, message, request_id, *, status=None, numbers=None):
    payload = {"op": current_op()}
    if status is not None:
        payload["status"] = status
    return emit("error", render(code, message, payload=payload, numbers=numbers), request_id)


def mount(
    app,
    settings,
    identity,
    allowed_transport,
    runtime_for,
    store,
    now,
    bridge,
    bridge_waiters,
    hub_version,
):
    @app.websocket("/v1/ws")
    async def websocket(ws: WebSocket):
        # The socket's share of the global handler. A frame's failures are
        # answered in `incoming`; this catches what is left of the connection.
        try:
            await serve(ws)
        except DISCONNECTED:
            return
        except Exception:  # noqa: BLE001 - never expose secret-bearing exceptions
            report("internal_error", "internal_error", route="/v1/ws")

    async def serve(ws: WebSocket):
        try:
            if not allowed_transport(ws.scope, settings) or ws.headers.get("origin"):
                await ws.close(code=4403)
                return
            device = identity(ws)
        except HubError:
            await ws.close(code=4401)
            return
        await ws.accept()
        runtime = runtime_for(device)
        device_version = ws.headers.get("x-orbit-device-version")
        if device_version is not None and len(device_version) > MAX_DEVICE_VERSION_LENGTH:
            report(
                event="device_version_ignored",
                code="device_version_too_long",
                device=device,
                exc_info=False,
            )
            device_version = None
        if len(runtime.listeners) >= 4:
            await ws.close(code=4429)
            return
        queue = runtime.subscribe()
        store.mark_device_seen(device, now())
        # Submit lanes outlive the viewer that opened them: their receipts are
        # durable and their prompts must not be cancelled mid-flight.
        in_flight: set[asyncio.Task] = set()

        async def outgoing():
            await _send_or_evict(
                emit(
                    "ready",
                    {
                        "device_id": device,
                        "hub_protocol": HUB_PROTOCOL,
                        "hub_version": hub_version(),
                        "assistant_name": settings.assistant_name,
                        "device_version": device_version,
                    },
                )
            )
            while True:
                frame = await queue.get()
                if frame is None:
                    await ws.close(code=4408)
                    return
                await _send_or_evict(frame)
                if frame["event"] == "revoked":
                    await ws.close(code=4403)
                    return

        async def _send_or_evict(frame: dict) -> None:
            """Send one frame with a 10s deadline; on timeout free the slot."""
            try:
                await asyncio.wait_for(ws.send_json(frame), timeout=10)
            except TimeoutError:
                # A client that stops reading from the socket would hold its
                # slot forever. Free the slot and log the eviction; message
                # bodies never appear in the log line.
                report(
                    event="slow_client_evicted",
                    code="slow_client_evicted",
                    device=device,
                )
                await ws.close(code=4429)
                raise

        async def incoming():
            while True:
                raw = await ws.receive_text()
                request_id = None
                frame = None
                # A frame that does not parse still gets an op for its error.
                bind_op(None)
                try:
                    frame_bytes = len(raw.encode())
                    if frame_bytes > settings.max_frame_bytes:
                        raise HubError(
                            "too_large",
                            "Mensaje demasiado grande",
                            numbers={
                                "resource": "message",
                                "bytes": frame_bytes,
                                "max_bytes": settings.max_frame_bytes,
                            },
                        )
                    frame = Frame.model_validate_json(raw)
                    bind_op(frame.op if frame.op is not None else frame.id)
                    request_id = frame.uuid()
                    if not store.is_active(device):
                        raise HubError("revoked", "Dispositivo revocado")
                    if METHODS[frame.method] is MethodRoute.DEVICE_REPLY:
                        # Answered straight through: there is no upstream turn
                        # to reserve, and an answer this device was never asked
                        # for is dropped rather than delivered to whoever
                        # happens to be waiting on that id.
                        raw_id = frame.params.get("request_id")
                        if not isinstance(raw_id, str) or not 1 <= len(raw_id) <= 160:
                            report(
                                event="device_reply_invalid",
                                code="invalid_request",
                                device=device,
                                field="request_id",
                                exc_info=False,
                            )
                            queue.put_nowait(
                                error_event(
                                    "invalid_request",
                                    "El campo request_id debe ser texto de 1 a 160 caracteres.",
                                    request_id,
                                    numbers={
                                        "field": "request_id",
                                        "expected": "texto de 1 a 160 caracteres",
                                    },
                                )
                            )
                            continue
                        try:
                            DeviceReplyParams.model_validate(frame.params)
                        except ValidationError:
                            claimed = bridge.resolve(raw_id, device)
                            waiter = (
                                bridge_waiters.pop(raw_id, None) if claimed is not None else None
                            )
                            if waiter is not None and not waiter.done():
                                waiter.set_result(
                                    {
                                        "refusal": {
                                            "code": "device_reply_invalid",
                                            "field": "device.reply",
                                            "expected": "un resultado o una negativa tipada",
                                            "retry": {
                                                "kind": "check_outcome"
                                                if claimed.capability == "perform_action"
                                                else "after_fix"
                                            },
                                        },
                                        "device_version": device_version,
                                    }
                                )
                            queue.put_nowait(
                                error_event(
                                    "invalid_request",
                                    "La respuesta del Mac no cumple el sobre device.reply.",
                                    request_id,
                                    numbers={
                                        "field": "device.reply",
                                        "expected": "un resultado o una negativa tipada",
                                    },
                                )
                            )
                            continue
                        claimed = bridge.resolve(raw_id, device)
                        if claimed is None:
                            continue
                        waiter = bridge_waiters.pop(raw_id, None)
                        if waiter is not None and not waiter.done():
                            waiter.set_result({**frame.params, "device_version": device_version})
                        continue
                    params = PARAMETERS[frame.method].model_validate(frame.params)
                    mutating = frame.method != "ping"
                    safe_control = frame.method == "chat.interrupt" or (
                        frame.method == "approval.respond" and not params.approved
                    )
                    safety_target = None
                    if safe_control:
                        if not params.turn_id:
                            raise HubError(
                                "turn_id_required",
                                "El control requiere la identidad del turno original",
                            )
                        safety_target = safety.target_key(
                            frame.method,
                            params.session_id,
                            params.turn_id,
                            getattr(params, "request_id", ""),
                        )
                        live_targets = {
                            safety.target_key("chat.interrupt", session, turn)
                            for session, turn in runtime.turn_ids.items()
                            if not can_start_turn(
                                TurnState(store.session(device, session)["status"])
                            )
                        }
                        live_targets.update(
                            safety.target_key(
                                "approval.respond", owner, runtime.turn_ids[owner], interaction
                            )
                            for interaction, (owner, kind, _) in runtime.pending.items()
                            if kind == "approval" and owner in runtime.turn_ids
                        )
                        cached = safety.reserve(
                            store,
                            device,
                            request_id,
                            frame.model_dump(exclude={"op"}),
                            target=safety_target,
                            session_id=params.session_id,
                            turn_id=params.turn_id,
                            live_targets=live_targets,
                            validate=lambda method=frame.method, parameters=params: (
                                runtime.safety_target(method, parameters)
                            ),
                        )
                        if cached is not None:
                            queue.put_nowait(cached)
                            continue
                    elif mutating:
                        cached = store.reserve_operation(
                            device, request_id, frame.model_dump(exclude={"op"})
                        )
                        if cached is not None:
                            queue.put_nowait(cached)
                            continue

                    async def dispatch(
                        frame=frame,
                        params=params,
                        request_id=request_id,
                        safe_control=safe_control,
                        mutating=mutating,
                        safety_target=safety_target,
                    ) -> None:
                        """Answer one request frame with the reader loop's error contract."""
                        try:
                            try:
                                name, data = await runtime.handle(frame.method, params, request_id)
                                response = emit(name, data, request_id)
                            except HubError as exc:
                                if not safe_control:
                                    raise
                                response = error_event(
                                    exc.code,
                                    str(exc),
                                    request_id,
                                    status="error",
                                    numbers=exc.numbers,
                                )
                            except Exception:
                                if not safe_control:
                                    raise
                                report(
                                    event="outcome_unknown",
                                    code="outcome_unknown",
                                    device=device,
                                    session=request_id,
                                )
                                response = error_event(
                                    "outcome_unknown",
                                    "No se pudo completar el control; no se repetirá",
                                    request_id,
                                    status="error",
                                )
                            if safe_control:
                                safety.finish(store, device, safety_target, response)
                            elif mutating:
                                store.finish_operation(device, request_id, response)
                        except HubError as exc:
                            deliver(
                                error_event(exc.code, str(exc), request_id, numbers=exc.numbers)
                            )
                            return
                        except Exception:  # noqa: BLE001 - never expose secret-bearing exceptions
                            report(
                                event="internal_error",
                                code="internal_error",
                                device=device,
                                session=request_id,
                            )
                            deliver(
                                error_event(
                                    "internal_error",
                                    "No se pudo completar la operación; no se repetirá",
                                    request_id,
                                )
                            )
                            return
                        deliver(response)
                        # `chat.interrupt` is here for its refusals: a rejected or
                        # outcome-unknown interrupt changes no state, so `changed()`
                        # never fires, and without a fresh snapshot the daemon keeps
                        # discarding the turn it already hid.
                        if frame.method in {
                            "approval.respond",
                            "clarify.respond",
                            "chat.interrupt",
                        }:
                            await runtime.publish(emit("state.snapshot", runtime.snapshot()))

                    def deliver(response: dict, request_id=request_id) -> None:
                        try:
                            queue.put_nowait(response)
                        except asyncio.QueueFull:
                            # A viewer that stopped reading must not lose the
                            # durable receipt by taking the lane down with it.
                            report(
                                event="slow_client_evicted",
                                code="slow_client_evicted",
                                device=device,
                                session=request_id,
                                exc_info=False,
                            )

                    if METHODS[frame.method] is MethodRoute.SUBMIT_LANE:
                        # prompt.submit runs in its own lane so controls and
                        # pings are answered while Hermes has not acknowledged.
                        # The lane is never cancelled: its receipt is durable and
                        # a submit that may have reached Hermes is not retried.
                        lane = asyncio.create_task(dispatch())
                        in_flight.add(lane)
                        lane.add_done_callback(in_flight.discard)
                    else:
                        await dispatch()
                except (ValidationError, ValueError, json.JSONDecodeError):
                    if frame is not None and frame.method == "chat.send":
                        runtime.log_chat_rejection("invalid_request")
                    queue.put_nowait(
                        error_event("invalid_request", "Mensaje fuera de contrato", request_id)
                    )
                except HubError as exc:
                    if frame is not None and frame.method == "chat.send":
                        runtime.log_chat_rejection(exc.code, exc.numbers)
                    queue.put_nowait(
                        error_event(exc.code, str(exc), request_id, numbers=exc.numbers)
                    )
                except Exception:  # noqa: BLE001 - never expose secret-bearing upstream exceptions
                    if frame is not None and frame.method == "chat.send":
                        runtime.log_chat_rejection("internal_error")
                    report(
                        event="internal_error",
                        code="internal_error",
                        device=device,
                        session=request_id,
                    )
                    queue.put_nowait(
                        error_event(
                            "internal_error",
                            "No se pudo completar la operación; no se repetirá",
                            request_id,
                        )
                    )

        tasks = [asyncio.create_task(fn()) for fn in (incoming, outgoing)]
        try:
            with suppress(asyncio.CancelledError):
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        finally:
            runtime.listeners.discard(queue)
            store.mark_device_seen(device, now())
            if not runtime.listeners:
                # The Mac is gone. Its outstanding questions will never be
                # answered, and letting them survive a reconnection would
                # deliver a reply about a screen from before the gap.
                for abandoned in bridge.abandon(device):
                    waiter = bridge_waiters.pop(abandoned.request_id, None)
                    if waiter is not None and not waiter.done():
                        waiter.set_result(
                            {
                                "refusal": {
                                    "code": "mac_disconnected",
                                    "last_seen_ms": None,
                                    "retry": {"kind": "check_outcome"},
                                },
                                "device_version": device_version,
                            }
                        )
            for task in tasks:
                task.cancel()
            with suppress(asyncio.CancelledError):
                await asyncio.gather(*tasks, return_exceptions=True)
            for task in tasks:
                if task.cancelled():
                    continue
                failure = task.exception()
                if failure is not None and not isinstance(failure, (*DISCONNECTED, TimeoutError)):
                    # TimeoutError is the eviction `_send_or_evict` already reported.
                    report("websocket_task_failed", "internal_error", device=device, exc=failure)
