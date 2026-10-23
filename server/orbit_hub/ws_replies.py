"""A Mac's answer to a question an assistant put to it (`device.reply`).

Answered straight through: there is no upstream turn to reserve, and an
answer this device was never asked for is dropped rather than delivered to
whoever happens to be waiting on that id.
"""

from pydantic import ValidationError

from .log import report
from .protocol import DeviceReplyParams
from .ws_frames import Socket, error_event


def device_reply(socket: Socket, frame, request_id) -> None:
    raw_id = frame.params.get("request_id")
    if not isinstance(raw_id, str) or not 1 <= len(raw_id) <= 160:
        report(
            event="device_reply_invalid",
            code="invalid_request",
            device=socket.device,
            field="request_id",
            exc_info=False,
        )
        socket.queue.put_nowait(
            error_event(
                "invalid_request",
                "El campo request_id debe ser texto de 1 a 160 caracteres.",
                request_id,
                numbers={"field": "request_id", "expected": "texto de 1 a 160 caracteres"},
            )
        )
        return
    try:
        DeviceReplyParams.model_validate(frame.params)
    except ValidationError:
        _refuse_reply(socket, raw_id, request_id)
        return
    claimed = socket.bridge.resolve(raw_id, socket.device)
    if claimed is None:
        return
    waiter = socket.bridge_waiters.pop(raw_id, None)
    if waiter is not None and not waiter.done():
        waiter.set_result({**frame.params, "device_version": socket.device_version})


def _refuse_reply(socket: Socket, raw_id: str, request_id) -> None:
    """An answer outside the envelope: its question is refused, and the Mac told."""
    claimed = socket.bridge.resolve(raw_id, socket.device)
    waiter = socket.bridge_waiters.pop(raw_id, None) if claimed is not None else None
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
                "device_version": socket.device_version,
            }
        )
    socket.queue.put_nowait(
        error_event(
            "invalid_request",
            "La respuesta del Mac no cumple el sobre device.reply.",
            request_id,
            numbers={"field": "device.reply", "expected": "un resultado o una negativa tipada"},
        )
    )
