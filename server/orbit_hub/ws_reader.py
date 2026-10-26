"""The read loop of a Mac's socket.

It reads one frame at a time and never waits for one to be answered: it
checks the frame, takes its reservation (its operation id, or the safety
target of an interrupt or a denial), then hands the answer to a lane
(`ws_dispatch`). Only a `ping` and a `device.reply` are answered in the loop,
because neither waits for anything.
"""

import json

from pydantic import ValidationError

from . import safety
from .errors import HubError
from .log import bind_op
from .protocol import PARAMETERS, Frame
from .turns import TurnState, can_start_turn
from .ws_dispatch import dispatch, in_conversation_lane, in_lane
from .ws_frames import METHODS, MethodRoute, Socket, error_event, failed
from .ws_replies import device_reply


async def read_frames(socket: Socket) -> None:
    while True:
        raw = await socket.ws.receive_text()
        request_id = None
        frame = None
        # A frame that does not parse still gets an op for its error.
        bind_op(None)
        try:
            frame = _parse(socket, raw)
            bind_op(frame.op if frame.op is not None else frame.id)
            request_id = frame.uuid()
            if not socket.store.devices.is_active(socket.device):
                raise HubError("revoked", "Dispositivo revocado")
            if METHODS[frame.method] is MethodRoute.DEVICE_REPLY:
                device_reply(socket, frame, request_id)
                continue
            await _request(socket, frame, request_id)
        except (ValidationError, ValueError, json.JSONDecodeError):
            if frame is not None and frame.method == "chat.send":
                socket.runtime.log_chat_rejection("invalid_request")
            socket.queue.put_nowait(
                error_event("invalid_request", "Mensaje fuera de contrato", request_id)
            )
        except HubError as exc:
            if frame is not None and frame.method == "chat.send":
                socket.runtime.log_chat_rejection(exc.code, exc.numbers)
            socket.queue.put_nowait(
                error_event(exc.code, str(exc), request_id, numbers=exc.numbers)
            )
        except Exception as failure:  # noqa: BLE001 - never expose secret-bearing upstream exceptions
            response = failed(socket, failure, request_id)
            if frame is not None and frame.method == "chat.send":
                socket.runtime.log_chat_rejection(response["data"]["code"])
            socket.queue.put_nowait(response)


def _parse(socket: Socket, raw: str) -> Frame:
    frame_bytes = len(raw.encode())
    if frame_bytes > socket.settings.max_frame_bytes:
        raise HubError(
            "too_large",
            "Mensaje demasiado grande",
            numbers={
                "resource": "message",
                "bytes": frame_bytes,
                "max_bytes": socket.settings.max_frame_bytes,
            },
        )
    return Frame.model_validate_json(raw)


async def _request(socket: Socket, frame, request_id) -> None:
    """Reserve one request and hand its answer to its lane."""
    params = PARAMETERS[frame.method].model_validate(frame.params)
    safe_control = frame.method == "chat.interrupt" or (
        frame.method == "approval.respond" and not params.approved
    )
    safety_target = None
    if safe_control:
        if not params.turn_id:
            raise HubError(
                "turn_id_required", "El control requiere la identidad del turno original"
            )
        safety_target = safety.target_key(
            frame.method, params.session_id, params.turn_id, getattr(params, "request_id", "")
        )
        cached = _reserve_control(socket, frame, params, request_id, safety_target)
    elif frame.method not in {"ping", "chat.observe"}:
        cached = socket.store.operations.reserve(
            socket.device, request_id, frame.model_dump(exclude={"op"})
        )
    else:
        cached = None
    if cached is not None:
        socket.queue.put_nowait(cached)
        return

    def answer():
        return dispatch(socket, frame, params, request_id, safety_target)

    if METHODS[frame.method] is MethodRoute.SUBMIT_LANE:
        # The prompt runs in its own lane so controls and pings are answered
        # while the agent has not acknowledged it. The lane is never
        # cancelled: its receipt is durable and a submit that may have
        # reached the agent is not retried.
        in_lane(socket, answer)
    elif frame.method in {"ping", "chat.observe"}:
        await answer()
    else:
        in_conversation_lane(socket, params.session_id, answer)


def _reserve_control(socket: Socket, frame, params, request_id, target) -> dict | None:
    """The durable reservation of an interrupt or a denial, by its target."""
    runtime, store, device = socket.runtime, socket.store, socket.device
    live_targets = {
        safety.target_key("chat.interrupt", session, turn)
        for session, turn in runtime.turn_ids.items()
        if not can_start_turn(TurnState(store.chat.session(device, session)["status"]))
    }
    live_targets.update(
        safety.target_key("approval.respond", owner, runtime.turn_ids[owner], interaction)
        for interaction, (owner, kind, _) in runtime.pending.items()
        if kind == "approval" and owner in runtime.turn_ids
    )
    return safety.reserve(
        store,
        device,
        request_id,
        frame.model_dump(exclude={"op"}),
        target=target,
        session_id=params.session_id,
        turn_id=params.turn_id,
        live_targets=live_targets,
        validate=lambda method=frame.method, parameters=params: runtime.safety_target(
            method, parameters
        ),
    )
