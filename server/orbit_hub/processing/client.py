"""One bounded JSON-schema completion against the configured processing model.

Every refusal names its cause and carries its numbers (status, bytes, tokens,
seconds) so whoever asked can correct itself; none carries provider text,
material or credentials.
"""

import asyncio
import json
import time
from dataclasses import dataclass
from typing import Any

import httpx
from pydantic import SecretStr

from ..log import logger
from .config import ProcessingConfiguration

# Bound of one request body. A four-hour transcript is about 120 KB of text and
# the model's 131 072-token window holds roughly 500 KB; 4 MiB leaves room for
# the largest window a configured model may declare without letting a caller
# build an unbounded body in memory.
MAX_REQUEST_BYTES = 4 * 1024 * 1024
# Bound of one response body, reasoning included. The longest answer allowed
# (32 768 tokens) is under 200 KB; 1 MiB protects memory from a provider that
# streams without end.
MAX_RESPONSE_BYTES = 1024 * 1024
CHUNK_BYTES = 16 * 1024


class ProcessingError(Exception):
    """A typed refusal: `code` plus the numbers that explain it."""

    def __init__(self, code: str, **details: float | str):
        super().__init__(code)
        self.code = code
        self.details = details

    def as_dict(self) -> dict[str, Any]:
        return {"code": self.code, **self.details}


@dataclass(frozen=True)
class Completion:
    content: dict[str, Any]
    model: str
    prompt_tokens: int | None
    completion_tokens: int | None
    elapsed_ms: int


def request_body(
    configuration: ProcessingConfiguration,
    messages: list[dict[str, str]],
    schema_name: str,
    schema: dict[str, Any],
    max_completion_tokens: int,
) -> bytes:
    payload: dict[str, Any] = {
        "model": configuration.model,
        "messages": messages,
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": schema_name, "strict": True, "schema": schema},
        },
        "max_tokens": max_completion_tokens,
        "stream": False,
        "n": 1,
    }
    if configuration.reasoning != "provider_default":
        payload["chat_template_kwargs"] = {"enable_thinking": configuration.reasoning == "on"}
    body = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(body) > MAX_REQUEST_BYTES:
        raise ProcessingError("request_too_large", bytes=len(body), limit=MAX_REQUEST_BYTES)
    return body


def parse_completion(raw: bytes, max_completion_tokens: int) -> tuple[dict, str, dict]:
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise ProcessingError("invalid_response", reason="not_json") from None
    if not isinstance(document, dict):
        raise ProcessingError("invalid_response", reason="not_object")
    choices = document.get("choices")
    if not isinstance(choices, list) or len(choices) != 1 or not isinstance(choices[0], dict):
        raise ProcessingError("invalid_response", reason="choices")
    choice = choices[0]
    message = choice.get("message")
    if not isinstance(message, dict):
        raise ProcessingError("invalid_response", reason="message")
    usage = document.get("usage") if isinstance(document.get("usage"), dict) else {}
    if message.get("refusal"):
        raise ProcessingError("provider_refusal")
    finish = choice.get("finish_reason")
    if finish == "length":
        raise ProcessingError(
            "incomplete_completion",
            completion_tokens=int(usage.get("completion_tokens") or 0),
            max_completion_tokens=max_completion_tokens,
        )
    if finish != "stop":
        raise ProcessingError("invalid_response", reason=f"finish_{finish}"[:40])
    content = message.get("content")
    if not isinstance(content, str) or not content.strip():
        raise ProcessingError("invalid_response", reason="empty_content")
    try:
        value = json.loads(content)
    except json.JSONDecodeError:
        raise ProcessingError("invalid_json_content", bytes=len(content.encode())) from None
    if not isinstance(value, dict):
        raise ProcessingError("invalid_json_content", bytes=len(content.encode()))
    model = document.get("model") if isinstance(document.get("model"), str) else ""
    return value, model[:128], usage


