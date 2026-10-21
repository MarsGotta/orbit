"""`GET /health/deps`: whether the services the hub is configured to use answer.

For a monitor that cannot reach the hub's loopback services itself, such as
one running in a container network or on another machine. It is anonymous, so it
says nothing but fixed names: never a URL, a host, an error text or anything a
dependency answered. Why one failed goes to the hub's own log.

Probed, in parallel, each the cheapest real way:
- ``whisper``: the meeting STT (``[meeting_transcription]``), ``GET /health``,
  which must answer 2xx. The configured service must expose that route.
- ``hermes``: the Hermes serve the chat uses (``hermes_url``). No health route
  of Hermes is relied on: any HTTP answer below 500 to ``GET /`` counts, which
  proves the server is up and answering HTTP, not that a turn would succeed.
  Voice STT and TTS go through the same Hermes, so they are not a separate
  dependency.
- ``processing``: the notes model (``[processing]``), only when configured;
  any HTTP answer below 500 to ``GET`` on its endpoint. A 5xx is what a proxy
  in front of a stopped model answers. No credential is sent.
"""

import asyncio
import time
from collections.abc import Callable
from dataclasses import dataclass

import httpx
from fastapi.responses import JSONResponse

from ..log import logger

# Total bound of one probe, connection included. The dependencies answer a
# health request in milliseconds when they are up (unmeasured here: the live
# services are out of reach of development); three seconds ride out a pause
# without making the monitor wait long, and are far below a typical monitor's
# own request timeout (Uptime Kuma's is 48 s by default). It is not a measure of transcription or
# turn latency.
PROBE_TIMEOUT_SECONDS = 3.0
# How long one round of probes answers every caller. The route is anonymous,
# so this is what keeps it from being used to hammer Whisper: at most one probe
# per dependency every ten seconds, however often it is called. A monitor
# polls once a minute by default (20 s at the least), so it always sees a
# fresh round.
CACHE_SECONDS = 10.0


@dataclass(frozen=True)
class Probe:
    name: str
    url: str
    healthy: Callable[[int], bool]


def _success(status: int) -> bool:
    return 200 <= status < 300


def _answers(status: int) -> bool:
    return status < 500


def probes_for(settings) -> tuple[Probe, ...]:
    probes = [
        Probe("whisper", settings.meeting_transcription.endpoint.rstrip("/") + "/health", _success),
        Probe("hermes", settings.hermes_url.rstrip("/") + "/", _answers),
    ]
    if settings.processing is not None:
        probes.append(Probe("processing", settings.processing.endpoint, _answers))
    return tuple(probes)


class DependencyHealth:
    def __init__(self, probes: tuple[Probe, ...]):
        self.probes = probes
        self._lock = asyncio.Lock()
        self._failing: list[str] = []
        self._checked_at: float | None = None

    async def failing(self) -> list[str]:
        # Callers that arrive during a round wait for it and share its answer.
        async with self._lock:
            now = time.monotonic()
            if self._checked_at is None or now - self._checked_at >= CACHE_SECONDS:
                async with httpx.AsyncClient(
                    timeout=PROBE_TIMEOUT_SECONDS, follow_redirects=False
                ) as client:
                    answers = await asyncio.gather(
                        *(self._answers(client, probe) for probe in self.probes)
                    )
                self._failing = [probe.name for probe, ok in zip(self.probes, answers) if not ok]
                self._checked_at = time.monotonic()
            return list(self._failing)

    async def _answers(self, client: httpx.AsyncClient, probe: Probe) -> bool:
        status = None
        try:
            async with asyncio.timeout(PROBE_TIMEOUT_SECONDS):
                status = (await client.get(probe.url)).status_code
            if probe.healthy(status):
                return True
            cause = "status"
        except (TimeoutError, httpx.TimeoutException):
            cause = "timeout"
        except httpx.HTTPError:
            cause = "connection"
        logger.warning(
            "dependency_unavailable",
            extra={
                "event": "dependency_unavailable",
                "code": probe.name,
                "field": cause,
                "status": status,
            },
        )
        return False


def mount(app, settings):
    health = DependencyHealth(probes_for(settings))

    @app.get("/health/deps")
    async def dependencies():
        failing = await health.failing()
        if failing:
            return JSONResponse({"ok": False, "failing": failing}, status_code=503)
        return {"ok": True}
