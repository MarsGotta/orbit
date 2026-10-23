"""The socket keeps reading a Mac's frames while a control waits.

An approval, a clarification or an interrupt can wait on the agent (its answer
RPC) or on the admission of a turn of its own conversation (a reopen). Neither
wait holds the socket's read loop: a `ping` sent after the control is answered
before the control is. The agent is a synthetic Hermes; names are invented.
"""

import asyncio
import threading
from uuid import uuid4

from conftest import device
from support_app import client
from support_review_regressions import ControlledConnection, pending, submit
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414

from orbit_hub.assistant.backend import ResumedSession

# A pong answers in milliseconds; this is far over it and far under the time
# the test holds the control.
PONG_BUDGET_SECONDS = 5
# How long the test holds the agent at most, so a red run ends.
HOLD_SECONDS = 15


def read_until(ws, frame_id, frames, done):
    """Read frames (keeping them) until the one with `frame_id` arrives."""
    while True:
        frame = ws.receive_json()
        frames.append(frame)
        if frame.get("id") == frame_id:
            done.set()
            return


def ping_while_held(ws, frames):
    """Send a ping and wait, off the test's thread, for its pong."""
    ping = str(uuid4())
    answered = threading.Event()
    ws.send_json({"v": 1, "id": ping, "method": "ping", "params": {}})
    reader = threading.Thread(target=read_until, args=(ws, ping, frames, answered))
    reader.start()
    return ping, answered, reader


def test_the_socket_keeps_reading_while_an_approval_waits_for_the_agent(reviewed, monkeypatch):
    _, store, adapter = reviewed
    paired = device(store)
    release = threading.Event()
    with (
        client(reviewed) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        pending(http, connection)
        original = connection.rpc

        async def rpc(method, params):
            if method == "approval.respond":
                connection.calls.append((method, params))
                await asyncio.to_thread(release.wait, HOLD_SECONDS)
                return {"resolved": 1}
            return await original(method, params)

        monkeypatch.setattr(connection, "rpc", rpc)
        approval = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": approval,
                "method": "approval.respond",
                "params": {
                    "session_id": session,
                    "request_id": "fixture",
                    "approved": True,
                    "turn_id": turn,
                },
            }
        )
        frames: list[dict] = []
        _, answered, reader = ping_while_held(ws, frames)
        try:
            assert answered.wait(PONG_BUDGET_SECONDS), "the read loop waited for the approval"
            assert not release.is_set()
            assert approval not in {frame.get("id") for frame in frames}
        finally:
            release.set()
            reader.join(HOLD_SECONDS)
        resolved = []
        read_until(ws, approval, resolved, threading.Event())
        assert resolved[-1]["event"] == "interaction.resolved"
        assert resolved[-1]["data"]["applied"] is True


class SlowOpenConnection(ControlledConnection):
    """A connection that opens only when the test lets it."""

    gate: threading.Event

    async def open(self):
        await asyncio.to_thread(self.gate.wait, HOLD_SECONDS)
        self.connected = True

    async def resume_session(self, stored_id):
        self.live_sessions.add("reopened-1")
        return ResumedSession("reopened-1", stored_id, None, False)


def test_the_socket_keeps_reading_while_a_control_waits_for_its_conversation_to_reopen(
    reviewed, monkeypatch
):
    _, store, adapter = reviewed
    paired = device(store)
    gate = threading.Event()

    def connection(device, device_name, callback):
        made = ControlledConnection if not adapter.connections else SlowOpenConnection
        value = made(callback, device, device_name)
        value.gate = gate
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    with (
        client(reviewed) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        session, _ = submit(ws)
        # The turn ends and Hermes goes away: the next send reopens the
        # conversation on a new connection, whose start the test holds.
        store.chat.set_status(session, "completed")
        adapter.connections[0].connected = False
        reopening = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": reopening,
                "method": "chat.send",
                "params": {"text": "Sigo", "session_id": session},
            }
        )
        clarification = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": clarification,
                "method": "clarify.respond",
                "params": {"session_id": session, "request_id": "pregunta-1", "answer": "Sí"},
            }
        )
        frames: list[dict] = []
        _, answered, reader = ping_while_held(ws, frames)
        try:
            assert answered.wait(PONG_BUDGET_SECONDS), "the read loop waited for the reopen"
            assert not gate.is_set()
            assert clarification not in {frame.get("id") for frame in frames}
        finally:
            gate.set()
            reader.join(HOLD_SECONDS)
        # The control still applies after the turn sent before it: that turn's
        # question was never asked, so it is answered as unknown.
        answers = []
        read_until(ws, clarification, answers, threading.Event())
        assert answers[-1]["event"] == "error"
        assert answers[-1]["data"]["code"] == "interaction_not_found"
        assert reopening in {frame.get("id") for frame in answers}
