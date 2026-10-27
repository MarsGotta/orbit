"""AI settings through paired routes and real sockets owned by this test."""

import asyncio
import json

import pytest
from conftest import device
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response
from support_client_e2e import tcp_server
from support_synthetic_stt import form_fields
from test_meeting_language_and_title import routes_only
from test_meeting_views import bearer

from orbit_hub.app import create_app

KEY = "synthetic-ai-key-never-real"


@pytest.fixture
def tools_for_ai(tmp_path):
    from support_openai_compatible import serve
    from support_orbit_mcp import FakeHub, write_mcp_config

    hub = FakeHub()
    with serve(hub.app) as url:
        yield hub, write_mcp_config(tmp_path, url)


class Endpoint:
    def __init__(self, name="cedar", capabilities=("text", "transcription", "speech")):
        self.app, self.requests = FastAPI(), []
        self.headers = []
        self.name, self.capabilities = name, capabilities
        self.status, self.hold = 200, None
        self.app.api_route("/v1/{path:path}", methods=["GET", "POST"])(self.answer)

    async def answer(self, path: str, request: Request):
        self.headers.append(dict(request.headers))
        if self.hold:
            await self.hold.wait()
        if self.status != 200:
            return JSONResponse({"error": {"message": KEY}}, status_code=self.status)
        if path == "models":
            return {"data": [{"id": self.name, "supported_parameters": ["tools"]}]}
        body = await request.body()
        if path == "audio/transcriptions":
            model = (
                json.loads(body)["model"]
                if request.headers["content-type"].startswith("application/json")
                else form_fields(request.headers["content-type"], body)["model"][2].decode()
            )
            result = {"text": "Hola", "words": [{"word": "Hola", "start": 0, "end": 1}]}
            capability = "transcription"
        else:
            payload = json.loads(body)
            model = payload["model"]
            capability = "speech" if path == "audio/speech" else "text"
            result = {
                "id": "fake-completion",
                "object": "chat.completion",
                "created": 0,
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "finish_reason": "stop",
                        "message": {"role": "assistant", "content": "{}"},
                    }
                ],
            }
        if path == "chat/completions" and "response_format" in json.loads(body):
            from support_notes import note_answer

            result["choices"][0]["message"]["content"] = json.dumps(note_answer())
        if path == "chat/completions" and "tools" in json.loads(body):
            result["choices"][0]["message"]["tool_calls"] = [
                {
                    "id": "probe",
                    "type": "function",
                    "function": {"name": "orbit_probe", "arguments": "{}"},
                }
            ]
        self.requests.append((path, model, body))
        if capability not in self.capabilities:
            return JSONResponse({}, status_code=404)
        if path == "chat/completions" and json.loads(body).get("stream"):
            from fastapi.responses import StreamingResponse
            from support_openai_compatible import Text, stream

            return StreamingResponse(iter(stream(Text(("Hola",)))), media_type="text/event-stream")
        return Response(b"fake-mp3", media_type="audio/mpeg") if capability == "speech" else result


async def add(http, owner, url, provider_id="cedar", **extra):
    return await http.put(
        f"/v1/settings/ai/providers/{provider_id}",
        headers=bearer(owner),
        json={
            "name": provider_id.title(),
            "kind": "local",
            "base_url": url,
            "api_key": KEY,
            **extra,
        },
    )


async def select(http, owner, function, provider="cedar", model="cedar"):
    return await http.put(
        f"/v1/settings/ai/functions/{function}",
        headers=bearer(owner),
        json={"provider_id": provider, "model": model},
    )


