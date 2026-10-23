"""What the hub answers when the speech-to-text behind Hermes refuses a clip.

Synthetic WAVs only; the provider is a double raising what HermesSpeech
raises for an HTTP status (`from_provider`).
"""

import httpx
import pytest
from conftest import device
from support_app import client
from support_audio import wav_bytes

from orbit_hub.transcription_failure import NO_SPEECH_REFUSAL_MAX_MS, from_provider


def refusing(status):
    """A provider double that answers every clip with HTTP `status`."""

    async def transcribe(_body):
        request = httpx.Request("POST", "https://hermes.invalid/api/audio/transcribe")
        error = httpx.HTTPStatusError(
            "synthetic refusal", request=request, response=httpx.Response(status, request=request)
        )
        raise from_provider(error, timeout_ms=1, connect_timeout_ms=1)

    return transcribe


def clip(milliseconds):
    """A 16 kHz mono clip of `milliseconds` of synthetic low noise."""
    return wav_bytes(b"\x10\x00" * (16 * milliseconds))


def answer(environment, body):
    _, store, _ = environment
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with client(environment) as http:
        return http.post("/v1/audio/transcribe", headers=headers, content=body)


@pytest.mark.parametrize("milliseconds", [1_000, 1_500, NO_SPEECH_REFUSAL_MAX_MS - 1])
def test_a_short_clip_the_provider_refuses_is_silence_not_a_failure(environment, milliseconds):
    # Orbit 0.11.0, 2026-10-23: call turns of 1-2 s came back as
    # `transcription_http_error`, hub 502, provider 400, and stayed as
    # failed notes. A clip that short is heard as nothing.
    environment[2].transcribe = refusing(400)
    response = answer(environment, clip(milliseconds))
    assert response.status_code == 200
    assert response.json() == {"text": ""}


def test_a_long_clip_the_provider_refuses_names_the_refusal(environment):
    environment[2].transcribe = refusing(400)
    response = answer(environment, clip(NO_SPEECH_REFUSAL_MAX_MS))
    assert response.status_code == 502
    body = response.json()
    assert body["code"] == "transcription_provider_refused"
    assert body["refusal"]["provider_status"] == 400
    assert body["refusal"]["duration_ms"] == NO_SPEECH_REFUSAL_MAX_MS
    assert body["refusal"]["retry"] == {"kind": "never"}


@pytest.mark.parametrize("status", [401, 413, 422, 500, 503])
def test_other_provider_statuses_stay_what_they_were(environment, status):
    environment[2].transcribe = refusing(status)
    response = answer(environment, clip(1_000))
    assert response.status_code == 502
    body = response.json()
    assert body["code"] == "transcription_http_error"
    assert body["refusal"]["provider_status"] == status


def test_the_threshold_is_the_call_turns_padding_plus_two_seconds():
    # 300 ms of preroll and 700 ms of closing silence (orbit-core
    # VoicePolicy) frame every call turn: under 3 s holds under 2 s of voice.
    assert NO_SPEECH_REFUSAL_MAX_MS == 3_000
