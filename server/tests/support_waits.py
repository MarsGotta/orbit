"""Hang-guarded waits shared by hub tests: a hung test is never a result.

Every wait in these suites resolves on an event the product produces; the
budget below only bounds how long that event may lag on a loaded host before
the test must fail naming the elapsed time. It is the same ceiling
``support_client_e2e`` gives its condition waits, and it is never an oracle:
no assertion passes or softens because the budget expired.
"""

import asyncio
import time

from support_client_e2e import DAEMON_E2E_TIMEOUT_SECONDS

__all__ = ("DAEMON_E2E_TIMEOUT_SECONDS", "wait_for", "wait_thread")


async def wait_for(awaitable, what: str, budget: float = DAEMON_E2E_TIMEOUT_SECONDS):
    """Await an event-like coroutine, failing with the elapsed time if it jams."""
    started = time.monotonic()
    try:
        return await asyncio.wait_for(awaitable, budget)
    except TimeoutError as exc:
        raise AssertionError(
            f"{what} not observed within {time.monotonic() - started:.1f}s (hang guard)"
        ) from exc


def wait_thread(event, what: str, budget: float = DAEMON_E2E_TIMEOUT_SECONDS) -> None:
    """Thread-side mirror of ``wait_for`` for ``threading.Event`` barriers."""
    if not event.wait(budget):
        raise AssertionError(f"{what} not observed within {budget:.1f}s (hang guard)")
