import json
from uuid import uuid4

from conftest import FakeConnection, device, only, said
from support_app import client
from support_audio import wav_bytes

from orbit_hub.protocol import HubError, SendParams
from orbit_hub.runtime import DeviceRuntime


def wav_fixture() -> bytes:
    return wav_bytes(b"\0\0" * 160)


async def test_text_turn_declares_orbit_only_to_hermes(environment):
    _, store, adapter = environment
    owner = device(store, "Sample Mac")["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    published = runtime.subscribe()

    _, receipt = await runtime.handle("chat.send", SendParams(text="Hola, Deimos"), str(uuid4()))

    method, prompt = adapter.connections[0].calls[-1]
    assert method == "prompt.submit"
    assert prompt == {
        "session_id": next(iter(runtime.bindings))[1],
        "text": "[Orbit · Sample Mac · text]\nHola, Deimos",
    }
    assert said(store, owner, receipt["session_id"]) == [
        {"role": "user", "text": "Hola, Deimos"},
        {"role": "assistant", "text": "respuesta sintética"},
    ]
    public_frames = []
    while not published.empty():
        public_frames.append(published.get_nowait())
    assert {frame["event"] for frame in public_frames} >= {"chat.delta", "chat.completed"}
    assert "[Orbit ·" not in json.dumps(public_frames, ensure_ascii=False)


async def test_active_call_reaches_only_the_hermes_header(environment):
    _, store, adapter = environment
    owner = device(store, "Sample Mac")["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    published = runtime.subscribe()

    _, receipt = await runtime.handle(
        "chat.send",
        SendParams(text="Hola durante la llamada", call_active=True),
        str(uuid4()),
    )

    method, prompt = adapter.connections[0].calls[-1]
    assert method == "prompt.submit"
    assert prompt["text"] == ("[Orbit · Sample Mac · ongoing voice call]\nHola durante la llamada")
    assert said(store, owner, receipt["session_id"]) == [
        {"role": "user", "text": "Hola durante la llamada"},
        {"role": "assistant", "text": "respuesta sintética"},
    ]
    public_frames = []
    while not published.empty():
        public_frames.append(published.get_nowait())
    assert "ongoing voice call" not in json.dumps(public_frames, ensure_ascii=False)


def test_transcription_marker_is_device_scoped_and_consumed(environment):
    _, store, adapter = environment
    owner = device(store, "Sample Mac")
    other = device(store, "Otra Mac")
    owner_headers = {"Authorization": "Bearer " + owner["token"]}
    other_headers = {"Authorization": "Bearer " + other["token"]}

    with client(environment) as http:
        response = http.post("/v1/audio/transcribe", headers=owner_headers, content=wav_fixture())
        assert response.json() == {"text": "Transcripción sintética"}

        with http.websocket_connect("wss://localhost/v1/ws", headers=other_headers) as ws:
            assert ws.receive_json()["event"] == "ready"
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "chat.send",
                    "params": {"text": response.json()["text"]},
                }
            )
            [ws.receive_json() for _ in range(5)]
        assert adapter.connections[0].calls[-1][1]["text"].startswith("[Orbit · Otra Mac · text]\n")

        with http.websocket_connect("wss://localhost/v1/ws", headers=owner_headers) as ws:
            assert ws.receive_json()["event"] == "ready"
            first_id = str(uuid4())
            ws.send_json(
                {
                    "v": 1,
                    "id": first_id,
                    "method": "chat.send",
                    "params": {"text": response.json()["text"]},
                }
            )
            first_frames = [ws.receive_json() for _ in range(5)]
            session_id = next(
                frame["data"]["session_id"]
                for frame in first_frames
                if frame["event"] == "chat.session"
            )
            assert (
                adapter.connections[1]
                .calls[-1][1]["text"]
                .startswith("[Orbit · Sample Mac · transcribed voice note]\n")
            )

            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "chat.send",
                    "params": {"session_id": session_id, "text": "Turno siguiente"},
                }
            )
            [ws.receive_json() for _ in range(5)]
            assert (
                adapter.connections[1]
                .calls[-1][1]["text"]
                .startswith("[Orbit · Sample Mac · text]\n")
            )


async def test_prompt_submit_omits_unproven_source_without_retry_or_log(environment, monkeypatch):
    logged = []

    async def reject_source(connection, method, params):
        connection.calls.append((method, params))
        if method == "prompt.submit" and "source" in params:
            raise HubError("upstream_source_unsupported", "Hermes no reconoce el campo source")
        if method == "prompt.submit":
            await connection.callback(
                {
                    "type": "message.complete",
                    "session_id": params["session_id"],
                    "payload": {"text": "respuesta sintética", "status": "success"},
                }
            )
        return {"status": "streaming" if method == "prompt.submit" else "ok"}

    def record_fallback(*_args, **fields):
        logged.append(fields)

    monkeypatch.setattr(FakeConnection, "rpc", reject_source)
    # Every piece of the runtime that reports a failure.
    for piece in ("device", "events", "send"):
        monkeypatch.setattr(f"orbit_hub.runtime.{piece}.report", record_fallback)
    _, store, adapter = environment
    owner = device(store, "Sample Mac")["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)

    _, first = await runtime.handle("chat.send", SendParams(text="Contenido privado"), str(uuid4()))
    await runtime.handle(
        "chat.send",
        SendParams(text="Segundo turno", session_id=first["session_id"]),
        str(uuid4()),
    )

    calls = adapter.connections[0].calls
    assert len(calls) == 2
    assert all(method == "prompt.submit" for method, _ in calls)
    assert all("source" not in params for _, params in calls)
    assert logged == []
    assert "Contenido privado" not in json.dumps(logged)
