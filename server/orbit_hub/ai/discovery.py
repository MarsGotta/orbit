"""Bounded discovery with invented text and silence, never user material."""

import asyncio
import json

import httpx

from ..errors import HubError
from ..transcription_preferences import validation_audio
from .types import MAI, MAX_CLOUD_MODELS, MAX_LOCAL_MODELS

# One complete save, including every model probe. Fits inside the Mac's 90 s
# settings request budget. Per-request timeouts alone allow a drip-fed catalog
# or many slow models to keep settings waiting indefinitely.
PROBE_TIMEOUT_SECONDS = 20
# Catalog/probe responses only: 2 MiB holds a cloud catalog of 2048 model IDs
# and metadata; this does not limit generated text, speech or meeting audio.
MAX_DISCOVERY_BYTES = 2 * 1024 * 1024


def api_root(url):
    return url.rstrip("/") if url.rstrip("/").endswith("/v1") else url.rstrip("/") + "/v1"


def refusal(code, message, **numbers):
    return HubError(code, message, http_status=502, numbers=numbers or None)


async def request(client, method, path, **kwargs):
    async with client.stream(method, path, **kwargs) as response:
        if response.status_code in (401, 403):
            raise refusal(
                "ai_key_invalid",
                "The provider rejected the key",
                provider_status=response.status_code,
            )
        if response.status_code in (402, 429) or response.status_code >= 500:
            raise refusal(
                "ai_provider_http_error",
                f"The provider returned HTTP {response.status_code}; comprueba su saldo, cuota o disponibilidad",
                provider_status=response.status_code,
            )
        body = bytearray()
        async for chunk in response.aiter_bytes():
            body.extend(chunk)
            if len(body) > MAX_DISCOVERY_BYTES:
                raise refusal(
                    "ai_response_too_large",
                    "The provider response exceeds the limit",
                    bytes=len(body),
                    max_bytes=MAX_DISCOVERY_BYTES,
                )
        return response.status_code, bytes(body), response.headers


async def discover(provider, key):
    headers = {"Authorization": "Bearer " + key} if key else {}
    try:
        async with (
            asyncio.timeout(PROBE_TIMEOUT_SECONDS),
            httpx.AsyncClient(
                base_url=api_root(provider.base_url) + "/",
                headers=headers,
                timeout=PROBE_TIMEOUT_SECONDS,
                follow_redirects=False,
                trust_env=False,
            ) as client,
        ):
            return await catalog(client, provider.kind)
    except (TimeoutError, httpx.TimeoutException):
        raise refusal(
            "ai_provider_timeout",
            f"The provider did not finish verification within {int(PROBE_TIMEOUT_SECONDS * 1000)} ms",
            timeout_ms=int(PROBE_TIMEOUT_SECONDS * 1000),
        ) from None
    except httpx.HTTPError:
        raise refusal(
            "ai_provider_unreachable", "Could not connect to the provider; check its URL"
        ) from None
    except (ValueError, TypeError, KeyError, AttributeError, IndexError):
        raise refusal(
            "ai_invalid_catalog", "The provider returned an invalid model catalogue"
        ) from None


async def catalog(client, kind):
    status, body, _ = await request(client, "GET", "models")
    if status != 200:
        raise refusal(
            "ai_provider_http_error",
            f"The catalogue returned HTTP {status}",
            provider_status=status,
        )
    entries = json.loads(body)["data"]
    maximum = MAX_LOCAL_MODELS if kind == "local" else MAX_CLOUD_MODELS
    if not isinstance(entries, list):
        raise TypeError("Catalog must be a list")
    if not 1 <= len(entries) <= maximum:
        raise refusal(
            "ai_catalog_size",
            f"The catalogue has {len(entries)} models; it must have between 1 and {maximum}",
            count=len(entries),
            max_count=maximum,
        )
    models = []
    for entry in entries:
        model = entry["id"]
        if not isinstance(model, str) or not 1 <= len(model) <= 256:
            raise ValueError("Invalid model ID")
        capabilities = await probe(client, model) if kind == "local" else cloud_capabilities(entry)
        models.append({"id": model, "capabilities": capabilities})
    if kind == "openrouter":
        # /models is public: validate the credential with the existing MAI
        # silence probe; no free-text error from a provider reaches a caller.
        from ..meeting_transcription import MeetingTranscriptionConfiguration
        from ..openrouter_transcription import OpenRouterTranscriber

        config = MeetingTranscriptionConfiguration(endpoint="http://localhost", model="whisper")
        await OpenRouterTranscriber(
            client.headers["Authorization"][7:], config, timeout=PROBE_TIMEOUT_SECONDS
        ).transcribe(validation_audio())
        if not any(m["id"] == MAI for m in models):
            models.append({"id": MAI, "capabilities": ["transcription", "word_timestamps"]})
    return models


def cloud_capabilities(entry):
    if entry["id"] == MAI:
        return ["transcription", "word_timestamps"]
    architecture = entry.get("architecture", {})
    output = architecture.get("output_modalities", ["text"])
    if "text" not in output:
        return []
    return ["text", "tools"] if "tools" in entry.get("supported_parameters", []) else ["text"]


async def probe_text(client, model):
    payload = {
        "model": model,
        "messages": [{"role": "user", "content": "Reply OK."}],
        "max_tokens": 16,
    }
    capabilities = []
    status, body, _ = await request(client, "POST", "chat/completions", json=payload)
    if status != 200 or not isinstance(json.loads(body).get("choices"), list):
        return capabilities
    capabilities.append("text")
    payload.update(
        {
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "orbit_probe",
                        "description": "A synthetic capability probe",
                        "parameters": {"type": "object", "properties": {}},
                    },
                }
            ],
            "tool_choice": {"type": "function", "function": {"name": "orbit_probe"}},
        }
    )
    status, body, _ = await request(client, "POST", "chat/completions", json=payload)
    if status == 200:
        choices = json.loads(body).get("choices", [])
        if choices and choices[0].get("message", {}).get("tool_calls"):
            capabilities.append("tools")
    return capabilities


async def probe(client, model):
    capabilities = await probe_text(client, model)
    status, body, _ = await request(
        client,
        "POST",
        "audio/transcriptions",
        data={
            "model": model,
            "response_format": "verbose_json",
            "timestamp_granularities[]": "word",
        },
        files={"file": ("silence.wav", validation_audio(), "audio/wav")},
    )
    if status == 200:
        result = json.loads(body)
        if isinstance(result.get("text"), str):
            capabilities.append("transcription")
            if isinstance(result.get("words"), list) and result["words"]:
                from ..meeting_transcription import timed_words

                timed_words(result["words"])
                capabilities.append("word_timestamps")
    status, body, headers = await request(
        client,
        "POST",
        "audio/speech",
        json={
            "model": model,
            "input": "Hola",
            "voice": "alloy",
            "response_format": "mp3",
        },
    )
    if status == 200 and body and headers.get("content-type", "").startswith("audio/"):
        capabilities.append("speech")
    return capabilities
