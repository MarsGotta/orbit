"""A final-message persistence failure never drops the Hermes connection."""

from uuid import uuid4

import pytest
from conftest import device, only, said
from support_runtime_completion_persistence import (
    drained,
    streaming_turn,
)
from support_runtime_completion_persistence import isolated as isolated  # noqa: PLC0414

from orbit_hub.protocol import HubError, SendParams
from orbit_hub.runtime import DeviceRuntime


async def test_quota_failure_completes_turn_without_upstream_loss(isolated):
    runtime, session, turn, upstream = await streaming_turn(isolated)
    queue = runtime.subscribe()
    assert queue.get_nowait()["event"] == "state.snapshot"
    runtime.store._quota = 1  # Admission ceiling: persisting the final message must fail.
    await runtime.connections["default"].callback(
        {
            "type": "message.complete",
            "session_id": upstream,
            "payload": {"text": "final synthetic reply", "status": "success"},
        }
    )
    frames = drained(queue)
    completed = next(frame for frame in frames if frame["event"] == "chat.completed")
    assert completed["id"] == turn
    assert completed["data"]["text"] == "final synthetic reply"
    assert completed["data"]["persisted"] is False
    assert completed["data"]["warning"] == "quota_exceeded"
    status = runtime.store.session(runtime.device, session)["status"]
    assert status not in {"streaming", "outcome_unknown"}
    assert runtime.connections["default"].connected
    assert all(frame["data"].get("code") != "outcome_unknown" for frame in frames)
    with pytest.raises(HubError) as error:
        await runtime.handle(
            "chat.send", SendParams(text="next explicit message", session_id=session), str(uuid4())
        )
    assert error.value.code == "quota_exceeded"
    await runtime.close()


async def test_healthy_persistence_reports_persisted_true(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    queue = runtime.subscribe()
    turn = str(uuid4())
    _, created = await runtime.handle("chat.send", SendParams(text="synthetic"), turn)
    completed = next(frame for frame in drained(queue) if frame["event"] == "chat.completed")
    assert completed["id"] == turn
    assert completed["data"]["persisted"] is True
    assert "warning" not in completed["data"]
    assert {"role": "assistant", "text": "respuesta sintética"} in said(
        runtime.store, owner, created["session_id"]
    )
    assert runtime.store.session(owner, created["session_id"])["status"] == "completed"
    await runtime.close()


@pytest.mark.parametrize("payload", [["not", "an", "object"], "plain-text-payload"])
async def test_non_object_payload_warns_and_keeps_connection(isolated, payload):
    runtime, session, _, upstream = await streaming_turn(isolated)
    queue = runtime.subscribe()
    assert queue.get_nowait()["event"] == "state.snapshot"
    for kind in ("message.delta", "message.complete"):
        await runtime.connections["default"].callback(
            {"type": kind, "session_id": upstream, "payload": payload}
        )
    frames = drained(queue)
    assert any(
        frame["event"] == "orbit.warning"
        or (frame["event"] == "chat.status" and frame["data"].get("kind") == "warning")
        for frame in frames
    )
    assert runtime.connections["default"].connected
    assert runtime.store.session(runtime.device, session)["status"] == "streaming"
    await runtime.close()
