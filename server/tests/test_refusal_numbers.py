"""Three hub refusals that used to lose their numbers on the way to the caller.

The storage quota, an HTTP body over its ceiling and a voice clip over its
ceiling each say their cause and carry their sizes (AGENTS.md: a refusal names
its cause and carries its numbers). Codes and existing keys are unchanged;
only fields are added and the body's resource is named truly.
"""

import base64
import hashlib
import os
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_app import client
from support_transcription import wav_fixture

from orbit_hub.assistant.hermes_speech import MAX_SPEECH_RESPONSE_BYTES, HermesSpeech
from orbit_hub.assistant.speech import MAX_SPEECH_CLIP_BYTES
from orbit_hub.captures import MAX_CAPTURE_BYTES
from orbit_hub.protocol import HubError
from orbit_hub.storage import Store
from orbit_mcp.meetings import cause_sentence

QUOTA_FIELDS = {
    "bytes",
    "reserved_bytes",
    "used_bytes",
    "max_bytes",
    "remaining_bytes",
    "limit",
}


def assert_quota_numbers(numbers, quota):
    assert set(numbers) == QUOTA_FIELDS
    assert numbers["max_bytes"] == quota
    assert numbers["reserved_bytes"] == numbers["bytes"] * 3 + 256 * 1024
    assert numbers["used_bytes"] + numbers["reserved_bytes"] > numbers["max_bytes"]
    assert numbers["remaining_bytes"] == max(0, quota - numbers["used_bytes"])
    assert numbers["limit"] == "quota"


def test_every_quota_refusal_of_the_store_carries_its_numbers(environment):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    session = store.chat.create_session(owner, "default", "runtime", "stored", "epoch", 1000)
    body = wav_fixture()
    digest = hashlib.sha256(body).hexdigest()
    constrained = Store(settings.data_dir, settings.key_file, quota_bytes=1)
    try:
        actions = [
            lambda: constrained.devices.create_code(1000),
            lambda: constrained.chat.append_message(session, "assistant", "BLOCKED", now_ms=2),
            lambda: constrained.recordings.put_chunk(
                owner, str(uuid4()), str(uuid4()), "mic_local", 0, digest, body, {"frames": 1}
            ),
        ]
        for action in actions:
            with pytest.raises(HubError) as refused:
                action()
            assert refused.value.code == "quota_exceeded"
            assert_quota_numbers(refused.value.numbers, 1)
    finally:
        constrained.close()


def test_a_chunk_refused_by_the_quota_tells_the_mac_how_much(environment):
    _, store, _ = environment
    paired = device(store)
    body = wav_fixture()
    with client(environment) as http:
        store.space.quota_bytes = 1  # Admission ceiling, never physical disk exhaustion.
        response = http.put(
            "/v1/chunks/new",
            headers={
                "Authorization": "Bearer " + paired["token"],
                "X-Orbit-Recording": str(uuid4()),
                "X-Orbit-Track": "mic_local",
                "X-Orbit-Sequence": "0",
                "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
            },
            content=body,
        )
    assert response.status_code == 507
    payload = response.json()
    assert payload["code"] == "quota_exceeded"
    refusal = payload["refusal"]
    assert refusal["code"] == "quota_exceeded"
    assert_quota_numbers({key: refusal[key] for key in QUOTA_FIELDS}, 1)
    assert refusal["bytes"] >= len(body)


def _chunk(paired, body):
    return {
        "url": "/v1/chunks/oversized",
        "headers": {
            "Authorization": "Bearer " + paired["token"],
            "X-Orbit-Recording": str(uuid4()),
            "X-Orbit-Track": "mic_local",
            "X-Orbit-Sequence": "0",
            "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
        },
    }


def _capture(paired, body):
    return {
        "url": f"/v1/captures/{uuid4()}",
        "headers": {
            "Authorization": "Bearer " + paired["token"],
            "Content-Type": "image/png",
            "X-Orbit-Request": str(uuid4()),
            "X-Orbit-Sha256": hashlib.sha256(body).hexdigest(),
        },
    }


