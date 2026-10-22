"""Inverted regressions for vault review F1–F4; all upstreams are synthetic."""

import asyncio
import hashlib
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_review_regressions import (
    pending,
    receive_event,
    runtime_fixture,
    submit,
)
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414
from support_transcription import wav_fixture

from orbit_hub.protocol import ApprovalParams, ClarifyParams, HubError, SendParams


@pytest.mark.parametrize(
    "message",
    [
        "Turn cancelled before the agent was ready",
        "Session no longer running before the agent was ready",
        "Context injection refused.",
    ],
)
async def test_f1_terminal_error_without_complete_releases_session(reviewed, message):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    assert queue.get_nowait()["event"] == "state.snapshot"
    await runtime.connections["default"].receive(
        {"type": "error", "session_id": upstream, "payload": {"message": message}}
    )
    failed = queue.get_nowait()
    assert failed["event"] == "error" and failed["id"] == turn
    assert failed["data"]["code"] == "upstream_turn_failed"
    assert runtime.store.session(runtime.device, session)["status"] == "failed"
    await runtime.handle(
        "chat.send", SendParams(text="next explicit message", session_id=session), str(uuid4())
    )
    assert [method for method, _ in runtime.connections["default"].calls] == [
        "prompt.submit",
        "prompt.submit",
    ]
    await runtime.close()


@pytest.mark.parametrize("running", [True, False])
async def test_f1_ambiguous_error_is_reconciled_not_blindly_terminal(reviewed, running):
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    runtime.connections["default"].running = running
    await runtime.connections["default"].receive(
        {
            "type": "error",
            "session_id": upstream,
            "payload": {"message": "arbitrary context diagnostic"},
        }
    )
    await asyncio.gather(*list(runtime.reconciliations.values()))
    assert runtime.store.session(runtime.device, session)["status"] == (
        "streaming" if running else "failed"
    )
    assert runtime.connections["default"].calls[-1][0] == "session.status"
    assert all(method != "session.resume" for method, _ in runtime.connections["default"].calls)
    await runtime.close()


@pytest.mark.parametrize("kind", ["approval", "clarify"])
@pytest.mark.parametrize("disconnect_before", [True, False])
async def test_f2_reconnect_and_history_restore_pending_without_resurrection(
    reviewed, kind, disconnect_before
):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    old = runtime.subscribe()
    if disconnect_before:
        runtime.listeners.discard(old)
    await runtime.connections["default"].receive(
        {
            "type": kind + ".request",
            "session_id": upstream,
            "payload": {"request_id": "request", "question": "Question", "command": "Command"},
        }
    )
    runtime.listeners.discard(old)
    fresh = runtime.subscribe()
    snapshot = fresh.get_nowait()
    assert snapshot["event"] == "state.snapshot"
    current = snapshot["data"]["sessions"][0]
    assert current["session_id"] == session and current["turn_id"] == turn
    assert current["pending"][0]["request_id"] == "request"
    assert current["pending"][0]["kind"] == kind and current["pending"][0]["expires_at"] is None
    # The snapshot a reconnecting Mac reads is the one subscribers get.
    assert (
        next(item for item in runtime.snapshot()["sessions"] if item["session_id"] == session)
        == current
    )
    await runtime.connections["default"].receive(
        {"type": kind + ".expire", "session_id": upstream, "payload": {"request_id": "request"}}
    )
    assert fresh.get_nowait()["event"] == "interaction.expired"
    cleared = fresh.get_nowait()["data"]
    assert cleared["revision"] > snapshot["data"]["revision"]
    assert cleared["sessions"][0]["pending"] == []
    assert runtime.subscribe().get_nowait()["data"]["sessions"][0]["pending"] == []
    await runtime.close()


@pytest.mark.parametrize("resolved", [0, 1, 2, True, None])
async def test_f3_approval_ack_requires_exact_individual_resolution(reviewed, resolved):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    await runtime.connections["default"].receive(
        {"type": "approval.request", "session_id": upstream, "payload": {"request_id": "request"}}
    )
    runtime.connections["default"].approval_result = {"resolved": resolved}
    params = ApprovalParams(session_id=session, request_id="request", approved=True, turn_id=turn)
    if type(resolved) is int and resolved in {0, 1}:
        name, data = await runtime.handle("approval.respond", params, str(uuid4()))
        assert name == ("interaction.resolved" if resolved else "interaction.expired")
        assert data["applied"] is (resolved == 1)
    else:
        with pytest.raises(HubError) as error:
            await runtime.handle("approval.respond", params, str(uuid4()))
        assert error.value.code == "upstream_protocol"
    assert not runtime.pending
    await runtime.close()


