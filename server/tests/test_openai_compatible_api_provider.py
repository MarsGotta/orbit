"""Any OpenAI-compatible API as a provider in Settings › AI, not only OpenRouter.

An `api` provider is a service at any base URL, with its key, whose catalogue is
read but never probed: a paid API is not sent test requests for every model it
lists, and its catalogue may be far larger than a local server's. The person
names the models they use from that catalogue and what each one does. The
service here is synthetic, on a port this process binds; names are invented.
"""

import json
from contextlib import asynccontextmanager

import httpx
import pytest
from conftest import device
from fastapi import FastAPI, Request
from support_audio import wav_bytes
from support_client_e2e import tcp_server
from support_notes import note_answer
from test_meeting_views import bearer

from orbit_hub.ai.types import MAX_LOCAL_MODELS
from orbit_hub.app import create_app

KEY = "clave-sintetica-de-la-nube"
TRANSCRIBER = "transcriptor-sintetico"
WRITER = "redactor-sintetico"
DECLARED = [
    {"id": TRANSCRIBER, "capabilities": ["transcription", "word_timestamps"]},
    {"id": WRITER, "capabilities": ["text"]},
]


class CloudApi:
    """An OpenAI-compatible cloud API with a catalogue larger than a local one."""

    def __init__(self):
        self.app = FastAPI()
        self.requests: list[tuple[str, str, str | None]] = []
        self.bodies: list[bytes] = []
        catalogue = [{"id": f"modelo-{n}", "object": "model"} for n in range(MAX_LOCAL_MODELS)]
        catalogue += [{"id": TRANSCRIBER, "object": "model"}, {"id": WRITER, "object": "model"}]

        @self.app.api_route("/v1/{path:path}", methods=["GET", "POST"])
        async def answer(path: str, request: Request):
            self.requests.append(
                (request.method, "/v1/" + path, request.headers.get("authorization"))
            )
            body = await request.body()
            self.bodies.append(body)
            if path == "models":
                return {"object": "list", "data": catalogue}
            if path == "audio/transcriptions":
                return {
                    "text": "Dictado por la nube sintética",
                    "words": [{"word": "Dictado", "start": 0.0, "end": 0.5}],
                }
            if path == "chat/completions":
                return {
                    "id": "nube-sintetica",
                    "object": "chat.completion",
                    "created": 0,
                    "model": json.loads(body)["model"],
                    "choices": [
                        {
                            "index": 0,
                            "finish_reason": "stop",
                            "message": {"role": "assistant", "content": json.dumps(note_answer())},
                        }
                    ],
                }
            return {"error": {"message": "unknown synthetic route"}}


async def save(http, owner, url, models=DECLARED, provider_id="nube-sintetica"):
    body = {"name": "Nube Sintética", "kind": "api", "base_url": url + "/v1", "api_key": KEY}
    if models is not None:
        body["models"] = models
    return await http.put(
        f"/v1/settings/ai/providers/{provider_id}", headers=bearer(owner), json=body
    )


@asynccontextmanager
async def hub(environment):
    """The hub as it runs: its lifespan starts the transcription worker."""
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter, now=lambda: 1001)
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client,
    ):
        yield client


async def test_an_api_is_added_by_url_key_and_the_models_named_without_probing(environment):
    _, store, _ = environment
    owner, api = device(store), CloudApi()
    async with tcp_server(api.app) as url, hub(environment) as http:
        saved = await save(http, owner, url)
    assert saved.status_code == 200, saved.text
    # The catalogue is read with the key; no model is sent a test request.
    assert api.requests == [("GET", "/v1/models", "Bearer " + KEY)]
    [provider] = [p for p in saved.json()["providers"] if p["id"] == "nube-sintetica"]
    assert provider["kind"] == "api"
    assert provider["base_url"] == url + "/v1"
    assert provider["key_configured"] is True
    assert provider["models"] == DECLARED
    assert KEY not in saved.text


async def test_dictation_uses_the_chosen_api_at_its_base_url_with_its_key(environment):
    _, store, _ = environment
    owner, api = device(store), CloudApi()
    async with tcp_server(api.app) as url, hub(environment) as http:
        assert (await save(http, owner, url)).status_code == 200
        chosen = await http.put(
            "/v1/settings/ai/functions/listening",
            headers=bearer(owner),
            json={"provider_id": "nube-sintetica", "model": TRANSCRIBER},
        )
        assert chosen.status_code == 200, chosen.text
        heard = await http.post(
            "/v1/audio/transcribe",
            headers=bearer(owner),
            content=wav_bytes(b"\x10\x00" * 16000),
        )
    assert heard.status_code == 200, heard.text
    assert heard.json() == {"text": "Dictado por la nube sintética"}
    assert api.requests[-1] == ("POST", "/v1/audio/transcriptions", "Bearer " + KEY)
    assert TRANSCRIBER.encode() in api.bodies[-1]


async def test_notes_can_use_the_chosen_api(environment):
    _, store, _ = environment
    owner, api = device(store), CloudApi()
    async with tcp_server(api.app) as url, hub(environment) as http:
        assert (await save(http, owner, url)).status_code == 200
        chosen = await http.put(
            "/v1/settings/ai/functions/meeting_notes",
            headers=bearer(owner),
            json={"provider_id": "nube-sintetica", "model": WRITER},
        )
        assert chosen.status_code == 200, chosen.text
        listed = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
    [notes] = [f for f in listed["functions"] if f["id"] == "meeting_notes"]
    assert (notes["provider_id"], notes["model"]) == ("nube-sintetica", WRITER)


async def test_a_model_the_api_does_not_list_is_refused(environment):
    _, store, _ = environment
    owner, api = device(store), CloudApi()
    invented = [{"id": "modelo-que-no-existe", "capabilities": ["text"]}]
    async with tcp_server(api.app) as url, hub(environment) as http:
        refused = await save(http, owner, url, models=invented)
        listed = (await http.get("/v1/settings/ai", headers=bearer(owner))).json()
    assert refused.status_code == 400, refused.text
    assert refused.json()["code"] == "ai_models_invalid"
    assert all(p["id"] != "nube-sintetica" for p in listed["providers"])


@pytest.mark.parametrize("models", [None, []])
async def test_an_api_without_named_models_is_refused(environment, models):
    _, store, _ = environment
    owner, api = device(store), CloudApi()
    async with tcp_server(api.app) as url, hub(environment) as http:
        refused = await save(http, owner, url, models=models)
    assert refused.status_code == 400, refused.text
    assert refused.json()["code"] == "ai_models_missing"
    # Refused before the catalogue is even read.
    assert api.requests == []
