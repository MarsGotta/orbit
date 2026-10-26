"""Answering one request frame of a Mac, in its own lane.

A frame's reservation (its operation id or its safety target) is taken in
the read loop; its answer runs here, off the read loop: a `chat.send` in a
lane of its own, a control in its conversation's lane, after the controls of
that conversation read before it. Every lane delivers its answer and records
its durable receipt; none is cancelled when its socket closes.
"""

import asyncio
from collections.abc import Awaitable, Callable

from . import safety
from .errors import HubError
from .log import report
from .protocol import emit
from .storage import is_storage_full
from .ws_frames import Socket, error_event, failed

# The controls whose answer is followed by the runtime's snapshot.
_SNAPSHOT_AFTER = {"approval.respond", "clarify.respond", "chat.interrupt"}


async def dispatch(socket: Socket, frame, params, request_id, safety_target) -> None:
    """Answer one request frame with the reader loop's error contract."""
    safe_control = safety_target is not None
    mutating = frame.method not in {"ping", "chat.observe"}
    runtime = socket.runtime
    try:
        try:
            name, data = await runtime.handle(frame.method, params, request_id)
            response = emit(name, data, request_id)
        except HubError as exc:
            if not safe_control:
                release(socket, frame.method, request_id)
                raise
            response = error_event(
                exc.code, str(exc), request_id, status="error", numbers=exc.numbers
            )
        except Exception:
            if not safe_control:
                release(socket, frame.method, request_id)
                raise
            report(
                event="outcome_unknown",
                code="outcome_unknown",
                device=socket.device,
                session=request_id,
            )
            response = error_event(
                "outcome_unknown",
                "No se pudo completar el control; no se repetirá",
                request_id,
                status="error",
            )
        if safe_control:
            safety.finish(socket.store, socket.device, safety_target, response)
        elif mutating:
            socket.store.operations.finish(socket.device, request_id, response)
    except HubError as exc:
        deliver(
            socket, error_event(exc.code, str(exc), request_id, numbers=exc.numbers), request_id
        )
        return
    except Exception as failure:  # noqa: BLE001 - never expose secret-bearing exceptions
        deliver(socket, failed(socket, failure, request_id), request_id)
        return
    finally:
        runtime.forget_commit(request_id)
    deliver(socket, response, request_id)
    # `chat.interrupt` is here for its refusals: a rejected or outcome-unknown
    # interrupt changes no state, so `changed()` never fires, and without a
    # fresh snapshot the daemon keeps discarding the turn it already hid.
    if frame.method in _SNAPSHOT_AFTER:
        await runtime.publish(emit("state.snapshot", runtime.snapshot()))


def release(socket: Socket, method: str, request_id) -> None:
    """A message refused before the hub kept or sent anything gives its id
    back, so the Mac can send it again."""
    if method != "chat.send" or socket.runtime.committed(request_id):
        return
    try:
        socket.store.operations.release(socket.device, request_id)
    except Exception as failure:  # noqa: BLE001 - the refusal still goes out
        report(
            event="operation_release_failed",
            code="storage_full" if is_storage_full(failure) else "operation_release_failed",
            device=socket.device,
            session=request_id,
        )


def deliver(socket: Socket, response: dict, request_id) -> None:
    try:
        socket.queue.put_nowait(response)
    except asyncio.QueueFull:
        # A viewer that stopped reading must not lose the durable receipt by
        # taking the lane down with it.
        report(
            event="slow_client_evicted",
            code="slow_client_evicted",
            device=socket.device,
            session=request_id,
            exc_info=False,
        )


def in_lane(socket: Socket, answer: Callable[[], Awaitable[None]]) -> asyncio.Task:
    """Run one answer in a lane of its own, kept until it ends."""
    lane = asyncio.create_task(answer())
    socket.in_flight.add(lane)
    lane.add_done_callback(socket.in_flight.discard)
    return lane


def in_conversation_lane(
    socket: Socket, session_id: str, answer: Callable[[], Awaitable[None]]
) -> None:
    """Run a control after the controls of its conversation read before it,
    never in the read loop: its wait for the agent or for the conversation's
    admission holds only that conversation."""
    before = socket.controls.get(session_id)

    async def in_order() -> None:
        if before is not None:
            await asyncio.wait((before,))
        await answer()

    lane = in_lane(socket, in_order)
    socket.controls[session_id] = lane

    def forget(done: asyncio.Task) -> None:
        if socket.controls.get(session_id) is done:
            del socket.controls[session_id]

    lane.add_done_callback(forget)
