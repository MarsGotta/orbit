"""A control whose RPC fails never leaves an open safety receipt.

Synthetic upstream counts interrupt RPCs and can fail them with the same typed
errors the real transport produces (upstream_rejected, outcome_unknown).
"""

import json
from uuid import uuid4

import pytest
from conftest import FakeConnection, device
from support_app import client
from support_review_regressions import receive_event, submit

from orbit_hub import safety
from orbit_hub.protocol import HubError


class CountingConnection(FakeConnection):
    def __init__(self, callback):
        super().__init__(callback)
        self.interrupt_errors = []

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "session.interrupt" and self.interrupt_errors:
            raise self.interrupt_errors.pop(0)
        return {"status": "streaming"} if method == "prompt.submit" else {"status": "ok"}


@pytest.fixture
def counted(environment, monkeypatch):
    _, _, adapter = environment

    def connection(callback):
        value = CountingConnection(callback)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


def interrupt(ws, session, turn, operation=None):
    command = str(operation or uuid4())
    ws.send_json(
        {
            "v": 1,
            "id": command,
            "method": "chat.interrupt",
            "params": {"session_id": session, "turn_id": turn},
        }
    )
    return receive_event(ws)


def interrupt_calls(connection):
    return [method for method, _ in connection.calls].count("session.interrupt")


def stored_receipt(store, device_id, target):
    row = store._db.execute(
        "SELECT response FROM safety_operations WHERE device_id=? AND target=?",
        (device_id, target),
    ).fetchone()
    assert row is not None and row["response"] is not None, "safety receipt left NULL"
    return json.loads(store.decrypt(row["response"], f"safety:{device_id}:{target}"))


def test_rejected_interrupt_closes_error_receipt_and_new_uuid_reaches_upstream(counted):
    _, store, adapter = counted
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(counted) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        connection.interrupt_errors = [
            HubError("upstream_rejected", "Hermes rechazó la operación (código synthetic)")
        ]
        first = interrupt(ws, session, turn)
        assert first["data"]["code"] == "upstream_rejected"
        target = safety.target_key("chat.interrupt", session, turn)
        receipt = stored_receipt(store, paired["device_id"], target)
        assert receipt["data"]["status"] == "error"
        assert receipt["data"]["code"] == "upstream_rejected"
        assert receipt["data"]["refusal"]["code"] == "upstream_rejected"
        second = interrupt(ws, session, turn, uuid4())
        assert second["event"] == "chat.interrupted"
        assert interrupt_calls(connection) == 2


def test_same_uuid_retry_after_error_replays_recorded_response_without_new_rpc(counted):
    _, store, adapter = counted
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(counted) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        connection.interrupt_errors = [
            HubError("upstream_rejected", "Hermes rechazó la operación (código synthetic)")
        ]
        operation = str(uuid4())
        first = interrupt(ws, session, turn, operation)
        assert first["data"]["code"] == "upstream_rejected"
        replay = interrupt(ws, session, turn, operation)
        assert replay == first
        assert interrupt_calls(connection) == 1


def test_unconfirmed_interrupt_records_outcome_unknown_and_new_uuid_is_admitted(counted):
    _, store, adapter = counted
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(counted) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        connection.interrupt_errors = [
            HubError("outcome_unknown", "Hermes no confirmó la operación; no se repetirá")
        ]
        first = interrupt(ws, session, turn)
        assert first["data"]["code"] == "outcome_unknown"
        target = safety.target_key("chat.interrupt", session, turn)
        receipt = stored_receipt(store, paired["device_id"], target)
        assert receipt["data"]["status"] == "error"
        assert receipt["data"]["code"] == "outcome_unknown"
        assert receipt["data"]["refusal"]["retry"] == {"kind": "check_outcome"}
        second = interrupt(ws, session, turn, uuid4())
        assert second["event"] == "chat.interrupted"
        assert interrupt_calls(connection) == 2


def test_successful_interrupt_deduplicates_by_target_without_new_rpc(counted):
    _, store, adapter = counted
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with (
        client(counted) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert receive_event(ws)["event"] == "ready"
        session, turn = submit(ws)
        connection = adapter.connections[0]
        first = interrupt(ws, session, turn)
        assert first["event"] == "chat.interrupted" and first["data"]["status"] == "requested"
        second = interrupt(ws, session, turn, uuid4())
        assert second["event"] == "chat.interrupted"
        assert second["id"] != first["id"] and second["data"] == first["data"]
        assert interrupt_calls(connection) == 1
