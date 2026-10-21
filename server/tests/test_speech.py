"""The speech contract against synthetic Hermes transports; no real profile is contacted."""

import base64
import json
import os
from dataclasses import replace

import httpx
import pytest
from conftest import device
from support_app import client

from orbit_hub.hermes import MAX_SPEECH_CLIP_BYTES, HermesAdapter, SpeechClip
from orbit_hub.protocol import HubError

ORIGINAL_ASYNC_CLIENT = httpx.AsyncClient


def credential(tmp_path):
    path = tmp_path / "synthetic-speech-credential"
    if path.exists():
        return path
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(descriptor, b"synthetic-token-only")
    finally:
        os.close(descriptor)
    return path


def install_http(monkeypatch, reply):
    monkeypatch.setattr(
        "orbit_hub.hermes.httpx.AsyncClient",
        lambda **kwargs: ORIGINAL_ASYNC_CLIENT(transport=httpx.MockTransport(reply), **kwargs),
    )


@pytest.mark.parametrize(
    ("mime_type", "audio", "provider"),
    [
        ("audio/mpeg", b"synthetic-mp3", "synthetic-profile-a"),
        ("audio/wav", b"RIFFsynthetic-wave", "synthetic-profile-b"),
    ],
)
async def test_default_uses_profile_clip_without_opening_stream(
    environment, tmp_path, monkeypatch, mime_type, audio, provider
):
    settings, _, _ = environment
    requests = []

    def reply(request):
        requests.append(request)
        return httpx.Response(
            200,
            json={
                "ok": True,
                "data_url": f"data:{mime_type};base64,{base64.b64encode(audio).decode()}",
                "mime_type": mime_type,
                "provider": provider,
            },
        )

    install_http(monkeypatch, reply)
    configured = replace(settings, hermes_credential_file=credential(tmp_path))
    adapter = HermesAdapter(configured)

    result = await adapter.synthesize("frase sintética")

    assert result == SpeechClip(mime_type, provider, audio)
    assert len(requests) == 1
    assert requests[0].url.path == "/api/audio/speak"
    assert requests[0].headers["X-Hermes-Session-Token"] == "synthetic-token-only"
    assert json.loads(requests[0].content) == {"text": "frase sintética", "profile": "deimos"}


async def test_unavailable_speech_and_oversized_clip_are_typed(environment, tmp_path, monkeypatch):
    settings, _, _ = environment

    def unavailable(_request):
        return httpx.Response(503, json={"ok": False})

    install_http(monkeypatch, unavailable)

    configured = replace(settings, hermes_credential_file=credential(tmp_path))
    with pytest.raises(HubError) as error:
        await HermesAdapter(configured).synthesize("frase")
    assert error.value.code == "speech_unavailable"

    oversized = b"x" * (MAX_SPEECH_CLIP_BYTES + 1)

    def too_large(_request):
        return httpx.Response(
            200,
            json={
                "ok": True,
                "data_url": "data:audio/mp4;base64," + base64.b64encode(oversized).decode(),
                "mime_type": "audio/mp4",
                "provider": "synthetic-large",
            },
        )

    install_http(monkeypatch, too_large)
    configured = replace(
        settings,
        hermes_credential_file=credential(tmp_path),
    )
    with pytest.raises(HubError) as error:
        await HermesAdapter(configured).synthesize("frase")
    assert error.value.code == "speech_too_large"


@pytest.mark.parametrize("mime_type", ["audio/mpeg", "audio/wav"])
def test_route_preserves_clip_headers_and_bytes(environment, mime_type):
    settings, store, _ = environment
    audio = b"synthetic-clip"

    class ClipAdapter:
        async def synthesize(self, _text):
            return SpeechClip(mime_type, "synthetic-route", audio)

    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with client((settings, store, ClipAdapter())) as http:
        response = http.post("/v1/audio/speech", headers=headers, json={"text": "frase"})
    assert response.status_code == 200
    assert response.content == audio
    assert response.headers["content-type"] == mime_type
    assert response.headers["X-Orbit-Speech-Kind"] == "clip"
    assert response.headers["X-Orbit-Speech-Provider"] == "synthetic-route"