async def test_discovers_two_endpoints_and_keeps_credentials_on_hub(environment, caplog):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner = device(store)
    first, second = Endpoint(), Endpoint("birch", ("text",))
    async with tcp_server(first.app) as a, tcp_server(second.app) as b, routes_only(app) as http:
        assert (await http.get("/v1/settings/ai")).status_code == 401
        for url, name in [(a, "cedar"), (b, "birch")]:
            saved = await add(http, owner, url, name)
            assert saved.status_code == 200, saved.text
            assert KEY not in saved.text
        result = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        providers = {p["id"]: p for p in result["providers"]}
        assert providers["cedar"]["models"] == [
            {
                "id": "cedar",
                "capabilities": ["text", "tools", "transcription", "word_timestamps", "speech"],
            }
        ]
        assert providers["birch"]["models"] == [{"id": "birch", "capabilities": ["text", "tools"]}]
        assert (await select(http, owner, "speech", "birch", "birch")).status_code == 400
        assert KEY not in json.dumps(result) + caplog.text
        assert KEY.encode() not in store._db.serialize()
        assert {p for p, _, _ in first.requests} == {
            "chat/completions",
            "audio/transcriptions",
            "audio/speech",
        }
        corrected = await add(
            http, owner, b, "birch", models=[{"id": "birch", "capabilities": ["text", "speech"]}]
        )
        assert corrected.status_code == 200, corrected.text
        assert (await select(http, owner, "speech", "birch", "birch")).status_code == 200


async def test_bad_key_and_slow_endpoint_refused_without_saving(environment, monkeypatch, caplog):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner, endpoint = device(store), Endpoint()
    async with tcp_server(endpoint.app) as url, routes_only(app) as http:
        endpoint.status = 401
        failed = await add(http, owner, url)
        assert failed.status_code == 502, failed.text
        assert failed.json()["code"] == "ai_key_invalid"
        assert failed.json()["refusal"]["provider_status"] == 401
        from orbit_hub.ai import discovery

        monkeypatch.setattr(discovery, "PROBE_TIMEOUT_SECONDS", 0.1)
        endpoint.status, endpoint.hold = 200, asyncio.Event()
        try:
            async with asyncio.timeout(2):
                slow = await add(http, owner, url)
            assert slow.json()["code"] == "ai_provider_timeout"
            assert slow.json()["refusal"]["timeout_ms"] == 100
        finally:
            endpoint.hold.set()
        result = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        assert not any(p["id"] == "cedar" for p in result["providers"])
        assert KEY not in failed.text + slow.text + caplog.text


async def test_load_migrates_legacy_key_and_separates_function_choices(environment):
    from orbit_hub.transcription_preferences import Preferences

    settings, store, adapter = environment
    Preferences(store).save("mai", KEY)
    # Fixture of the previous version: only its encrypted preference exists.
    with store.transaction() as db:
        db.execute("DELETE FROM ai_settings")
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner = device(store)
    async with routes_only(app) as http:
        result = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        router = next(p for p in result["providers"] if p["id"] == "openrouter")
        assert router["key_configured"]
        functions = {f["id"]: f for f in result["functions"]}
        assert functions["meeting_transcription"]["provider_id"] == "openrouter"
        assert functions["listening"]["provider_id"] == "openrouter"
        assert "Microsoft a través de OpenRouter" in functions["meeting_transcription"]["privacy"]
        assert KEY not in json.dumps(result)
        assert (
            await select(http, owner, "meeting_transcription", "hub", "whisper")
        ).status_code == 200
        after = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        untouched = {f["id"]: f for f in after["functions"]}
        assert untouched["listening"] == functions["listening"]


async def test_privacy_includes_non_loopback_network(environment):
    from orbit_hub.ai.registry import privacy_line

    assert (
        privacy_line(
            "speech",
            {"kind": "local", "base_url": "https://100.64.0.1/v1", "name": "Cedro"},
            "voice",
        )
        == "El texto sale a Cedro, fuera de tu hub, por tu red o Internet."
    )
    assert (
        privacy_line(
            "listening",
            {"kind": "local", "base_url": "http://127.0.0.1:10000/v1", "name": "Cedro"},
            "audio",
        )
        == "El audio se queda en tu hub."
    )