def provider_error(status: int, raw: bytes) -> ProcessingError:
    """Classify a provider refusal from its shape, never echoing its text."""
    try:
        error = json.loads(raw).get("error", {})
    except (ValueError, AttributeError):
        error = {}
    if not isinstance(error, dict):
        error = {}
    if error.get("type") == "exceed_context_size_error" or "context" in str(error.get("code", "")):
        details = {"status": status}
        for source, target in (("n_prompt_tokens", "prompt_tokens"), ("n_ctx", "context_tokens")):
            if isinstance(error.get(source), int):
                details[target] = error[source]
        return ProcessingError("provider_context_exceeded", **details)
    if status in (401, 403):
        return ProcessingError("provider_rejected_credential", status=status)
    return ProcessingError("provider_http_error", status=status)


class ProcessingClient:
    def __init__(
        self,
        configuration: ProcessingConfiguration,
        *,
        credential: SecretStr,
        transport: httpx.AsyncBaseTransport | None = None,
    ):
        secret = credential.get_secret_value()
        if not 1 <= len(secret) <= 8192 or any(ord(c) < 33 or ord(c) > 126 for c in secret):
            raise ValueError("processing credential must be one printable token")
        self.configuration = configuration
        self._credential = credential
        # One call at a time: the model host serves Deimos and other tools too,
        # and a note waiting its turn costs nothing.
        self._slot = asyncio.Semaphore(1)
        self._http = httpx.AsyncClient(
            timeout=configuration.timeout_seconds,
            follow_redirects=False,
            trust_env=False,
            transport=transport,
        )

    async def close(self) -> None:
        await self._http.aclose()

    async def complete_json(
        self,
        *,
        purpose: str,
        messages: list[dict[str, str]],
        schema_name: str,
        schema: dict[str, Any],
        max_completion_tokens: int | None = None,
    ) -> Completion:
        limit = max_completion_tokens or self.configuration.max_completion_tokens
        body = request_body(self.configuration, messages, schema_name, schema, limit)
        async with self._slot:
            started = time.monotonic()
            try:
                raw, status = await self._post(body)
                if status != 200:
                    raise provider_error(status, raw)
                content, model, usage = parse_completion(raw, limit)
            except ProcessingError as failure:
                logger.info(
                    "processing_failed",
                    extra={"event": "processing_failed", "code": failure.code, "field": purpose},
                )
                raise
            elapsed = int((time.monotonic() - started) * 1000)
        completion = Completion(
            content=content,
            model=model or self.configuration.model,
            prompt_tokens=usage.get("prompt_tokens"),
            completion_tokens=usage.get("completion_tokens"),
            elapsed_ms=elapsed,
        )
        logger.info(
            "processing_completed",
            extra={"event": "processing_completed", "field": purpose, "elapsed_ms": elapsed},
        )
        return completion

    async def _post(self, body: bytes) -> tuple[bytes, int]:
        headers = {
            "Authorization": "Bearer " + self._credential.get_secret_value(),
            "Content-Type": "application/json",
            "Accept": "application/json",
            "Accept-Encoding": "identity",
        }
        timeout = self.configuration.timeout_seconds
        try:
            async with (
                asyncio.timeout(timeout),
                self._http.stream(
                    "POST", self.configuration.endpoint, content=body, headers=headers
                ) as response,
            ):
                if response.headers.get("content-encoding", "identity").lower() != "identity":
                    raise ProcessingError("invalid_response", reason="encoded_body")
                result = bytearray()
                async for chunk in response.aiter_raw(chunk_size=CHUNK_BYTES):
                    if len(result) + len(chunk) > MAX_RESPONSE_BYTES:
                        raise ProcessingError(
                            "response_too_large",
                            bytes=len(result) + len(chunk),
                            limit=MAX_RESPONSE_BYTES,
                        )
                    result.extend(chunk)
                return bytes(result), response.status_code
        except TimeoutError:
            raise ProcessingError("provider_timeout", timeout_seconds=timeout) from None
        except httpx.TimeoutException:
            raise ProcessingError("provider_timeout", timeout_seconds=timeout) from None
        except httpx.HTTPError as failure:
            raise ProcessingError(
                "provider_unreachable", error=type(failure).__name__[:40]
            ) from None
