"""The processing client: one configured model, typed refusals with their numbers."""

import asyncio
import json
import logging
import os
from pathlib import Path

import httpx
import pytest
from pydantic import SecretStr, ValidationError

from orbit_hub.config import Settings
from orbit_hub.processing import ProcessingClient, ProcessingConfiguration, open_client
from orbit_hub.processing.client import MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ProcessingError

SECRET = "synthetic-processing-secret"
SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["title"],
    "properties": {"title": {"type": "string"}},
}


def configuration(**overrides) -> ProcessingConfiguration:
    values = {
        "endpoint": "https://llm.example.test/v1/chat/completions",
        "model": "qwen3.8-flash-next",
        "credential_file": "/srv/orbit/secrets/processing.key",
        "reasoning": "off",
        "timeout_seconds": 5,
    }
    values.update(overrides)
    return ProcessingConfiguration.model_validate(values)


def completion(content, *, finish="stop", usage=None, **extra) -> dict:
    return {
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "qwen3.8-flash-next",
        "choices": [
            {
                "index": 0,
                "finish_reason": finish,
                "message": {"role": "assistant", "content": content, **extra},
            }
        ],
        "usage": usage or {"prompt_tokens": 120, "completion_tokens": 40},
    }


def reply(status: int, *, json_body=None, text: str | None = None, content: bytes | None = None):
    """A streamed response, as a real provider sends it."""
    if json_body is not None:
        content = json.dumps(json_body).encode()
    elif text is not None:
        content = text.encode()
    return httpx.Response(status, stream=httpx.ByteStream(content or b""))


def client_for(handler, **overrides) -> ProcessingClient:
    return ProcessingClient(
        configuration(**overrides),
        credential=SecretStr(SECRET),
        transport=httpx.MockTransport(handler),
    )


async def ask(client: ProcessingClient):
    return await client.complete_json(
        purpose="note",
        messages=[{"role": "user", "content": "transcripción sintética"}],
        schema_name="note",
        schema=SCHEMA,
    )


@pytest.mark.parametrize(
    "overrides,field",
    [
        ({"endpoint": "http://llm.example.test/v1/chat/completions"}, "HTTPS"),
        ({"endpoint": "https://llm.example.test/v1/completions"}, "/chat/completions"),
        ({"endpoint": "https://user:pw@llm.example.test/v1/chat/completions"}, "plain URL"),
        ({"credential_file": "relative/processing.key"}, "absolute"),
        ({"timeout_seconds": 901}, "timeout_seconds"),
        ({"max_completion_tokens": 40_000}, "max_completion_tokens"),
        ({"reasoning": "maybe"}, "reasoning"),
    ],
)
def test_configuration_refuses_with_the_field_that_is_wrong(overrides, field):
    with pytest.raises(ValidationError) as caught:
        configuration(**overrides)
    assert field in str(caught.value)


def test_loopback_http_needs_the_explicit_opt_in():
    url = "http://127.0.0.1:0/v1/chat/completions"
    with pytest.raises(ValidationError):
        configuration(endpoint=url)
    assert configuration(endpoint=url, allow_insecure_localhost=True).endpoint == url


async def test_request_asks_for_strict_json_and_turns_reasoning_off():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["body"] = json.loads(request.content)
        seen["auth"] = request.headers["authorization"]
        return reply(200, json_body=completion('{"title": "Planificación"}'))

    result = await ask(client_for(handler))
    body = seen["body"]
    assert result.content == {"title": "Planificación"}
    assert (result.prompt_tokens, result.completion_tokens) == (120, 40)
    assert body["response_format"]["json_schema"] == {
        "name": "note",
        "strict": True,
        "schema": SCHEMA,
    }
    assert body["chat_template_kwargs"] == {"enable_thinking": False}
    assert body["max_tokens"] == 8192 and body["stream"] is False
    assert seen["auth"] == "Bearer " + SECRET


async def test_provider_default_reasoning_sends_no_template_field():
    seen = {}

    def handler(request):
        seen.update(json.loads(request.content))
        return reply(200, json_body=completion('{"title": "x"}'))

    await ask(client_for(handler, reasoning="provider_default"))
    assert "chat_template_kwargs" not in seen


@pytest.mark.parametrize(
    "make,code,details",
    [
        (
            lambda: reply(
                200,
                json_body=completion(
                    '{"title"', finish="length", usage={"completion_tokens": 8192}
                ),
            ),
            "incomplete_completion",
            {"completion_tokens": 8192, "max_completion_tokens": 8192},
        ),
        (
            lambda: reply(
                400,
                json_body={
                    "error": {
                        "type": "exceed_context_size_error",
                        "n_prompt_tokens": 140000,
                        "n_ctx": 131072,
                    }
                },
            ),
            "provider_context_exceeded",
            {"status": 400, "prompt_tokens": 140000, "context_tokens": 131072},
        ),
        (
            lambda: reply(401, json_body={"error": {"message": "bad key"}}),
            "provider_rejected_credential",
            {"status": 401},
        ),
        (lambda: reply(503, text="overloaded"), "provider_http_error", {"status": 503}),
        (lambda: reply(200, text="not json"), "invalid_response", {"reason": "not_json"}),
        (
            lambda: reply(200, json_body=completion("no es json")),
            "invalid_json_content",
            {"bytes": 10},
        ),
        (lambda: reply(200, json_body=completion(None, refusal="no")), "provider_refusal", {}),
        (
            lambda: reply(200, json_body=completion('{"title": "x"}', finish="tool_calls")),
            "invalid_response",
            {"reason": "finish_tool_calls"},
        ),
    ],
)
async def test_each_refusal_names_its_cause_and_numbers(make, code, details):
    with pytest.raises(ProcessingError) as caught:
        await ask(client_for(lambda request: make()))
    assert caught.value.code == code
    assert caught.value.details == details