async def test_each_function_routes_its_choice_without_changing_the_others(environment):
    from pathlib import Path

    from pydantic import SecretStr

    from orbit_hub.ai.consumers import FunctionProcessing, FunctionSpeech, assistant_model
    from orbit_hub.ai.registry import Registry
    from orbit_hub.assistant.openai_settings import OpenAIAgentSettings
    from orbit_hub.processing.client import ProcessingClient
    from orbit_hub.processing.config import ProcessingConfiguration
    from orbit_hub.transcription_preferences import Transcriber

    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner, endpoint = device(store), Endpoint()
    registry = Registry(store, settings)
    from support_audio import wav_bytes

    audio = wav_bytes(b"\x10\x00" * 16000)
    async with tcp_server(endpoint.app) as url, routes_only(app) as http:
        assert (await add(http, owner, url)).status_code == 200
        initial = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()["functions"]
        for function in (
            "meeting_transcription",
            "listening",
            "speech",
            "meeting_notes",
            "vault",
            "assistants",
        ):
            response = await select(http, owner, function)
            assert response.status_code == 200, response.text
            current = response.json()["functions"]
            assert all(a == b for a, b in zip(initial, current, strict=True) if a["id"] != function)
            initial = current
        endpoint.requests.clear()
        transcriber = Transcriber(store, settings.meeting_transcription)
        assert (await transcriber.transcribe(audio, final=True)).text == "Hola"
        assert await transcriber.interactive(adapter, audio) == "Hola"
        speech = FunctionSpeech(registry, adapter)
        assert (await speech.synthesize("Hola")).body == b"fake-mp3"
        configuration = ProcessingConfiguration(
            endpoint=url + "/v1/chat/completions",
            model="operator",
            credential_file=Path("/tmp/synthetic-key"),
            allow_insecure_localhost=True,
        )
        fallback = ProcessingClient(configuration, credential=SecretStr("synthetic-default"))
        processing = FunctionProcessing(registry, fallback)
        try:
            result = await processing.complete_json(
                purpose="note",
                messages=[{"role": "user", "content": "Invented"}],
                schema_name="test",
                schema={},
            )
            assert result.model == "cedar"
        finally:
            await processing.close()
        generic = OpenAIAgentSettings(
            "openai", url + "/v1", "operator", None, Path("/tmp/synthetic-mcp"), False, None
        )
        model = assistant_model(registry, generic)
        from pydantic_ai import Agent

        await Agent(model).run("Invented")
        assert registry.selected("vault")["model"] == "cedar"
        configured = await http.put(
            "/v1/vault/config",
            headers=bearer(owner),
            json={"kind": "folder", "path": str(settings.data_dir)},
        )
        assert configured.status_code == 200, configured.text
        vault = (await http.get("/v1/vault/config", headers=bearer(owner))).json()
        assert vault["maintenance_model"] == "cedar"
        assert [path for path, _, _ in endpoint.requests] == [
            "audio/transcriptions",
            "audio/transcriptions",
            "audio/speech",
            "chat/completions",
            "chat/completions",
        ]
        assert all(model == "cedar" for _, model, _ in endpoint.requests)


async def test_default_listening_and_speech_survive_a_meeting_only_override(environment):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner, endpoint = device(store), Endpoint()
    async with (
        tcp_server(endpoint.app) as url,
        app.router.lifespan_context(app),
        routes_only(app) as http,
    ):
        initial = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        assert (await add(http, owner, url)).status_code == 200
        assert (await select(http, owner, "meeting_transcription")).status_code == 200
        endpoint.requests.clear()
        from support_audio import wav_bytes

        heard = await http.post(
            "/v1/audio/transcribe", headers=bearer(owner), content=wav_bytes(b"\x10\x00" * 16000)
        )
        assert heard.status_code == 200, heard.text
        spoken = await http.post("/v1/audio/speech", headers=bearer(owner), json={"text": "Hola"})
        assert spoken.status_code == 200, spoken.text
        assert not endpoint.requests
        after = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        assert all(
            a == b
            for a, b in zip(initial["functions"], after["functions"], strict=True)
            if a["id"] != "meeting_transcription"
        )
        assert (await select(http, owner, "listening")).status_code == 200
        heard = await http.post(
            "/v1/audio/transcribe", headers=bearer(owner), content=wav_bytes(b"\x10\x00" * 16000)
        )
        assert heard.json()["text"] == "Hola"
        assert endpoint.requests[-1][1] == "cedar"
        assert (await select(http, owner, "speech")).status_code == 200
        spoken = await http.post("/v1/audio/speech", headers=bearer(owner), json={"text": "Hola"})
        assert spoken.content == b"fake-mp3"
        assert endpoint.requests[-1][0] == "audio/speech"


