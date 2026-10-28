"""Contract checks over a real loopback WebSocket with a labeled synthetic Hermes peer."""

import asyncio
import json
import os
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device, only
from websockets.asyncio.server import serve

from orbit_hub.assistant.backend import MessageChunk, ResumedSession, TextBlock
from orbit_hub.assistant.hermes import HermesBackend, HermesSpeech
from orbit_hub.protocol import HubError, SendParams
from orbit_hub.runtime import DeviceRuntime


def credential(tmp_path):
    path = tmp_path / "synthetic-credential"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(fd, b"synthetic-token-only")
    finally:
        os.close(fd)
    return path


async def test_gateway_handshake_rpc_stream_and_resume(environment, tmp_path):
    settings, _, _ = environment
    received, events = [], []

    async def peer(ws):
        assert ws.request.path == "/api/ws?token=synthetic-token-only"
        await ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": "fixture-epoch"},
                    },
                }
            )
        )
        async for raw in ws:
            request = json.loads(raw)
            received.append(request)
            if request["method"] == "session.resume" and request["params"]["session_id"] == "lost":
                await ws.close()
                break
            if (
                request["method"] == "prompt.submit"
                and request["params"]["session_id"] == "missing-runtime"
            ):
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": {"code": 4001, "message": "session not found"},
                        }
                    )
                )
                continue
            failures = {
                "missing": (4007, "session not found"),
                "race": (4007, "session no longer live; retry resume"),
                "too-long": (
                    4130,
                    "This session is too long to reload safely (20001 messages; limit 20000).",
                ),
                "busy": (4009, "session disconnect interrupt settling"),
                "storage": (5000, "Session storage is unavailable: synthetic failure"),
                "unreadable": (
                    5000,
                    "Could not reopen that session (its transcript could not be read). Details: synthetic failure",
                ),
                "unsupported": (-32601, "Method not found"),
            }
            refused = failures.get(request["params"].get("session_id"))
            if request["method"] == "session.resume" and refused is not None:
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": {"code": refused[0], "message": refused[1]},
                        }
                    )
                )
                continue
            result = (
                {"session_id": "runtime-fixture", "stored_session_id": "stored-fixture"}
                if request["method"] == "session.create"
                else {"session_id": "resumed-fixture", "resumed": "stored-fixture"}
                if request["method"] == "session.resume"
                else {"status": "streaming"}
            )
            await ws.send(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            if request["method"] == "prompt.submit":
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {
                                "type": "message.delta",
                                "session_id": "runtime-fixture",
                                "seq": 1,
                                "payload": {"text": "synthetic"},
                            },
                        }
                    )
                )

    async def callback(event):
        events.append(event)

    async with serve(peer, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        configured = replace(
            settings,
            hermes_url=f"http://127.0.0.1:{port}",
            allow_insecure_localhost=True,
            hermes_credential_file=credential(tmp_path),
        )
        connection = HermesBackend(configured, configured.profile).connection(
            "fixture-device", "fixture", callback
        )
        await connection.open()
        assert connection.epoch == "fixture-epoch"
        assert (await connection.create_session()).stored_id == "stored-fixture"
        await connection.prompt("runtime-fixture", (TextBlock("fixture"),))
        for _ in range(20):
            if events:
                break
            await asyncio.sleep(0.01)
        assert events[0] == MessageChunk("runtime-fixture", "synthetic")
        assert received[0]["params"]["profile"] == "deimos"
        assert await connection.resume_session("stored-fixture") == ResumedSession(
            "resumed-fixture", "stored-fixture", None, None
        )
        assert received[-1]["params"] == {
            "profile": "deimos",
            "session_id": "stored-fixture",
            "source": "orbit",
            "close_on_disconnect": False,
            "defer_history": True,
            "omit_messages": False,
        }
        assert "resumed-fixture" in connection.live_sessions
        connection.live_sessions.add("missing-runtime")
        with pytest.raises(HubError) as error:
            await connection.prompt("missing-runtime", (TextBlock("synthetic"),))
        assert error.value.code == "session_runtime_not_found"
        assert "missing-runtime" not in connection.live_sessions
        with pytest.raises(HubError) as error:
            await connection.resume_session("missing")
        assert error.value.code == "session_resume_not_found"
        assert "4007" in error.value.message
        with pytest.raises(HubError) as error:
            await connection.resume_session("race")
        assert error.value.code == "session_resume_race"
        assert "try again" in error.value.message
        with pytest.raises(HubError) as error:
            await connection.resume_session("too-long")
        assert error.value.code == "session_resume_too_large"
        assert "20001" in error.value.message and "20000" in error.value.message
        with pytest.raises(HubError) as error:
            await connection.resume_session("busy")
        assert error.value.code == "session_resume_busy"
        with pytest.raises(HubError) as error:
            await connection.resume_session("storage")
        assert error.value.code == "session_resume_storage_unavailable"
        with pytest.raises(HubError) as error:
            await connection.resume_session("unreadable")
        assert error.value.code == "session_resume_history_unreadable"
        assert "history" in error.value.message
        with pytest.raises(HubError) as error:
            await connection.resume_session("unsupported")
        assert error.value.code == "session_resume_unsupported"
        assert "-32601" in error.value.message
        with pytest.raises(HubError) as error:
            await connection.resume_session("lost")
        assert error.value.code == "session_resume_connection_lost"
        await connection.close()


async def test_large_resume_keeps_other_hermes_session_connected(environment, tmp_path):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    large_response = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": "replaced-by-peer",
            "result": {
                "session_id": "resumed-quiet",
                "resumed": "stored-quiet",
                "messages": [{"role": "assistant", "content": "x" * (4 * 1024 * 1024)}],
            },
        }
    )
    assert len(large_response.encode()) > 4 * 1024 * 1024
    requests = []

    async def peer(ws):
        await ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": "fixture-epoch"},
                    },
                }
            )
        )
        async for raw in ws:
            request = json.loads(raw)
            requests.append(request)
            method = request["method"]
            params = request["params"]
            if method == "session.create":
                number = sum(item["method"] == "session.create" for item in requests)
                result = {
                    "session_id": "runtime-quiet" if number == 1 else "runtime-active",
                    "stored_session_id": "stored-quiet" if number == 1 else "stored-active",
                }
            elif method == "session.resume":
                if not params.get("defer_history"):
                    oversized = json.loads(large_response)
                    oversized["id"] = request["id"]
                    await ws.send(json.dumps(oversized))
                    continue
                result = {
                    "session_id": "resumed-quiet",
                    "resumed": "stored-quiet",
                    "messages": [],
                    "message_count": 2,
                    "hydrating": True,
                }
            else:
                result = {"status": "streaming"}
            if method == "prompt.submit" and "turno activo" not in params["text"]:
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {
                                "type": "message.complete",
                                "session_id": params["session_id"],
                                "payload": {"text": "respuesta sintética", "status": "success"},
                            },
                        }
                    )
                )
            await ws.send(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            if method == "session.resume":
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {
                                "type": "session.resume_progress",
                                "session_id": result["session_id"],
                                "payload": {
                                    "phase": "history",
                                    "status": "complete",
                                    "message_count": 2,
                                },
                            },
                        }
                    )
                )

    async with serve(peer, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        configured = replace(
            settings,
            hermes_url=f"http://127.0.0.1:{port}",
            allow_insecure_localhost=True,
            hermes_credential_file=credential(tmp_path),
        )
        runtime = DeviceRuntime(
            owner, store, only(HermesBackend(configured, configured.profile)), lambda: 1001
        )
        try:
            _, first = await runtime.handle(
                "chat.send", SendParams(text="sesión quieta"), str(uuid4())
            )
            quiet = first["session_id"]
            _, second = await runtime.handle(
                "chat.send", SendParams(text="turno activo"), str(uuid4())
            )
            active = second["session_id"]
            assert store.chat.session(owner, quiet)["status"] == "completed"
            assert store.chat.session(owner, active)["status"] == "streaming"
            runtime.connections["default"].live_sessions.discard(
                store.chat.session(owner, quiet)["runtime_id"]
            )

            _, resumed = await runtime.handle(
                "chat.send", SendParams(text="después", session_id=quiet), str(uuid4())
            )
            assert resumed["session_id"] == quiet
            assert store.chat.session(owner, active)["status"] == "streaming"
            assert runtime.connections["default"].connected
            resume = next(item for item in requests if item["method"] == "session.resume")
            assert resume["params"]["defer_history"] is True
            assert resume["params"]["omit_messages"] is False
        finally:
            await runtime.close()


async def test_session_create_declares_orbit_and_the_device(environment, tmp_path):
    settings, _, _ = environment
    received = []

    async def peer(ws):
        await ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": "fixture-epoch"},
                    },
                }
            )
        )
        request = json.loads(await ws.recv())
        received.append(request)
        await ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": {
                        "session_id": "runtime-fixture",
                        "stored_session_id": "stored-fixture",
                    },
                }
            )
        )

    async def callback(_event):
        pass

    async with serve(peer, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        configured = replace(
            settings,
            hermes_url=f"http://127.0.0.1:{port}",
            allow_insecure_localhost=True,
            hermes_credential_file=credential(tmp_path),
        )
        connection = HermesBackend(configured, configured.profile).connection(
            "fixture-device", "Sample Mac", callback
        )
        await connection.open()
        await connection.create_session()
        await connection.close()

    assert received[0]["method"] == "session.create"
    assert received[0]["params"]["source"] == "orbit"
    assert "Sample Mac" in received[0]["params"]["title"]


async def test_transcription_contract_is_json_not_multipart(environment, tmp_path, monkeypatch):
    settings, _, _ = environment
    configured = replace(settings, hermes_credential_file=credential(tmp_path))
    requests = []

    def reply(request):
        requests.append(request)
        return httpx.Response(200, json={"ok": True, "transcript": "", "provider": "synthetic"})

    original = httpx.AsyncClient
    monkeypatch.setattr(
        "orbit_hub.assistant.hermes_speech.httpx.AsyncClient",
        lambda **kwargs: original(transport=httpx.MockTransport(reply), **kwargs),
    )
    assert await HermesSpeech(configured, configured.profile).transcribe(b"fixture") == ""
    assert requests[0].url.path == "/api/audio/transcribe"
    assert requests[0].url.params["profile"] == "deimos"
    assert requests[0].headers["X-Hermes-Session-Token"] == "synthetic-token-only"
    assert json.loads(requests[0].content) == {
        "data_url": "data:audio/wav;base64,Zml4dHVyZQ==",
        "mime_type": "audio/wav",
    }


async def test_http_failure_names_provider_status(environment, tmp_path, monkeypatch):
    settings, _, _ = environment
    configured = replace(settings, hermes_credential_file=credential(tmp_path))
    original = httpx.AsyncClient
    monkeypatch.setattr(
        "orbit_hub.assistant.hermes_speech.httpx.AsyncClient",
        lambda **kwargs: original(
            transport=httpx.MockTransport(
                lambda _request: httpx.Response(503, text="private body")
            ),
            **kwargs,
        ),
    )

    with pytest.raises(HubError) as failure:
        await HermesSpeech(configured, configured.profile).transcribe(b"synthetic wav")
    assert failure.value.code == "transcription_http_error"
    assert failure.value.numbers == {"provider_status": 503}
    assert "private body" not in str(failure.value)


async def test_connect_timeout_uses_connection_budget(environment, tmp_path, monkeypatch):
    settings, _, _ = environment
    configured = replace(settings, hermes_credential_file=credential(tmp_path))

    def expire(_request):
        raise httpx.ConnectTimeout("synthetic connection timeout")

    original = httpx.AsyncClient
    monkeypatch.setattr(
        "orbit_hub.assistant.hermes_speech.httpx.AsyncClient",
        lambda **kwargs: original(transport=httpx.MockTransport(expire), **kwargs),
    )
    with pytest.raises(HubError) as failure:
        await HermesSpeech(configured, configured.profile).transcribe(b"synthetic wav")
    assert failure.value.code == "transcription_connection_timeout"
    assert failure.value.numbers == {"timeout_ms": 10_000}
