"""The hub drives any `AssistantBackend`: a whole turn with a permission, without Hermes.

The assistant here is a double that speaks only the typed interface
(`orbit_hub.assistant.backend`); no Hermes message, method or transport is
involved. What runs for real is everything on the hub's side of the seam:
the app, the device WebSocket, the runtime, its turn machine and the store.
"""

import asyncio
from uuid import uuid4

from conftest import device, said
from fastapi.testclient import TestClient

from orbit_hub.app import create_app
from orbit_hub.assistant.backend import (
    Capabilities,
    MessageChunk,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    TextBlock,
)


class AgentDouble:
    """One device's connection to an invented agent that asks before acting."""

    epoch = "double-epoch"

    def __init__(self, device_id, device_name, on_event):
        self.device_id, self.device_name, self.on_event = device_id, device_name, on_event
        self.live_sessions: set[str] = set()
        self.connected = False
        self.prompts: list[tuple[str, tuple]] = []
        self.answers: list[tuple[str, str, str]] = []
        self.finishing: asyncio.Task | None = None

    async def open(self):
        self.connected = True

    async def close(self):
        self.connected = False

    async def create_session(self):
        self.live_sessions.add("live-1")
        return OpenedSession(session_id="live-1", stored_id="stored-1")

    async def prompt(self, session_id, blocks):
        self.prompts.append((session_id, tuple(blocks)))
        await self.on_event(MessageChunk(session_id, "Voy a crear la nota. "))
        await self.on_event(
            PermissionRequested(
                session_id,
                PermissionRequest(
                    request_id="permission-1",
                    description="Crear la nota «Plan de la excursión»",
                    choices=("once", "deny"),
                ),
            )
        )

    async def respond_permission(self, session_id, request_id, *, choice):
        self.answers.append((session_id, request_id, choice))
        # The answer is acknowledged first; the agent carries on afterwards.
        self.finishing = asyncio.create_task(self.finish(session_id))
        return PermissionReceipt(applied=True)

    async def finish(self, session_id):
        await self.on_event(MessageChunk(session_id, "Hecha."))
        await self.on_event(
            MessageComplete(session_id, "Voy a crear la nota. Hecha.", status="completed")
        )

    async def resume_session(self, stored_id):
        raise AssertionError("this turn never resumes")

    async def cancel(self, session_id):
        raise AssertionError("this turn is never cancelled")

    async def respond_clarification(self, session_id, request_id, answer, question_id):
        raise AssertionError("this turn asks no clarification")

    async def turn_stopped(self, session_id):
        raise AssertionError("this turn reports no error")


class DoubleAssistant:
    capabilities = Capabilities(images=False, resume=False, history=False, voice=False)

    def __init__(self):
        self.connections: list[AgentDouble] = []

    def connection(self, device, device_name, on_event):
        connection = AgentDouble(device, device_name, on_event)
        self.connections.append(connection)
        return connection


def next_event(ws):
    while True:
        frame = ws.receive_json()
        if frame["event"] != "state.snapshot":
            return frame


def test_a_turn_with_a_permission_runs_through_any_backend(environment):
    settings, store, speech = environment
    assistant = DoubleAssistant()
    paired = device(store, name="Portátil de Prueba")
    headers = {"Authorization": "Bearer " + paired["token"]}
    app = create_app(settings, store=store, assistant=assistant, speech=speech, now=lambda: 1001)
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert next_event(ws)["event"] == "ready"
        turn = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": turn,
                "method": "chat.send",
                "params": {"text": "Crea la nota de la excursión"},
            }
        )
        opened = next_event(ws)
        assert opened["event"] == "chat.session" and opened["id"] == turn
        session = opened["data"]["session_id"]
        assert next_event(ws) == {
            "v": 1,
            "id": turn,
            "event": "chat.delta",
            "data": {"session_id": session, "text": "Voy a crear la nota. "},
        }
        assert next_event(ws) == {
            "v": 1,
            "id": turn,
            "event": "approval.required",
            "data": {
                "session_id": session,
                "request_id": "permission-1",
                "kind": "approval",
                "state": "pending",
                "expires_at": None,
                "description": "Crear la nota «Plan de la excursión»",
                "choices": ["once", "deny"],
            },
        }
        accepted = next_event(ws)
        assert accepted["event"] == "chat.session" and accepted["id"] == turn
        assert accepted["data"] == {"session_id": session, "status": "streaming"}

        answer = str(uuid4())
        ws.send_json(
            {
                "v": 1,
                "id": answer,
                "method": "approval.respond",
                "params": {
                    "session_id": session,
                    "turn_id": turn,
                    "request_id": "permission-1",
                    "approved": True,
                },
            }
        )
        resolved = next_event(ws)
        assert resolved == {
            "v": 1,
            "id": answer,
            "event": "interaction.resolved",
            "data": {
                "session_id": session,
                "request_id": "permission-1",
                "status": "ok",
                "resolved": 1,
                "applied": True,
            },
        }
        assert next_event(ws)["data"] == {"session_id": session, "text": "Hecha."}
        completed = next_event(ws)
        assert completed["event"] == "chat.completed" and completed["id"] == turn
        assert completed["data"] == {
            "session_id": session,
            "text": "Voy a crear la nota. Hecha.",
            "status": "completed",
            "persisted": True,
        }

    [connection] = assistant.connections
    assert (connection.device_id, connection.device_name) == (
        paired["device_id"],
        "Portátil de Prueba",
    )
    assert connection.prompts == [
        (
            "live-1",
            (TextBlock("[Orbit · Portátil de Prueba · text]\nCrea la nota de la excursión"),),
        )
    ]
    assert connection.answers == [("live-1", "permission-1", "once")]
    assert store.chat.session(paired["device_id"], session)["status"] == "completed"
    assert said(store, paired["device_id"], session) == [
        {"role": "user", "text": "Crea la nota de la excursión"},
        {"role": "assistant", "text": "Voy a crear la nota. Hecha."},
    ]
