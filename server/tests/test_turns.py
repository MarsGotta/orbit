"""An explicit turn state machine; synthetic upstreams, never live Hermes."""

import asyncio
import threading
from uuid import uuid4

import pytest
from conftest import FakeConnection, device, only
from support_app import client
from support_review_regressions import receive_event
from support_runtime_completion_persistence import drained

from orbit_hub.protocol import HubError, InterruptParams, SendParams
from orbit_hub.runtime import DeviceRuntime


async def test_an_expired_interrupt_blocks_resend_without_replacing_the_turn(refused_interrupt):
    _, store, adapter = refused_interrupt
    now = [1001.0]
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: now[0])
    turn = str(uuid4())
    try:
        _, data = await runtime.handle("chat.send", SendParams(text="synthetic"), turn)
        session = data["session_id"]

        # This connection accepts the interruption but sends no terminal event.
        async def accepted(method, params):
            adapter.connections[0].calls.append((method, params))
            return {"status": "interrupted"}

        runtime.connections["default"].rpc = accepted
        await runtime.handle(
            "chat.interrupt", InterruptParams(session_id=session, turn_id=turn), str(uuid4())
        )
        now[0] = 1030.0
        with pytest.raises(HubError) as busy:
            await runtime.handle(
                "chat.send", SendParams(text="before deadline", session_id=session), str(uuid4())
            )
        assert busy.value.code == "session_busy"
        assert store.chat.session(owner, session)["status"] == "interrupting"
        now[0] = 1031.0
        with pytest.raises(HubError) as expired:
            await runtime.handle(
                "chat.send", SendParams(text="at deadline", session_id=session), str(uuid4())
            )
        assert expired.value.code == "turn_outcome_unknown"
        assert store.chat.session(owner, session)["status"] == "outcome_unknown"
        assert runtime.turn_ids[session] == turn
        assert [method for method, _ in runtime.connections["default"].calls] == [
            "prompt.submit",
            "session.interrupt",
        ]
    finally:
        await runtime.close()


class UnconfirmedSubmitConnection(FakeConnection):
    """prompt.submit expires with the same typed error the real transport produces."""

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "prompt.submit":
            raise HubError("outcome_unknown", "Hermes no confirmó la operación; no se repetirá")
        return {"status": "ok"}


@pytest.fixture
def unconfirmed(environment, monkeypatch):
    _, _, adapter = environment

    def connection(device, device_name, callback):
        value = UnconfirmedSubmitConnection(callback, device, device_name)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


async def test_unconfirmed_submit_marks_outcome_unknown_and_blocks_resend(unconfirmed):
    _, store, adapter = unconfirmed
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    queue = runtime.subscribe()
    turn = str(uuid4())
    with pytest.raises(HubError):
        await runtime.handle("chat.send", SendParams(text="synthetic"), turn)
    session = next(frame for frame in drained(queue) if frame["event"] == "chat.session")["data"][
        "session_id"
    ]
    upstream = next(iter(runtime.bindings))[1]
    assert store.chat.session(owner, session)["status"] == "outcome_unknown"
    with pytest.raises(HubError) as error:
        await runtime.handle(
            "chat.send",
            SendParams(text="second explicit attempt", session_id=session),
            str(uuid4()),
        )
    assert error.value.code == "turn_outcome_unknown"
    assert "session.new" in error.value.message
    assert [method for method, _ in runtime.connections["default"].calls] == ["prompt.submit"]
    assert runtime.turn_ids.get(session) == turn  # Never reused or overwritten.
    await runtime.connections["default"].receive(
        {
            "type": "message.delta",
            "session_id": upstream,
            "payload": {"text": "LATE_SYNTHETIC_DELTA"},
        }
    )
    for frame in drained(queue):
        if frame["event"] == "chat.delta":
            assert frame["id"] == turn  # A late delta stays on the old turn only.
    await runtime.close()


class HeldSubmitConnection(FakeConnection):
    """prompt.submit waits for an explicit release; controls must not queue behind it."""

    def __init__(self, callback, *device):
        super().__init__(callback, *device)
        self.submit_arrived = asyncio.Event()
        self.release_submit = asyncio.Event()

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "prompt.submit":
            self.submit_arrived.set()
            await self.release_submit.wait()
            return {"status": "streaming"}
        return {"status": "interrupted"}


@pytest.fixture
def held(environment, monkeypatch):
    _, _, adapter = environment
    connection_created = asyncio.Event()

    def connection(device, device_name, callback):
        value = HeldSubmitConnection(callback, device, device_name)
        adapter.connections.append(value)
        connection_created.set()
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    adapter.connection_created = connection_created
    return environment


