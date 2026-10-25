"""Each agent's backend as the hub runs it: an isolated integration.

After Home Assistant's integrations: every start of an agent's backend (its
connection opening, a session opening or reopening) runs bounded, and the
last failure of a start (a connection or a new session) is visible in the
agents list (`backend`, contracts/hub-v1.md) until a start succeeds again. A
reopen's refusal is the conversation's (the agent no longer keeps it, it
cannot read its history), never the backend's. A backend that fails or hangs
only refuses the turn that asked for it: other agents, other conversations
and the hub go on.
"""

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TypeVar

from ..errors import HubError
from ..log import report

T = TypeVar("T")

# The most one start of a backend may take. What it protects: the turn and the
# conversation that wait for it, and the Mac that waits for their answer,
# from a backend that never answers its start. Why 300 s: it is Home
# Assistant's cut for an integration's setup, and it is above every bound the
# backends keep themselves (an official vendor agent: 30 s to start its process and 120 s
# to open or reload a session; orbit-mcp for an `openai` agent: 30 s; Hermes:
# 25 s to connect and 30 s for an RPC), so it never cuts a start they would
# finish. It does not bound a turn, a history load (`RESUME_HISTORY_TIMEOUT_
# SECONDS`) or another agent.
START_CEILING_SECONDS = 300.0
# From here on a start still running is logged as slow, once, with no content.
# Why 10 s: Home Assistant's warning for a slow setup; every backend's start
# measured in the hub's tests takes milliseconds.
START_WARNING_SECONDS = 10.0


@dataclass(frozen=True)
class StartFailure:
    # The refusal the start ended with (`agent_start_failed`,
    # `agent_start_timeout`, `upstream_unavailable`…) or `internal_error`.
    code: str
    at_ms: int


class Integration:
    def __init__(self):
        self.failure: StartFailure | None = None
        # Called with no argument when `failure` changes; the agents registry
        # sets it, so every Mac learns it (`agents.changed`).
        self.changed: Callable[[], None] = lambda: None

    def public(self) -> dict | None:
        """What the agents list says of the backend: only a failure, while it lasts."""
        if self.failure is None:
            return None
        return {"state": "failed", "code": self.failure.code, "at_ms": self.failure.at_ms}

    async def start(
        self,
        agent_id: str,
        name: str,
        step: Callable[[], Awaitable[T]],
        now_ms: int,
        *,
        reopen: bool = False,
    ) -> T:
        """Run one start of the backend, bounded by `START_CEILING_SECONDS`.
        `reopen`: a session reopening, whose refusals are not the backend's."""
        slow = asyncio.get_running_loop().call_later(
            START_WARNING_SECONDS,
            lambda: report(
                "agent_start_slow", "agent_start_slow", task=f"agent:{agent_id}", exc_info=False
            ),
        )
        try:
            async with asyncio.timeout(START_CEILING_SECONDS) as bound:
                result = await step()
        except TimeoutError as exc:
            if not bound.expired():
                self._failed(agent_id, "internal_error", now_ms, exc_info=True)
                raise
            self._failed(agent_id, "agent_start_timeout", now_ms)
            milliseconds = round(START_CEILING_SECONDS * 1000)
            raise HubError(
                "agent_start_timeout",
                f"{name} no terminó de arrancar en {milliseconds} ms; no se envió el mensaje",
                numbers={"agent_id": agent_id, "timeout_ms": milliseconds},
            ) from exc
        except HubError as exc:
            if not reopen:
                self._failed(agent_id, exc.code, now_ms)
            raise
        except Exception:
            self._failed(agent_id, "internal_error", now_ms, exc_info=True)
            raise
        finally:
            slow.cancel()
        self._started()
        return result

    def _failed(self, agent_id: str, code: str, now_ms: int, *, exc_info=False) -> None:
        report("agent_start_failed", code, task=f"agent:{agent_id}", exc_info=exc_info)
        self.failure = StartFailure(code, now_ms)
        self.changed()

    def _started(self) -> None:
        if self.failure is not None:
            self.failure = None
            self.changed()