@pytest.mark.parametrize(
    ("method", "route", "maximum", "resource"),
    [
        ("PUT", _capture, MAX_CAPTURE_BYTES, "capture"),
        ("PUT", _chunk, 8 * 1024 * 1024, "recording"),
        (
            "POST",
            lambda paired, _body: {
                "url": "/v1/audio/transcribe",
                "headers": {"Authorization": "Bearer " + paired["token"]},
            },
            24 * 1024 * 1024,
            "audio",
        ),
        ("POST", lambda _paired, _body: {"url": "/v1/pair", "headers": {}}, 2048, "hub_request"),
    ],
)
def test_an_http_body_over_its_ceiling_names_what_it_was(
    environment, method, route, maximum, resource
):
    _, store, _ = environment
    paired = device(store)
    body = b"x" * (maximum + 1)
    with client(environment) as http:
        response = http.request(method, content=body, **route(paired, body))
    assert response.status_code == 413
    payload = response.json()
    assert payload["code"] == "too_large"
    refusal = payload["refusal"]
    assert refusal["code"] == "too_large"
    assert refusal["resource"] == resource
    assert refusal["max_bytes"] == maximum
    assert refusal["bytes"] > maximum


def _credential(tmp_path):
    path = tmp_path / "synthetic-speech-credential"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(descriptor, b"synthetic-token-only")
    finally:
        os.close(descriptor)
    return path


def _clip_reply(encoded: str):
    return {
        "ok": True,
        "data_url": "data:audio/mp4;base64," + encoded,
        "mime_type": "audio/mp4",
        "provider": "synthetic-large",
    }


@pytest.mark.parametrize(
    ("reply", "maximum"),
    [
        # Decoded: 4 MiB + 1 encodes to exactly the largest base64 admitted.
        (
            lambda: httpx.Response(
                200,
                json=_clip_reply(base64.b64encode(b"x" * (MAX_SPEECH_CLIP_BYTES + 1)).decode()),
            ),
            MAX_SPEECH_CLIP_BYTES,
        ),
        # Encoded: longer base64 than any 4 MiB clip, refused before decoding.
        (
            lambda: httpx.Response(
                200,
                json=_clip_reply(base64.b64encode(b"x" * (MAX_SPEECH_CLIP_BYTES + 3)).decode()),
            ),
            MAX_SPEECH_CLIP_BYTES,
        ),
        # Hermes's whole answer, cut while it streams.
        (
            lambda: httpx.Response(200, content=b"x" * (MAX_SPEECH_RESPONSE_BYTES + 1)),
            MAX_SPEECH_RESPONSE_BYTES,
        ),
    ],
)
async def test_a_voice_clip_over_its_ceiling_says_its_size(
    environment, tmp_path, monkeypatch, reply, maximum
):
    settings, _, _ = environment
    original = httpx.AsyncClient
    monkeypatch.setattr(
        "orbit_hub.assistant.hermes_speech.httpx.AsyncClient",
        lambda **kwargs: original(transport=httpx.MockTransport(lambda _r: reply()), **kwargs),
    )
    configured = replace(settings, hermes_credential_file=_credential(tmp_path))
    with pytest.raises(HubError) as refused:
        await HermesSpeech(configured, configured.profile).synthesize("frase sintética")
    assert refused.value.code == "speech_too_large"
    assert refused.value.numbers["max_bytes"] == maximum
    assert refused.value.numbers["bytes"] > maximum


def test_the_assistant_reads_the_quota_rule_that_fired():
    numbers = {"bytes": 10, "reserved_bytes": 262174, "used_bytes": 900, "max_bytes": 1000}
    assert cause_sentence({"cause": "quota_exceeded", **numbers}) == (
        "the hub reached its storage quota: it uses 900 bytes of 1000 and saving this (10 bytes) reserves 262174"
    )
    # A failure stored before the reservation was recorded keeps its numbers.
    older = cause_sentence({"cause": "quota_exceeded", "bytes": 10, "max_bytes": 1000})
    assert older == "code quota_exceeded (bytes: 10, max_bytes: 1000)"