@pytest.mark.parametrize(
    "result,expected",
    [
        ({"status": "expired"}, "interaction.expired"),
        ({"status": "ok", "remaining": ["q2"]}, "interaction.resolved"),
    ],
)
async def test_f3_clarify_preserves_its_own_remaining_contract(reviewed, result, expected):
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    await runtime.connections["default"].receive(
        {
            "type": "clarify.request",
            "session_id": upstream,
            "payload": {
                "request_id": "request",
                "questions": [
                    {"qid": "q1", "question": "first"},
                    {"qid": "q2", "question": "second"},
                ],
            },
        }
    )
    runtime.connections["default"].clarify_result = result
    name, data = await runtime.handle(
        "clarify.respond",
        ClarifyParams(session_id=session, request_id="request", answer="fixture"),
        str(uuid4()),
    )
    assert name == expected and data["applied"] is (result["status"] == "ok")
    assert bool(runtime.pending) is bool(result.get("remaining"))
    if result.get("remaining"):
        snapshot = runtime.subscribe().get_nowait()["data"]["sessions"][0]
        assert snapshot["pending"][0]["questions"] == [{"qid": "q2", "question": "second"}]
    await runtime.close()


def test_f4_quota_blocks_content_but_not_bound_interrupt_or_deny(reviewed):
    _, store, adapter = reviewed
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(reviewed) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        pending(http, connection)
        assert receive_event(ws)["event"] == "approval.required"
        store._quota = 1  # Admission ceiling, never physical disk exhaustion.
        interrupt = {
            "v": 1,
            "id": str(uuid4()),
            "method": "chat.interrupt",
            "params": {"session_id": session, "turn_id": turn},
        }
        for command in (interrupt, interrupt, {**interrupt, "id": str(uuid4())}):
            ws.send_json(command)
            receipt = receive_event(ws)
            assert receipt["event"] == "chat.interrupted" and receipt["id"] == command["id"]
        deny = {
            "v": 1,
            "id": str(uuid4()),
            "method": "approval.respond",
            "params": {
                "session_id": session,
                "turn_id": turn,
                "request_id": "fixture",
                "approved": False,
            },
        }
        for command in (deny, deny):
            ws.send_json(command)
            assert receive_event(ws)["data"]["applied"] is True
        ws.send_json(
            {"v": 1, "id": str(uuid4()), "method": "chat.send", "params": {"text": "BLOCKED"}}
        )
        assert receive_event(ws)["data"]["code"] == "quota_exceeded"
        body = wav_fixture()
        response = http.put(
            "/v1/chunks/new",
            headers={
                **headers,
                "X-Orbit-Recording": str(uuid4()),
                "X-Orbit-Track": "mic_local",
                "X-Orbit-Sequence": "0",
                "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
            },
            content=body,
        )
        assert response.status_code == 507
        assert [method for method, _ in connection.calls] == [
            "prompt.submit",
            "session.interrupt",
            "approval.respond",
        ]
        ws.send_json({**interrupt, "params": {"session_id": session, "turn_id": str(uuid4())}})
        assert receive_event(ws)["data"]["code"] == "operation_conflict"


def test_f4_more_than_128_resolved_denials_do_not_consume_interrupt_capacity(reviewed):
    _, store, adapter = reviewed
    paired = device(store)
    with (
        client(reviewed) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        store._quota = 1
        for index in range(140):
            request = f"fixture-{index}"
            pending(http, connection, request)
            assert receive_event(ws)["event"] == "approval.required"
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "approval.respond",
                    "params": {
                        "session_id": session,
                        "turn_id": turn,
                        "request_id": request,
                        "approved": False,
                    },
                }
            )
            assert receive_event(ws)["data"]["applied"] is True
        ws.send_json(
            {
                "v": 1,
                "id": str(uuid4()),
                "method": "chat.interrupt",
                "params": {"session_id": session, "turn_id": turn},
            }
        )
        assert receive_event(ws)["event"] == "chat.interrupted"
        assert connection.calls[-1][0] == "session.interrupt"
        assert store._db.execute("SELECT count(*) FROM safety_operations").fetchone()[0] == 1
