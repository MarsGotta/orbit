"""Synthetic RPC replies interleaved with real runtime events, never live Hermes."""

import asyncio
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_review_regressions import ControlledConnection, runtime_fixture, submit

from orbit_hub.protocol import ApprovalParams, ClarifyParams, HubError


@pytest.fixture
def race_environment(environment, monkeypatch):
    _, _, adapter = environment

    def connection(callback):
        value = ControlledConnection(callback)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


@pytest.fixture
async def race_runtime(race_environment):
    runtime, session, turn, upstream = await runtime_fixture(race_environment)
    try:
        yield runtime, session, turn, upstream
    finally:
        await runtime.close()


async def batch(runtime, upstream, *, request="batch", questions=None):
    await runtime.on_upstream(
        {
            "type": "clarify.request",
            "session_id": upstream,
            "payload": {
                "request_id": request,
                "questions": questions
                if questions is not None
                else [
                    {"qid": "q1", "question": "first"},
                    {"qid": "q2", "question": "second"},
                ],
            },
        }
    )


def paused_reply(runtime, monkeypatch, result, *, command="clarify.respond"):
    arrived, release = asyncio.Event(), asyncio.Event()
    original = runtime.connection.rpc

    async def rpc(method, params):
        if method != command:
            return await original(method, params)
        runtime.connection.calls.append((method, params))
        arrived.set()
        await release.wait()
        return result

    monkeypatch.setattr(runtime.connection, "rpc", rpc)
    return arrived, release


def answer(runtime, session, *, question="q1"):
    return asyncio.create_task(
        runtime.handle(
            "clarify.respond",
            ClarifyParams(
                session_id=session, request_id="batch", question_id=question, answer="yes"
            ),
            str(uuid4()),
        )
    )


@pytest.mark.parametrize("terminal", ["clarify.expire", "message.complete"])
async def test_partial_reply_cannot_restore_terminal_interaction(
    race_runtime, monkeypatch, terminal
):
    runtime, session, turn, upstream = race_runtime
    await batch(runtime, upstream)
    arrived, release = paused_reply(runtime, monkeypatch, {"status": "ok", "remaining": ["q2"]})
    task = answer(runtime, session)
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        payload = (
            {"request_id": "batch"}
            if terminal == "clarify.expire"
            else {
                "status": "success",
                "text": "synthetic completed",
            }
        )
        # The reader must progress while the command is waiting for its RPC reply.
        await asyncio.wait_for(
            runtime.on_upstream(
                {
                    "type": terminal,
                    "session_id": upstream,
                    "payload": payload,
                }
            ),
            2,
        )
        assert "batch" not in runtime.pending  # Prove the terminal interleaving actually happened.
        release.set()
        name, receipt = await asyncio.wait_for(task, 2)
        assert name == "interaction.resolved"
        assert receipt["status"] == "ok" and receipt["applied"] is True
        assert receipt["remaining"] == ["q2"]  # Historical RPC fact, not live pending state.
        assert receipt["stale_interaction"] is True and receipt["turn_id"] == turn
        snapshot = runtime.subscribe().get_nowait()["data"]["sessions"][0]
        current = next(
            item for item in runtime.snapshot()["sessions"] if item["session_id"] == session
        )
        assert snapshot["pending"] == []
        assert current["pending"] == []
        assert snapshot["status"] == (
            "completed" if terminal == "message.complete" else "streaming"
        )
        assert [method for method, _ in runtime.connection.calls].count("clarify.respond") == 1
    finally:
        release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)


@pytest.mark.parametrize("identical", [False, True])
@pytest.mark.parametrize(
    "result",
    [
        {"status": "ok", "remaining": ["q2"]},
        {"status": "ok"},
        {"status": "expired"},
        {"status": "malformed"},
    ],
)
async def test_replaced_request_survives_old_partial_full_expired_and_invalid_ack(
    race_runtime, monkeypatch, identical, result
):
    runtime, session, turn, upstream = race_runtime
    await batch(runtime, upstream)
    original = runtime.pending["batch"]
    arrived, release = paused_reply(runtime, monkeypatch, result)
    task = answer(runtime, session)
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        await batch(
            runtime,
            upstream,
            questions=None if identical else [{"qid": "new", "question": "replacement"}],
        )
        replacement = runtime.pending["batch"]
        assert replacement is not original and (replacement == original) is identical
        release.set()
        if result["status"] == "malformed":
            with pytest.raises(HubError, match="no confirmó"):
                await asyncio.wait_for(task, 2)
        else:
            _, receipt = await asyncio.wait_for(task, 2)
            assert receipt["applied"] is (result["status"] == "ok")
            assert receipt["stale_interaction"] is True and receipt["turn_id"] == turn
        assert runtime.pending["batch"] is replacement
        current = runtime.subscribe().get_nowait()["data"]["sessions"][0]
        assert current["pending"][0]["questions"] == replacement[2]["questions"]
    finally:
        release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)


@pytest.mark.parametrize("change", ["turn", "connection", "epoch", "disconnected", "revoked"])
async def test_rpc_requires_same_turn_connection_epoch_and_authority_after_await(
    race_runtime, monkeypatch, change
):
    runtime, session, turn, upstream = race_runtime
    await batch(runtime, upstream)
    original = runtime.pending["batch"]
    arrived, release = paused_reply(runtime, monkeypatch, {"status": "ok", "remaining": ["q2"]})
    task = answer(runtime, session)
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        if change == "turn":
            runtime.turn_ids[session] = str(uuid4())
        elif change == "connection":
            replacement = ControlledConnection(runtime.on_upstream)
            await replacement.open()
            replacement.live_sessions.add(upstream)
            runtime.connection = replacement
        elif change == "epoch":
            runtime.connection.epoch = "another-synthetic-epoch"
        elif change == "disconnected":
            runtime.connection.connected = False
        else:
            runtime.store.revoke(runtime.device)
        release.set()
        _, receipt = await asyncio.wait_for(task, 2)
        assert receipt["stale_interaction"] is True and receipt["turn_id"] == turn
        assert receipt["applied"] is True
        assert (
            runtime.pending["batch"] is original
        )  # These white-box scope changes did not replace it.
    finally:
        release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)


