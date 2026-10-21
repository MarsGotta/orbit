"""Shared fixtures and helpers for test_review_regressions."""

from uuid import uuid4

import pytest
from conftest import FakeConnection, device

from orbit_hub.protocol import SendParams
from orbit_hub.runtime import DeviceRuntime

__all__ = (
    "ControlledConnection",
    "pending",
    "receive_event",
    "reviewed",
    "runtime_fixture",
    "submit",
)


class ControlledConnection(FakeConnection):
    def __init__(self, callback, *device):
        super().__init__(callback, *device)
        self.approval_result = {"resolved": 1}
        self.clarify_result = {"status": "ok"}
        self.running = True

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "approval.respond":
            return self.approval_result
        if method == "clarify.respond":
            return self.clarify_result
        if method == "session.status":
            return {
                "output": "Hermes TUI Status\nAgent Running: " + ("Yes" if self.running else "No")
            }
        return {"status": "streaming" if method == "prompt.submit" else "interrupted"}


@pytest.fixture
def reviewed(environment, monkeypatch):
    _, _, adapter = environment

    def connection(device, device_name, callback):
        value = ControlledConnection(callback, device, device_name)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


def receive_event(ws):
    while True:
        frame = ws.receive_json()
        if frame["event"] != "state.snapshot":
            return frame


def submit(ws):
    turn = str(uuid4())
    ws.send_json(
        {"v": 1, "id": turn, "method": "chat.send", "params": {"text": "synthetic pending"}}
    )
    session = receive_event(ws)["data"]["session_id"]
    assert receive_event(ws)["event"] == "chat.session"
    return session, turn


def pending(http, connection, request="fixture", kind="approval"):
    http.portal.call(
        connection.callback,
        {
            "type": kind + ".request",
            "session_id": next(iter(connection.live_sessions)),
            "payload": {
                "request_id": request,
                "command": "synthetic command",
                "question": "synthetic question",
                "choices": ["once", "deny"],
            },
        },
    )


async def runtime_fixture(reviewed):
    _, store, adapter = reviewed
    owner = device(store)["device_id"]
    runtime, turn = DeviceRuntime(owner, store, adapter, lambda: 1001), str(uuid4())
    _, created = await runtime.handle("chat.send", SendParams(text="synthetic"), turn)
    return runtime, created["session_id"], turn, next(iter(runtime.bindings))
