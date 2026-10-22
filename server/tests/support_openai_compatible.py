"""An OpenAI-compatible provider, played by the test process itself.

The `kind = "openai"` assistant calls `POST {base_url}/chat/completions` with
`stream: true` and reads Server-Sent Events, as OpenAI, OpenRouter, LiteLLM,
Ollama and vLLM answer it; read-aloud calls `POST {base_url}/audio/speech`.
`FakeProvider` is that service with scripted replies, one per request (the
last repeats); every request body is kept in `requests`. `serve` runs an app
from a thread on a port this process binds, so the hub's real clients cross a
real socket to it and to nothing else (see `support_network_fence`).
"""

import asyncio
import json
import socket
import threading
import time
from contextlib import contextmanager
from dataclasses import dataclass, field

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response, StreamingResponse

START_TIMEOUT_SECONDS = 10


@dataclass(frozen=True)
class Text:
    """A streamed answer, one SSE chunk per piece."""

    pieces: tuple[str, ...]


@dataclass(frozen=True)
class ToolCall:
    """A streamed call to one tool, its arguments split in two chunks."""

    name: str
    arguments: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Thought:
    """A streamed thought (`reasoning_content`, as DeepSeek and vLLM send it),
    one SSE chunk per piece, then the reply `then` in the same response."""

    thinking: tuple[str, ...]
    then: Text | ToolCall


@dataclass(frozen=True)
class Refusal:
    """An HTTP error before any stream, with an OpenAI-shaped body."""

    status: int
    body: dict
    headers: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Hold:
    """Accept the request and never answer."""


def _chunk(delta: dict, finish: str | None = None) -> str:
    choice = {"index": 0, "delta": delta, "finish_reason": finish}
    payload = {
        "id": "chatcmpl-sintetico",
        "object": "chat.completion.chunk",
        "created": 1_700_000_000,
        "model": "modelo-sintetico",
        "choices": [choice],
    }
    return f"data: {json.dumps(payload)}\n\n"


def stream(reply) -> list[str]:
    if isinstance(reply, Thought):
        thought = [
            _chunk({"role": "assistant", "reasoning_content": piece}) for piece in reply.thinking
        ]
        return thought + stream(reply.then)
    if isinstance(reply, Text):
        events = [_chunk({"role": "assistant", "content": ""})]
        events += [_chunk({"content": piece}) for piece in reply.pieces]
        events.append(_chunk({}, "stop"))
    else:
        arguments = json.dumps(reply.arguments)
        half = len(arguments) // 2
        call = {
            "index": 0,
            "id": "call-sintetico-1",
            "type": "function",
            "function": {"name": reply.name, "arguments": arguments[:half]},
        }
        events = [
            _chunk({"role": "assistant", "tool_calls": [call]}),
            _chunk({"tool_calls": [{"index": 0, "function": {"arguments": arguments[half:]}}]}),
            _chunk({}, "tool_calls"),
        ]
    return [*events, "data: [DONE]\n\n"]


class FakeProvider:
    def __init__(self, *replies, speech: bytes = b"ID3 voz sintetica"):
        self.app = FastAPI()
        self.replies = replies
        self.requests: list[dict] = []
        self.headers: list[dict] = []
        self.speech_requests: list[dict] = []

        @self.app.post("/v1/chat/completions")
        async def completions(request: Request):
            self.requests.append(await request.json())
            self.headers.append(dict(request.headers))
            reply = self.replies[min(len(self.requests), len(self.replies)) - 1]
            if isinstance(reply, Refusal):
                return JSONResponse(reply.body, status_code=reply.status, headers=reply.headers)
            if isinstance(reply, Hold):
                await asyncio.Future()

            async def events():
                for event in stream(reply):
                    yield event

            return StreamingResponse(events(), media_type="text/event-stream")

        @self.app.post("/v1/audio/speech")
        async def audio_speech(request: Request):
            self.speech_requests.append(await request.json())
            return Response(speech, media_type="audio/mpeg")


@contextmanager
def serve(app):
    """`app` on a loopback port of its own, from a thread; yields its origin."""
    bound = socket.socket()
    bound.bind(("127.0.0.1", 0))
    bound.listen(64)
    config = uvicorn.Config(
        app, log_level="critical", access_log=False, lifespan="off", timeout_graceful_shutdown=2
    )
    server = uvicorn.Server(config)
    thread = threading.Thread(
        target=server.run, kwargs={"sockets": [bound]}, name="orbit-test-peer", daemon=True
    )
    thread.start()
    deadline = time.monotonic() + START_TIMEOUT_SECONDS
    try:
        while not server.started:
            if not thread.is_alive() or time.monotonic() > deadline:
                raise RuntimeError(f"a synthetic peer did not start in {START_TIMEOUT_SECONDS} s")
            time.sleep(0.01)
        yield f"http://127.0.0.1:{bound.getsockname()[1]}"
    finally:
        server.should_exit = True
        thread.join(START_TIMEOUT_SECONDS)
        bound.close()


def closed_port() -> int:
    """A loopback port this process bound and closed: nothing listens on it."""
    bound = socket.socket()
    bound.bind(("127.0.0.1", 0))
    port = bound.getsockname()[1]
    bound.close()
    return port
