"""`GET /health/deps` for an outside monitor, against fake dependencies on loopback.

Every dependency here is a server this process bound (support_client_e2e.tcp_server):
none of the machine's real services is ever asked.
"""

import asyncio
import json
import socket
import time
from contextlib import AsyncExitStack, asynccontextmanager
from dataclasses import replace

import httpx
from fastapi import FastAPI
from fastapi.responses import JSONResponse
from support_client_e2e import tcp_server

from orbit_hub.app import create_app
from orbit_hub.meeting_transcription import MeetingTranscriptionConfiguration
from orbit_hub.processing.config import ProcessingConfiguration
from orbit_hub.routes.health import PROBE_TIMEOUT_SECONDS


class Dependency:
    """A fake service: answers `status`, or holds every request until released."""

    def __init__(self, status=200, *, hold=False):
        self.status, self.hold = status, hold
        self.requests = 0
        self.released = asyncio.Event()
        self.app = FastAPI()

        @self.app.get("/{path:path}")
        async def answer(path: str):
            self.requests += 1
            if self.hold:
                await self.released.wait()
            return JSONResponse({"detalle": "texto interno del servicio"}, status_code=self.status)


def closed_port_url() -> str:
    """A loopback port this process bound and released: connecting is refused."""
    with socket.socket() as bound:
        bound.bind(("127.0.0.1", 0))
        port = bound.getsockname()[1]
    return f"http://127.0.0.1:{port}"


@asynccontextmanager
async def hub(environment, *, whisper, hermes, processing=None, key=None):
    base, store, adapter = environment
    settings = replace(
        base,
        hermes_url=hermes,
        allow_insecure_localhost=True,
        meeting_transcription=MeetingTranscriptionConfiguration(endpoint=whisper, model="m"),
        processing=(
            ProcessingConfiguration(
                endpoint=processing + "/v1/chat/completions",
                model="m",
                credential_file=key,
                allow_insecure_localhost=True,
            )
            if processing is not None
            else None
        ),
    )
    app = create_app(settings, store=store, adapter=adapter, now=lambda: 1001)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost"
    ) as client:
        yield client


async def served(stack, dependency):
    return await stack.enter_async_context(tcp_server(dependency.app))


async def test_all_dependencies_answer(environment, tmp_path):
    key = tmp_path / "processing.key"
    key.write_text("clave-sintetica")
    key.chmod(0o600)
    whisper, hermes, processing = Dependency(200), Dependency(404), Dependency(405)
    async with AsyncExitStack() as stack:
        urls = [await served(stack, item) for item in (whisper, hermes, processing)]
        async with hub(
            environment, whisper=urls[0], hermes=urls[1], processing=urls[2], key=key
        ) as http:
            response = await http.get("/health/deps")
    assert response.status_code == 200
    assert response.json() == {"ok": True}
    assert (whisper.requests, hermes.requests, processing.requests) == (1, 1, 1)


async def test_one_dependency_down_is_named_and_nothing_else_is_said(environment):
    whisper = Dependency(200)
    async with AsyncExitStack() as stack:
        whisper_url = await served(stack, whisper)
        hermes_url = closed_port_url()
        async with hub(environment, whisper=whisper_url, hermes=hermes_url) as http:
            response = await http.get("/health/deps", headers={"X-Orbit-Op": "op-health-0001"})
    assert response.status_code == 503
    assert response.json() == {"ok": False, "failing": ["hermes"]}
    for private in (hermes_url, hermes_url.rsplit(":", 1)[1], "127.0.0.1", "op-health-0001"):
        assert private not in response.text


async def test_whisper_that_answers_an_error_is_failing(environment):
    whisper, hermes = Dependency(503), Dependency(404)
    async with AsyncExitStack() as stack:
        urls = [await served(stack, item) for item in (whisper, hermes)]
        async with hub(environment, whisper=urls[0], hermes=urls[1]) as http:
            response = await http.get("/health/deps")
    assert response.status_code == 503
    assert response.json() == {"ok": False, "failing": ["whisper"]}
    assert "texto interno" not in response.text


async def test_a_slow_dependency_is_bounded_by_the_probe_timeout(environment):
    whisper, hermes = Dependency(200, hold=True), Dependency(404)
    async with AsyncExitStack() as stack:
        urls = [await served(stack, item) for item in (whisper, hermes)]
        stack.callback(whisper.released.set)
        async with hub(environment, whisper=urls[0], hermes=urls[1]) as http:
            started = time.monotonic()
            response = await http.get("/health/deps")
            elapsed = time.monotonic() - started
    assert response.json() == {"ok": False, "failing": ["whisper"]}
    # The bound itself, plus generous room for a loaded machine.
    assert PROBE_TIMEOUT_SECONDS <= elapsed < PROBE_TIMEOUT_SECONDS + 5


async def test_an_unconfigured_dependency_is_skipped(environment):
    whisper, hermes = Dependency(200), Dependency(404)
    async with AsyncExitStack() as stack:
        urls = [await served(stack, item) for item in (whisper, hermes)]
        async with hub(environment, whisper=urls[0], hermes=urls[1], processing=None) as http:
            response = await http.get("/health/deps")
    assert response.status_code == 200
    assert response.json() == {"ok": True}


async def test_repeated_calls_do_not_hammer_the_dependencies(environment):
    whisper, hermes = Dependency(200), Dependency(404)
    async with AsyncExitStack() as stack:
        urls = [await served(stack, item) for item in (whisper, hermes)]
        async with hub(environment, whisper=urls[0], hermes=urls[1]) as http:
            answers = await asyncio.gather(*(http.get("/health/deps") for _ in range(5)))
            answers.append(await http.get("/health/deps"))
    assert {json.dumps(answer.json()) for answer in answers} == {'{"ok": true}'}
    assert (whisper.requests, hermes.requests) == (1, 1)


async def test_health_is_unchanged(environment):
    async with hub(environment, whisper=closed_port_url(), hermes=closed_port_url()) as http:
        response = await http.get("/health")
    assert response.status_code == 200
    assert response.json() == {"ok": True, "protocol": 1}
