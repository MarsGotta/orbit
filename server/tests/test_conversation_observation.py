"""A conversation is observed without sending or interrupting its turn."""

from uuid import uuid4

import pytest
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414
from support_review_regressions import runtime_fixture

from orbit_hub.protocol import SendParams, SessionParams


@pytest.mark.parametrize("new", [False, True])
@pytest.mark.parametrize("kind", ["approval", "clarify"])
async def test_observe_recovers_left_turn_and_answer(reviewed, new, kind):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    connection = runtime.connections["default"]
    await connection.receive(
        {"type": "message.delta", "session_id": upstream, "payload": {"text": "Partial"}}
    )
    await connection.receive(
        {
            "type": kind + ".request",
            "session_id": upstream,
            "payload": {"request_id": "permit", "command": "Permit?", "question": "Permit?"},
        }
    )
    if new:
        await runtime.handle("chat.send", SendParams(text="Other question"), str(uuid4()))
    name, snapshot = await runtime.handle(
        "chat.observe", SessionParams(session_id=session), str(uuid4())
    )
    assert name == "state.snapshot"
    [observed] = snapshot["sessions"]
    assert observed["turn_id"] == turn
    assert observed["turn"]["answer"]["text"] == "Partial"
    assert (
        observed["pending"][0].get("description", observed["pending"][0].get("question"))
        == "Permit?"
    )
    assert not any(method == "session.interrupt" for method, _ in connection.calls)
    await connection.receive(
        {
            "type": "message.complete",
            "session_id": upstream,
            "payload": {"text": "Whole answer", "status": "success"},
        }
    )
    _, snapshot = await runtime.handle(
        "chat.observe", SessionParams(session_id=session), str(uuid4())
    )
    assert snapshot["sessions"][0]["status"] == "completed"
    page = runtime.store.chat.session_messages(runtime.device, session, None, 200)
    assert page["messages"][-1]["text"] == "Whole answer"


async def test_observation_enforces_device_ownership(reviewed):
    from conftest import device

    from orbit_hub.protocol import HubError

    runtime, _, _, _ = await runtime_fixture(reviewed)
    other = device(runtime.store, "Other Mac")["device_id"]
    foreign = runtime.store.chat.create_session(
        other, "default", "runtime", "stored", "epoch", 1000
    )
    with pytest.raises(HubError) as error:
        await runtime.handle("chat.observe", SessionParams(session_id=foreign), str(uuid4()))
    assert error.value.code == "session_not_found"


@pytest.mark.parametrize("progress", ["message.delta", "tool.start", "approval.request"])
async def test_progress_ends_a_provider_retry_wait(reviewed, progress):
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    connection = runtime.connections["default"]
    await connection.receive(
        {
            "type": "status.update",
            "session_id": upstream,
            "payload": {"kind": "retrying_openai", "text": "Retrying"},
        }
    )
    _, before = await runtime.handle(
        "chat.observe", SessionParams(session_id=session), str(uuid4())
    )
    assert before["sessions"][0]["retry"] == "retrying_openai"
    await connection.receive(
        {
            "type": progress,
            "session_id": upstream,
            "payload": {
                "text": "Answer",
                "tool_id": "step",
                "name": "mcp_orbit_orbit_look",
                "request_id": "permit",
                "command": "Permit?",
            },
        }
    )
    _, after = await runtime.handle("chat.observe", SessionParams(session_id=session), str(uuid4()))
    assert after["sessions"][0]["retry"] is None


async def test_live_preview_fits_mac_frame_and_final_history_keeps_the_whole_answer(reviewed):
    import json
    from pathlib import Path

    from orbit_hub.runtime.observation import MAX_PREVIEW_BYTES

    runtime, session, _, upstream = await runtime_fixture(reviewed)
    connection = runtime.connections["default"]
    text = "Invented answer. " * (MAX_PREVIEW_BYTES // 8)
    await connection.receive(
        {"type": "message.delta", "session_id": upstream, "payload": {"text": text}}
    )
    name, snapshot = await runtime.handle(
        "chat.observe", SessionParams(session_id=session), str(uuid4())
    )
    observed = snapshot["sessions"][0]["turn"]["answer"]["text"]
    assert len(observed.encode()) <= MAX_PREVIEW_BYTES < len(text.encode())
    constants = json.loads((Path(__file__).parents[2] / "contracts/constants.json").read_text())
    frame_limit = next(
        item["value"] for item in constants if item["name"] == "MAX_HUB_INCOMING_FRAME_BYTES"
    )
    assert (
        len(json.dumps({"v": 1, "id": str(uuid4()), "event": name, "data": snapshot}).encode())
        <= frame_limit
    )
    await connection.receive(
        {
            "type": "message.complete",
            "session_id": upstream,
            "payload": {"text": text, "status": "success"},
        }
    )
    assert (
        runtime.store.chat.session_messages(runtime.device, session, None, 200)["messages"][-1][
            "text"
        ]
        == text
    )
