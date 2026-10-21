"""Structured JSON logging, the `op` of each request, and the one function that reports failures.

The schema is fixed: ``event``, ``op``, ``device``, ``session``, ``code``,
``route``, ``status``, ``numbers``, ``task``, ``field`` and ``elapsed_ms``
appear when set. Anything else on a record is dropped, so a caller cannot
smuggle message text, audio bytes, transcripts, tokens or secret paths into a
log line through an ``extra=`` keyword. ``report`` is the only way the rest of
the package records a failure: it carries the exception class name but never
the exception message itself, and it is what forwards the failure to Sentry
when the operator configured ``[error_reports]``.
"""

from __future__ import annotations

import asyncio
import json
import logging
import re
import sys
from collections.abc import Awaitable, Callable
from contextvars import ContextVar, copy_context
from functools import partial
from typing import Any
from uuid import uuid4

from . import error_reports

SCHEMA: tuple[str, ...] = (
    "event",
    "op",
    "device",
    "session",
    "code",
    "route",
    "status",
    "numbers",
    "task",
    "field",
    "elapsed_ms",
)

# The identifier of one action from end to end: the Mac sends it, every log
# line of the request carries it, every error body returns it, and the generic
# failure the user sees shows its first six characters. The shape is the
# contract's; it bounds nothing else.
OP_PATTERN = re.compile(r"[A-Za-z0-9-]{8,64}")
_OP: ContextVar[str | None] = ContextVar("orbit_op", default=None)


def new_op() -> str:
    return str(uuid4())


def bind_op(candidate: object) -> str:
    """Make ``candidate`` the op of the current request, or a new one if it is not valid."""
    valid = isinstance(candidate, str) and OP_PATTERN.fullmatch(candidate)
    op = candidate if valid else new_op()
    _OP.set(op)
    return op


def current_op() -> str | None:
    return _OP.get()


def carrying_op(function: Callable, *args, **kwargs) -> Callable:
    """``function(*args, **kwargs)`` for another thread, with this request's op.

    ``run_in_executor`` does not carry context variables, so without this a
    line logged in the store thread would not say which request it belongs to.
    """
    return partial(copy_context().run, function, *args, **kwargs)


def clear_op() -> None:
    """For a long-lived task started inside a request: its failures are not that request's."""
    _OP.set(None)


class JsonFormatter(logging.Formatter):
    """One JSON object per line; fixed schema; no embedded exception text."""

    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": self.formatTime(record, "%Y-%m-%dT%H:%M:%S"),
            "level": record.levelname,
        }
        for field in SCHEMA:
            value = getattr(record, field, None)
            if field == "op" and value is None:
                value = _OP.get()
            if value is None:
                continue
            payload[field] = value
        if record.exc_info and record.exc_info[0] is not None:
            payload["exc_type"] = record.exc_info[0].__name__
        return json.dumps(payload, ensure_ascii=False, default=str, sort_keys=True)


_configured = False


def configure() -> None:
    """Attach the JSON formatter to the ``orbit_hub`` package logger. Idempotent."""
    global _configured
    if _configured:
        return
    handler = logging.StreamHandler(stream=sys.stderr)
    handler.setFormatter(JsonFormatter())
    package = logging.getLogger("orbit_hub")
    for existing in list(package.handlers):
        package.removeHandler(existing)
    package.addHandler(handler)
    package.propagate = False
    package.setLevel(logging.INFO)
    _configured = True


def get_logger(name: str) -> logging.Logger:
    configure()
    return logging.getLogger(name)


def report(
    event: str,
    code: str,
    *,
    device: str | None = None,
    session: str | None = None,
    elapsed_ms: int | None = None,
    field: str | None = None,
    route: str | None = None,
    status: int | None = None,
    numbers: dict[str, object] | None = None,
    task: str | None = None,
    exc: BaseException | None = None,
    exc_info: bool = True,
) -> None:
    """Record one failure: a log line, and Sentry when configured. Never content.

    ``exc_info`` takes the exception being handled; ``exc`` names one that is
    not, such as the failure of a task read from its done callback. Only
    numbers survive in ``numbers``. A failure outside any request gets an op
    of its own, so its line and its Sentry event can be found from each other.
    """
    op = _OP.get() or new_op()
    figures = {key: value for key, value in (numbers or {}).items() if type(value) in (int, float)}
    extra: dict[str, Any] = {"event": event, "code": code, "op": op}
    for key, value in (
        ("device", device),
        ("session", session),
        ("elapsed_ms", elapsed_ms),
        ("field", field),
        ("route", route),
        ("status", status),
        ("numbers", figures or None),
        ("task", task),
    ):
        if value is not None:
            extra[key] = value
    if exc is None and exc_info:
        exc = sys.exc_info()[1]
    failure = (type(exc), exc, exc.__traceback__) if exc is not None else None
    logger.error(event, exc_info=failure, extra=extra)
    error_reports.capture(code=code, op=op, route=route, numbers=figures, exception=exc)


def stop_if_asked() -> None:
    """Honour a cancel a call swallowed: stop at the top of a worker's loop.

    A call can finish its work under a cancel instead of raising it (httpcore
    closes a response inside an anyio shield). The request to stop is still
    pending on the task (`cancelling()`), and each later `wait_for` turns its
    own timeout into the `TimeoutError` the loop ignores, so a worker would
    never stop and the hub would never finish closing.
    """
    task = asyncio.current_task()
    if task is not None and task.cancelling():
        raise asyncio.CancelledError


# Seconds before each relaunch of a crashed worker: quick after a passing
# failure, never a busy loop on a lasting one. The last one is the cap: every
# relaunch after it waits that long. It paces relaunches; it bounds nothing else.
RELAUNCH_BACKOFF_SECONDS = (1, 2, 4, 8, 16, 30)


async def supervised(name: str, factory: Callable[[], Awaitable[None]]) -> None:
    """Loop ``factory``; on crash, report ``task_crashed`` and relaunch with backoff.

    A normal return ends the loop. ``CancelledError`` propagates so that shutdown
    remains a single cancellation signal.
    """
    backoff = RELAUNCH_BACKOFF_SECONDS
    attempt = 0
    while True:
        try:
            await factory()
            return
        except asyncio.CancelledError:
            raise
        except Exception:  # noqa: BLE001 - reported, then relaunched
            delay = backoff[min(attempt, len(backoff) - 1)]
            attempt += 1
            report("task_crashed", "task_crashed", task=name)
            await asyncio.sleep(delay)


logger = get_logger(__name__)


async def in_store(executor, function: Callable, *args, **kwargs):
    """Run a store operation off the loop, carrying its correlation context."""
    return await asyncio.get_running_loop().run_in_executor(
        executor, carrying_op(function, *args, **kwargs)
    )
