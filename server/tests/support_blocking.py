"""A detector of blocking calls made on a running event loop, strict in tests.

After Home Assistant's `block_async_io`: the calls that put a thread to sleep
or wait for a child process are wrapped, and one that the hub's or orbit-mcp's
code makes from a thread running an event loop is refused with
`BlockingCallInLoop`, naming the call and the innermost frame of that code.
Every detection is also recorded, so a test that swallows the error still
fails (`conftest`). A test's own harness (a helper that waits for a synthetic
peer to start) is not the hub and passes.

It is deterministic: it reads the thread, never a clock. It does not cover
SQLite, on purpose: the hub runs its small transactions on the loop by design
(`app.py`, the store's executor takes the heavy ones). Calls from a worker
thread (`asyncio.to_thread`, an executor) are what the loop should do, and
pass.
"""

import asyncio
import subprocess
import sys
import time
from dataclasses import dataclass
from functools import wraps
from pathlib import Path

# The calls watched, as (module, name). Each one blocks its thread until
# something outside the process answers or a time passes.
WATCHED = (
    (time, "sleep"),
    (subprocess, "run"),
    (subprocess, "call"),
    (subprocess, "check_call"),
    (subprocess, "check_output"),
)
_OWN_CODE = tuple(
    str(Path(__file__).resolve().parent.parent / package) for package in ("orbit_hub", "orbit_mcp")
)


class BlockingCallInLoop(RuntimeError):
    pass


@dataclass(frozen=True)
class Detection:
    call: str
    culprit: str


class Detector:
    def __init__(self):
        self.detections: list[Detection] = []
        self.originals: list[tuple[object, str, object]] = []

    def install(self) -> None:
        for module, name in WATCHED:
            original = getattr(module, name)
            self.originals.append((module, name, original))
            setattr(module, name, self._guarded(f"{module.__name__}.{name}", original))

    def uninstall(self) -> None:
        for module, name, original in reversed(self.originals):
            setattr(module, name, original)
        self.originals.clear()

    def mark(self) -> int:
        return len(self.detections)

    def since(self, mark: int) -> list[Detection]:
        return self.detections[mark:]

    def _guarded(self, call: str, original):
        @wraps(original)
        def guarded(*arguments, **keywords):
            culprit = _culprit() if _in_running_loop() else None
            if culprit is not None:
                detection = Detection(call, culprit)
                self.detections.append(detection)
                raise BlockingCallInLoop(
                    f"Detected blocking call to {call} inside the event loop by "
                    f"{detection.culprit}; run it with asyncio.to_thread or an executor"
                )
            return original(*arguments, **keywords)

        return guarded


def _in_running_loop() -> bool:
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return False
    return True


def _culprit() -> str | None:
    """The innermost frame of the hub's or orbit-mcp's code, if any made the call."""
    frame = sys._getframe(2)
    while frame is not None:
        if frame.f_code.co_filename.startswith(_OWN_CODE):
            return f"{frame.f_code.co_filename}:{frame.f_lineno}"
        frame = frame.f_back
    return None
