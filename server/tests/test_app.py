import time
from uuid import uuid4

import pytest
from conftest import device, said
from starlette.websockets import WebSocketDisconnect
from support_app import (
    client,
)
from support_audio import wav_bytes

from orbit_hub.protocol import ApprovalParams, HubError, SendParams
from orbit_hub.runtime import DeviceRuntime


def test_http_auth_and_pairing_validation(environment):
    _, store, _ = environment
    with client(environment) as http:
        assert http.get("/v1/sessions").status_code == 401
        code = store.create_code(1000)
        compatible = http.post("/v1/pair", json={"code": code, "name": "Fixture", "admin": True})
        assert compatible.status_code == 200 and "admin" not in compatible.json()
        code = store.create_code(1000)
        invalid = http.post("/v1/pair", json={"code": code, "name": ""})
        assert invalid.status_code == 400 and code not in invalid.text
        paired = http.post("/v1/pair", json={"code": code, "name": "Fixture"})
        assert paired.status_code == 200
        assert http.post("/v1/pair", json={"code": code, "name": "Again"}).status_code == 403
        assert http.get(
            "/v1/sessions", headers={"Authorization": "Bearer " + paired.json()["token"]}
        ).json() == {"sessions": [], "total": 0, "next": None}
        assert http.get("/health", headers={"Origin": "https://evil.invalid"}).status_code == 403