async def test_note_writer_uses_selected_text_model(environment):
    from test_notes import recording

    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter, now=lambda: 2000)
    owner, endpoint = device(store), Endpoint()
    identity = recording(store, owner["device_id"])
    async with tcp_server(endpoint.app) as url, routes_only(app) as http:
        assert (await add(http, owner, url)).status_code == 200
        assert (await select(http, owner, "meeting_notes")).status_code == 200
        endpoint.requests.clear()
        app.state.notes.repository.initialize()
        asked = await http.post(f"/v1/recordings/{identity}/note", headers=bearer(owner), json={})
        assert asked.status_code == 202, asked.text
        assert await app.state.notes.run_once()
        document = (await http.get(f"/v1/recordings/{identity}/note", headers=bearer(owner))).json()
        assert document["state"] == "ready", document
        assert document["note"]["model"] == "cedar"
        assert endpoint.requests[-1][0:2] == ("chat/completions", "cedar")


def test_live_generic_assistant_changes_provider_on_next_turn(environment, tools_for_ai, caplog):
    from dataclasses import replace

    from fastapi.testclient import TestClient
    from support_openai_compatible import serve
    from test_openai_assistant import entry, next_event, send, turn_frames

    settings, store, adapter = environment
    _, mcp_config = tools_for_ai
    first, second = Endpoint("cedar"), Endpoint("birch")
    with serve(first.app) as a, serve(second.app) as b:
        settings = replace(settings, agents=(entry(a, mcp_config),))
        app = create_app(settings, store=store, speech=adapter)
        owner = device(store)
        headers = bearer(owner)
        with TestClient(app, base_url="https://localhost") as http:
            saved = http.put(
                "/v1/settings/ai/providers/birch",
                headers=headers,
                json={"name": "Abedul", "kind": "local", "base_url": b, "api_key": KEY},
            )
            assert saved.status_code == 200, saved.text
            first.requests.clear()
            second.requests.clear()
            with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
                assert next_event(ws)["event"] == "ready"
                initial = turn_frames(ws, send(ws, "Hola"))
                assert initial[-1]["event"] == "chat.completed", initial
                session = initial[0]["data"]["session_id"]
                selected = http.put(
                    "/v1/settings/ai/functions/assistants",
                    headers=headers,
                    json={"provider_id": "birch", "model": "birch"},
                )
                assert selected.status_code == 200, selected.text
                changed = turn_frames(ws, send(ws, "Otra pregunta", session))
                assert changed[-1]["event"] == "chat.completed", changed
                assert KEY not in json.dumps(initial + changed) + caplog.text
            assert len(first.requests) == 1 and first.requests[0][1] == "modelo-sintetico"
            assert len(second.requests) == 1 and second.requests[0][1] == "birch"


async def test_changing_endpoint_does_not_forward_a_saved_key(environment):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner, first, second = device(store), Endpoint(), Endpoint()
    async with tcp_server(first.app) as a, tcp_server(second.app) as b, routes_only(app) as http:
        assert (await add(http, owner, a)).status_code == 200
        assert all(h.get("authorization") == "Bearer " + KEY for h in first.headers)
        assert (await add(http, owner, a, api_key=None)).status_code == 200
        changed = await add(http, owner, b, api_key=None)
        assert changed.status_code == 200, changed.text
        assert all("authorization" not in h for h in second.headers)
        provider = next(p for p in changed.json()["providers"] if p["id"] == "cedar")
        assert not provider["key_configured"]


