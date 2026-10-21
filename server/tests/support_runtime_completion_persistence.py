"""Shared fixtures and helpers for test_runtime_completion_persistence."""

from uuid import uuid4

import pytest
from conftest import FakeConnection, device

from orbit_hub.protocol import SendParams
from orbit_hub.runtime import DeviceRuntime

__all__ = (
    "DeferredConnection",
    "drained",
    "isolated",
    "streaming_turn",
)


class DeferredConnection(FakeConnection):
    """Synthetic upstream that acknowledges a prompt without completing it inline."""

    async def rpc(self, method, params):
        self.calls.append((method, params))
        return {"status": "streaming" if method == "prompt.submit" else "ok"}


@pytest.fixture
def isolated(environment, monkeypatch):
    _, _, adapter = environment

    def connection(device, device_name, callback):
        value = DeferredConnection(callback, device, device_name)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


async def streaming_turn(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, adapter, lambda: 1001)
    turn = str(uuid4())
    _, created = await runtime.handle("chat.send", SendParams(text="synthetic"), turn)
    return runtime, created["session_id"], turn, next(iter(runtime.bindings))


def drained(queue):
    frames = []
    while not queue.empty():
        frames.append(queue.get_nowait())
    return frames