def test_sessions_created_preserves_real_number_and_fraction(environment):
    _, store, _ = environment
    paired = device(store)
    created = 1789668184.393808
    session_id = store.create_session(
        paired["device_id"], "runtime-dae07", "stored-dae07", "epoch-dae07", created
    )

    with client(environment) as http:
        response = http.get("/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]})

    # SIM-30: the hub sends milliseconds, the fraction rounded, never cut.
    assert response.status_code == 200
    assert response.json() == {
        "sessions": [
            {
                "id": session_id,
                "created_at_ms": 1_789_668_184_394,
                "last_at_ms": 1_789_668_184_394,
                "message_count": 0,
            }
        ],
        "total": 1,
        "next": None,
    }


def test_websocket_contract_stream_ids_and_idempotency(environment):
    _, store, adapter = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    operation = str(uuid4())
    frame = {"v": 1, "id": operation, "method": "chat.send", "params": {"text": "hello synthetic"}}
    with (
        client(environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        ws.send_json(frame)
        received = [ws.receive_json() for _ in range(5)]
        assert [item["event"] for item in received] == [
            "chat.session",
            "chat.delta",
            "chat.completed",
            "state.snapshot",
            "chat.session",
        ]
        assert all(
            item["id"] == operation for item in received if item["event"] != "state.snapshot"
        )
        assert received[3]["data"]["sessions"][0]["status"] == "completed"
        assert received[2]["data"]["persisted"] is True
        assert "warning" not in received[2]["data"]
        session = received[0]["data"]["session_id"]
        ws.send_json(frame)
        assert ws.receive_json()["event"] == "chat.session"
        assert len(adapter.connections[0].calls) == 1
        page = http.get(f"/v1/sessions/{session}/messages", headers=headers).json()
        assert len(page["messages"]) == 2
        assert {"role": "assistant", "text": "respuesta sintética"} in [
            {"role": message["role"], "text": message["text"]} for message in page["messages"]
        ]
        ws.send_json({"v": 1, "id": str(uuid4()), "method": "config.set", "params": {}})
        assert ws.receive_json()["data"]["code"] == "invalid_request"
    with (
        client(environment) as http,
        pytest.raises(WebSocketDisconnect),
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer invalid"}
        ),
    ):
        pass


async def test_interaction_cannot_cross_session_or_device(environment):
    _, store, adapter = environment
    owner, other = device(store)["device_id"], device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, adapter, lambda: 1000)
    _, first = await runtime.handle("chat.send", SendParams(text="synthetic"), str(uuid4()))
    _, second = await runtime.handle("chat.send", SendParams(text="synthetic"), str(uuid4()))
    runtime.pending["request"] = (first["session_id"], "approval", {"choices": ["once", "deny"]})
    with pytest.raises(HubError) as error:
        await runtime.handle(
            "approval.respond",
            ApprovalParams(session_id=second["session_id"], request_id="request", approved=True),
            str(uuid4()),
        )
    assert error.value.code == "interaction_not_found"
    with pytest.raises(HubError):
        store.session_messages(other, first["session_id"], None, 1)
    await runtime.handle(
        "approval.respond",
        ApprovalParams(session_id=first["session_id"], request_id="request", approved=True),
        str(uuid4()),
    )
    assert adapter.connections[0].calls[-1][1]["choice"] == "once"


async def test_failed_completion_is_not_an_authoritative_history_message(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, adapter, lambda: 1000)
    upstream = str(uuid4())
    session = store.create_session(owner, upstream, str(uuid4()), "fixture", 1000)
    runtime.bindings[upstream] = session
    runtime.turn_ids[session] = str(uuid4())
    store.append_message(session, "user", "synthetic failed request", now_ms=1_000_000)
    await runtime.on_upstream(
        {
            "type": "message.complete",
            "session_id": upstream,
            "payload": {"text": "NEVER_A_CONFIRMED_ANSWER", "status": "error"},
        }
    )
    assert store.session(owner, session)["status"] == "failed"
    assert said(store, owner, session) == [{"role": "user", "text": "synthetic failed request"}]


def test_revocation_interrupts_upstream_after_client_disconnect(environment, monkeypatch):
    from conftest import FakeConnection

    async def unfinished_rpc(connection, method, params):
        connection.calls.append((method, params))
        return {"status": "streaming" if method == "prompt.submit" else "interrupted"}

    monkeypatch.setattr(FakeConnection, "rpc", unfinished_rpc)
    _, store, adapter = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with client(environment) as http:
        with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
            assert ws.receive_json()["event"] == "ready"
            ws.send_json(
                {"v": 1, "id": str(uuid4()), "method": "chat.send", "params": {"text": "fixture"}}
            )
            assert ws.receive_json()["event"] == "chat.session"
            assert ws.receive_json()["event"] == "chat.session"
        connection = adapter.connections[0]
        assert connection.connected  # Normal viewer closure must not interrupt the turn.
        assert [method for method, _ in connection.calls] == ["prompt.submit"]
        store.revoke(paired["device_id"])
        deadline = time.monotonic() + 5
        while connection.connected and time.monotonic() < deadline:
            time.sleep(0.02)
        assert connection.connected is False
        assert [method for method, _ in connection.calls] == ["prompt.submit", "session.interrupt"]
        assert connection.calls[-1][1]["session_id"] in connection.live_sessions
        with (
            pytest.raises(WebSocketDisconnect),
            http.websocket_connect("wss://localhost/v1/ws", headers=headers),
        ):
            pass


async def test_revoked_runtime_drops_late_events_and_does_not_interrupt_another_device(environment):
    _, store, adapter = environment
    owner, other = device(store)["device_id"], device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, adapter, lambda: 1000)
    neighbour = DeviceRuntime(other, store, adapter, lambda: 1000)
    _, response = await runtime.handle("chat.send", SendParams(text="fixture owner"), str(uuid4()))
    await neighbour.handle("chat.send", SendParams(text="fixture other"), str(uuid4()))
    queue = runtime.subscribe()
    assert queue.get_nowait()["event"] == "state.snapshot"
    store.revoke(owner)
    await runtime.revoke()
    assert queue.get_nowait()["event"] == "revoked"
    await runtime.on_upstream(
        {
            "type": "message.delta",
            "session_id": next(iter(runtime.bindings)),
            "payload": {"text": "REVOKED_LATE_FRAME"},
        }
    )
    assert queue.empty()
    assert all("REVOKED" not in item["text"] for item in said(store, owner, response["session_id"]))
    assert neighbour.connection.connected
    assert [method for method, _ in neighbour.connection.calls] == ["prompt.submit"]
    await neighbour.close()


def test_audio_wav_validation_and_clip_response(environment):
    _, store, adapter = environment
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    body = wav_bytes(b"\0\0" * 160)
    with client(environment) as http:
        assert (
            http.post("/v1/audio/transcribe", headers=headers, content=b"invalid").status_code
            == 400
        )
        response = http.post("/v1/audio/transcribe", headers=headers, content=body)
        assert response.json() == {"text": "Transcripción sintética"}
        assert adapter.audio == [body]
        speech = http.post("/v1/audio/speech", headers=headers, json={"text": "synthetic"})
        assert speech.status_code == 200
        assert speech.headers["X-Orbit-Speech-Kind"] == "clip"
        assert speech.headers["X-Orbit-Speech-Provider"] == "synthetic"
        assert speech.headers["content-type"] == "audio/wav"
        assert speech.content == wav_bytes(b"\0\0" * 32, rate=32000)