async def test_openrouter_catalog_and_legacy_save_preserve_other_functions(
    environment, monkeypatch
):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    owner, endpoint = device(store), Endpoint()
    async with tcp_server(endpoint.app) as url, routes_only(app) as http:
        monkeypatch.setattr("orbit_hub.ai.routes.OPENROUTER_URL", url + "/v1")
        monkeypatch.setattr(
            "orbit_hub.openrouter_transcription.URL", url + "/v1/audio/transcriptions"
        )
        saved = await add(http, owner, url + "/v1", "openrouter", kind="openrouter")
        assert saved.status_code == 200, saved.text
        chosen = await select(http, owner, "meeting_notes", "openrouter", "cedar")
        assert chosen.status_code == 200, chosen.text
        before = chosen.json()
        legacy = await http.put(
            "/v1/settings/transcription", headers=bearer(owner), json={"model": "whisper"}
        )
        assert legacy.status_code == 200, legacy.text
        after = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        assert after["providers"] == before["providers"]
        assert next(f for f in after["functions"] if f["id"] == "meeting_notes") == next(
            f for f in before["functions"] if f["id"] == "meeting_notes"
        )


def test_listening_and_speech_are_independent_without_an_operator_voice(environment, tmp_path):
    from dataclasses import replace

    from support_audio import wav_bytes
    from support_openai_compatible import serve
    from test_agent_kinds import every_kind, hub

    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path), default_agent="vega")
    owner, endpoint = device(store), Endpoint()
    headers = bearer(owner)
    with serve(endpoint.app) as url, hub(configured, store) as http:
        saved = http.put(
            "/v1/settings/ai/providers/cedar",
            headers=headers,
            json={"kind": "local", "name": "Cedro", "base_url": url},
        )
        assert saved.status_code == 200, saved.text
        assert (
            http.put(
                "/v1/settings/ai/functions/speech",
                headers=headers,
                json={"provider_id": "cedar", "model": "cedar"},
            ).status_code
            == 200
        )
        assert (
            http.post("/v1/audio/speech", headers=headers, json={"text": "Hola"}).content
            == b"fake-mp3"
        )
        audio = wav_bytes(b"\x10\x00" * 16000)
        before = http.post("/v1/audio/transcribe", headers=headers, content=audio)
        assert before.status_code == 503, before.text
        assert before.json()["refusal"]["agent_id"] == "vega"
        selected = http.put(
            "/v1/settings/ai/functions/listening",
            headers=headers,
            json={"provider_id": "cedar", "model": "cedar"},
        )
        assert selected.status_code == 200, selected.text
        heard = http.post("/v1/audio/transcribe", headers=headers, content=audio)
        assert heard.json()["text"] == "Hola", heard.text


async def test_selected_listening_can_use_a_shorter_provider_budget(environment):
    from support_audio import wav_bytes

    from orbit_hub.errors import HubError
    from orbit_hub.meeting_transcription import MeetingTranscriber

    endpoint = Endpoint()
    endpoint.hold = asyncio.Event()
    async with tcp_server(endpoint.app) as url:
        transcriber = MeetingTranscriber({"endpoint": url, "model": "cedar"}, timeout=0.1)
        try:
            async with asyncio.timeout(2):
                with pytest.raises(HubError) as refused:
                    await transcriber.transcribe(wav_bytes(b"\x10\x00" * 16000))
            assert refused.value.code == "transcription_timeout"
            assert refused.value.numbers["timeout_ms"] == 100
        finally:
            endpoint.hold.set()


async def test_operator_privacy_names_the_configured_destination(environment):
    from dataclasses import replace

    from support_notes import FakeModel, configuration

    settings, store, adapter = environment
    processing = configuration(endpoint="https://model.example.test/v1/chat/completions")
    app = create_app(
        replace(settings, processing=processing),
        store=store,
        assistant=adapter,
        speech=adapter,
        processing_client=FakeModel(configuration=processing),
    )
    owner = device(store)
    async with routes_only(app) as http:
        result = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
        note = next(f for f in result["functions"] if f["id"] == "meeting_notes")
        assert "model.example.test" in note["privacy"]
        assert "fuera de tu hub" in note["privacy"]