async def test_unrelated_request_does_not_make_a_valid_partial_ack_stale(race_runtime, monkeypatch):
    runtime, session, _, upstream = race_runtime
    await batch(runtime, upstream)
    arrived, release = paused_reply(runtime, monkeypatch, {"status": "ok", "remaining": ["q2"]})
    task = answer(runtime, session)
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        await batch(runtime, upstream, request="other")
        other = runtime.pending["other"]
        release.set()
        _, receipt = await asyncio.wait_for(task, 2)
        assert "stale_interaction" not in receipt
        assert runtime.pending["batch"][2]["questions"] == [{"qid": "q2", "question": "second"}]
        assert runtime.pending["other"] is other
    finally:
        release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)


async def test_two_responses_serialize_without_blocking_rpc_reader(race_runtime, monkeypatch):
    runtime, session, _, upstream = race_runtime
    await batch(runtime, upstream)
    arrived, release = asyncio.Event(), asyncio.Event()
    calls = []

    async def rpc(method, params):
        assert method == "clarify.respond"
        calls.append(params["question_id"])
        if len(calls) == 1:
            arrived.set()
            await release.wait()
            return {"status": "ok", "remaining": ["q2"]}
        assert runtime.pending["batch"][2]["questions"] == [{"qid": "q2", "question": "second"}]
        return {"status": "ok"}

    monkeypatch.setattr(runtime.connection, "rpc", rpc)
    first = answer(runtime, session)
    tasks = [first]
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        tasks.append(answer(runtime, session, question="q2"))
        await asyncio.wait_for(batch(runtime, upstream, request="unrelated"), 2)
        release.set()
        results = await asyncio.wait_for(asyncio.gather(*tasks), 2)
        assert calls == ["q1", "q2"]
        assert all(
            receipt["applied"] and "stale_interaction" not in receipt for _, receipt in results
        )
        assert "batch" not in runtime.pending and "unrelated" in runtime.pending
    finally:
        release.set()
        for task in tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


async def test_shared_approval_branch_does_not_consume_replacement(race_runtime, monkeypatch):
    runtime, session, turn, upstream = race_runtime
    request = {
        "type": "approval.request",
        "session_id": upstream,
        "payload": {"request_id": "batch"},
    }
    await runtime.on_upstream(request)
    arrived, release = paused_reply(
        runtime, monkeypatch, {"resolved": 1}, command="approval.respond"
    )
    task = asyncio.create_task(
        runtime.handle(
            "approval.respond",
            ApprovalParams(session_id=session, request_id="batch", approved=True, turn_id=turn),
            str(uuid4()),
        )
    )
    try:
        await asyncio.wait_for(arrived.wait(), 2)
        await runtime.on_upstream(request)
        replacement = runtime.pending["batch"]
        release.set()
        _, receipt = await asyncio.wait_for(task, 2)
        assert receipt["applied"] is True and receipt["resolved"] == 1
        assert receipt["stale_interaction"] is True and receipt["turn_id"] == turn
        assert runtime.pending["batch"] is replacement
    finally:
        release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)


def test_websocket_stale_ack_precedes_current_snapshot_and_stays_marked_on_cached_retry(
    race_environment, monkeypatch
):
    _, store, adapter = race_environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(race_environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        runtime = http.app.state.device_bridge.runtimes[paired["device_id"]]
        upstream = next(iter(connection.live_sessions))
        http.portal.call(batch, runtime, upstream)
        assert ws.receive_json()["event"] == "clarify.required"
        assert ws.receive_json()["event"] == "state.snapshot"
        replacement_questions = [{"qid": "new", "question": "new generation with same request ID"}]

        async def rpc(method, params):
            assert method == "clarify.respond"
            connection.calls.append((method, params))
            await batch(runtime, upstream, questions=replacement_questions)
            return {"status": "ok", "remaining": ["q2"]}

        monkeypatch.setattr(connection, "rpc", rpc)
        command = {
            "v": 1,
            "id": str(uuid4()),
            "method": "clarify.respond",
            "params": {
                "session_id": session,
                "request_id": "batch",
                "question_id": "q1",
                "answer": "yes",
            },
        }
        ws.send_json(command)
        frames = [ws.receive_json() for _ in range(4)]
        assert [frame["event"] for frame in frames] == [
            "clarify.required",
            "state.snapshot",
            "interaction.resolved",
            "state.snapshot",
        ]
        receipt = frames[2]
        assert receipt["id"] == command["id"]
        assert receipt["data"]["stale_interaction"] is True
        assert receipt["data"]["turn_id"] == turn and receipt["data"]["applied"] is True
        assert receipt["data"]["request_id"] == "batch" and receipt["data"]["session_id"] == session
        for frame in (frames[1], frames[3]):
            assert frame["data"]["sessions"][0]["pending"][0]["questions"] == replacement_questions
        assert frames[3]["data"]["revision"] > frames[1]["data"]["revision"]
        ws.send_json(command)
        assert ws.receive_json() == receipt  # Ledger preserves stale flag; no new RPC or mutation.
        assert [method for method, _ in connection.calls].count("clarify.respond") == 1
        assert runtime.pending["batch"][2]["questions"] == replacement_questions