async def test_submitting_interrupt_never_returns_to_streaming(held):
    _, store, adapter = held
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    turn = str(uuid4())
    send = asyncio.create_task(runtime.handle("chat.send", SendParams(text="synthetic"), turn))
    try:
        # Same 2 s deadline the neighboring waits in this file use: the
        # fixture sets the event in-process, so this only covers scheduling.
        try:
            await asyncio.wait_for(adapter.connection_created.wait(), 2)
        except TimeoutError:
            pytest.fail("chat.send did not open the upstream connection within 2 s")
        connection = adapter.connections[0]
        await asyncio.wait_for(connection.submit_arrived.wait(), 2)
        session = next(iter(runtime.bindings.values()))

        await runtime.handle(
            "chat.interrupt", InterruptParams(session_id=session, turn_id=turn), str(uuid4())
        )
        assert store.chat.session(owner, session)["status"] == "interrupting"

        connection.release_submit.set()
        name, data = await asyncio.wait_for(send, 2)
        assert name == "chat.session"
        assert store.chat.session(owner, session)["status"] == "interrupting"
        # The submit RPC reply must not claim the turn streamed when the
        # persisted state says the interrupt already owns it.
        assert data["status"] == "interrupting"
    finally:
        if adapter.connections:
            adapter.connections[0].release_submit.set()
        await asyncio.gather(send, return_exceptions=True)
        await runtime.close()


class InterruptRefusedConnection(FakeConnection):
    """session.interrupt is refused upstream while the turn stays live."""

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "session.interrupt":
            raise HubError("upstream_rejected", "Hermes no aceptó interrumpir el turno")
        # No callbacks: the turn stays `streaming`, alive and unanswered.
        return {"status": "streaming" if method == "prompt.submit" else "ok"}


@pytest.fixture
def refused_interrupt(environment, monkeypatch):
    _, _, adapter = environment

    def connection(device, device_name, callback):
        value = InterruptRefusedConnection(callback, device, device_name)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


def test_rejected_interrupt_publishes_a_snapshot_with_the_live_turn(refused_interrupt):
    """A refused interrupt changes no state, so `changed()` never fires; without
    a republished snapshot the daemon keeps discarding the turn it hid."""
    _, store, adapter = refused_interrupt
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(refused_interrupt) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        turn = str(uuid4())
        ws.send_json({"v": 1, "id": turn, "method": "chat.send", "params": {"text": "synthetic"}})
        session = receive_event(ws)["data"]["session_id"]  # Emitted before the submit RPC.
        assert receive_event(ws)["event"] == "chat.session"  # The reply: the turn streams.
        command = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": command,
                "method": "chat.interrupt",
                "params": {"session_id": session, "turn_id": turn},
            }
        )
        frames = []

        def read_two():
            for _ in range(2):
                frames.append(ws.receive_json())

        reader = threading.Thread(target=read_two, daemon=True)
        reader.start()
        reader.join(2)
        assert not reader.is_alive(), (
            "chat.interrupt was answered but no state.snapshot followed the refusal "
            f"(frames received: {[frame['event'] for frame in frames]})"
        )
        assert frames[0]["event"] == "error"
        assert frames[0]["id"] == command
        assert frames[0]["data"]["code"] == "upstream_rejected"
        assert frames[1]["event"] == "state.snapshot"
        entry = next(
            item for item in frames[1]["data"]["sessions"] if item["session_id"] == session
        )
        assert entry["turn_id"] == turn
        assert entry["status"] == "streaming", "the refused turn must still be live"
    assert [method for method, _ in adapter.connections[0].calls] == [
        "prompt.submit",
        "session.interrupt",
    ]


# How long the interrupt's answer may take while prompt.submit is held. The
# submit is released only after the answer (or this bound), so waiting longer
# cannot let an interrupt queued behind it pass: the bound only ends a hang.
# Measured 2026-10-21, 300 runs each, from sending the interrupt to its
# answer: idle machine median 2.8 ms, max 92 ms; 64 busy CPUs max 152 ms;
# 64 busy CPUs and four fsync writers p99 3.06 s, max 3.96 s (8 of 300 over
# the former 2 s). Under that load the whole test, setup included, took up to
# 9.5 s. 60 s is six times the slowest whole test and costs a passing run nothing.
INTERRUPT_ANSWER_BOUND_SECONDS = 60


def test_websocket_interrupt_is_answered_while_submit_waits_for_hermes(held):
    _, store, adapter = held
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(held) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        turn = str(uuid4())
        ws.send_json({"v": 1, "id": turn, "method": "chat.send", "params": {"text": "synthetic"}})
        session = receive_event(ws)["data"]["session_id"]  # Emitted before the submit RPC.
        connection = adapter.connections[0]
        command = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": command,
                "method": "chat.interrupt",
                "params": {"session_id": session, "turn_id": turn},
            }
        )
        answered = []
        reader = threading.Thread(target=lambda: answered.append(receive_event(ws)), daemon=True)
        reader.start()
        reader.join(INTERRUPT_ANSWER_BOUND_SECONDS)
        try:
            assert not reader.is_alive(), (
                f"chat.interrupt did not answer within {INTERRUPT_ANSWER_BOUND_SECONDS} s "
                "while chat.send waited"
            )
            assert answered[0]["event"] == "chat.interrupted"
            assert answered[0]["id"] == command
        finally:
            http.portal.call(connection.release_submit.set)
    assert [method for method, _ in connection.calls] == ["prompt.submit", "session.interrupt"]