async def test_an_endless_answer_is_cut_at_the_response_bound():
    big = b"x" * (MAX_RESPONSE_BYTES + 1)
    with pytest.raises(ProcessingError) as caught:
        await ask(client_for(lambda request: reply(200, content=big)))
    assert caught.value.code == "response_too_large"
    assert caught.value.details["limit"] == MAX_RESPONSE_BYTES
    assert caught.value.details["bytes"] > MAX_RESPONSE_BYTES


async def test_an_oversized_request_is_refused_before_it_is_sent():
    calls = []

    def handler(request):
        calls.append(request)
        return reply(200, json_body=completion('{"title": "x"}'))

    client = client_for(handler)
    with pytest.raises(ProcessingError) as caught:
        await client.complete_json(
            purpose="note",
            messages=[{"role": "user", "content": "x" * MAX_REQUEST_BYTES}],
            schema_name="note",
            schema=SCHEMA,
        )
    assert caught.value.code == "request_too_large"
    assert caught.value.details["limit"] == MAX_REQUEST_BYTES
    assert calls == []


async def test_timeout_and_unreachable_are_different_causes():
    def slow(request):
        raise httpx.ReadTimeout("slow", request=request)

    def down(request):
        raise httpx.ConnectError("refused", request=request)

    with pytest.raises(ProcessingError) as timed_out:
        await ask(client_for(slow))
    with pytest.raises(ProcessingError) as unreachable:
        await ask(client_for(down))
    assert timed_out.value.as_dict() == {"code": "provider_timeout", "timeout_seconds": 5.0}
    assert unreachable.value.as_dict() == {"code": "provider_unreachable", "error": "ConnectError"}


async def test_calls_never_overlap_on_the_model_host():
    active, peak = 0, 0

    async def handler(request):
        nonlocal active, peak
        active += 1
        peak = max(peak, active)
        await asyncio.sleep(0)
        await asyncio.sleep(0)
        active -= 1
        return reply(200, json_body=completion('{"title": "x"}'))

    client = client_for(handler)
    await asyncio.gather(*(ask(client) for _ in range(4)))
    assert peak == 1


async def test_neither_refusals_nor_logs_carry_the_secret_or_the_material(caplog):
    def handler(request):
        return reply(500, json_body={"error": {"message": "echo " + SECRET}})

    caplog.set_level(logging.INFO, logger="orbit_hub")
    with pytest.raises(ProcessingError) as caught:
        await ask(client_for(handler))
    rendered = repr(caught.value.as_dict()) + caplog.text
    assert SECRET not in rendered
    assert "transcripción sintética" not in rendered


def write_secret(path: Path, mode: int) -> Path:
    path.write_text(SECRET + "\n")
    os.chmod(path, mode)
    return path


def test_open_client_reads_a_private_credential(tmp_path):
    key = write_secret(tmp_path / "processing.key", 0o600)
    client = open_client(configuration(credential_file=str(key)))
    assert client.configuration.model == "qwen3.8-flash-next"


def test_open_client_refuses_a_readable_credential(tmp_path):
    key = write_secret(tmp_path / "processing.key", 0o644)
    with pytest.raises(ValueError, match="0600"):
        open_client(configuration(credential_file=str(key)))


def minimal_settings(tmp_path: Path, processing: str) -> Path:
    path = tmp_path / "orbit.toml"
    path.write_text(
        f"""
data_dir = "{tmp_path}/data"
key_file = "{tmp_path}/storage.key"
hermes_url = "http://127.0.0.1:0"
hermes_credential_file = "{tmp_path}/hermes"
allow_insecure_localhost = true

[meeting_transcription]
endpoint = "http://127.0.0.1:0"
model = "openai/whisper-large-v3"

{processing}
"""
    )
    return path


def test_settings_load_the_processing_table(tmp_path):
    path = minimal_settings(
        tmp_path,
        """[processing]
endpoint = "https://llama.example.co/v1/chat/completions"
model = "qwen3.8-flash-next"
credential_file = "/srv/orbit/secrets/llm-procesamiento.key"
reasoning = "off"
""",
    )
    settings = Settings.load(path)
    assert settings.processing is not None
    assert settings.processing.model == "qwen3.8-flash-next"
    assert settings.processing.reasoning == "off"


def test_a_wrong_processing_table_names_the_field(tmp_path):
    path = minimal_settings(
        tmp_path,
        """[processing]
endpoint = "https://llama.example.co/v1/chat/completions"
model = "qwen3.8-flash-next"
credential_file = "/srv/orbit/secrets/llm-procesamiento.key"
timeout_seconds = 5000
""",
    )
    with pytest.raises(ValueError, match="timeout_seconds"):
        Settings.load(path)


def test_without_the_table_the_hub_still_loads(tmp_path):
    assert Settings.load(minimal_settings(tmp_path, "")).processing is None
