"""The suite's detector of blocking calls on the event loop (`support_blocking`)."""

import asyncio
import time
from pathlib import Path

import pytest
from conftest import BLOCKING
from support_blocking import BlockingCallInLoop

import orbit_hub

# A function compiled as if it were the hub's own code: no module of the hub
# blocks the loop, so the test makes one that would.
HUB_FILE = str(Path(orbit_hub.__file__).parent / "invented_blocking_module.py")


def hub_function():
    namespace = {"time": time}
    exec(compile("def tick():\n    time.sleep(0)\n", HUB_FILE, "exec"), namespace)  # noqa: S102
    return namespace["tick"]


async def test_the_hub_sleeping_on_the_loop_is_refused_and_named():
    tick = hub_function()
    mark = BLOCKING.mark()
    with pytest.raises(BlockingCallInLoop) as refused:
        tick()
    assert str(refused.value).startswith(
        f"Detected blocking call to time.sleep inside the event loop by {HUB_FILE}:2"
    )
    # A test that swallowed it would still fail; this one takes it back.
    [detection] = BLOCKING.since(mark)
    assert detection.call == "time.sleep" and detection.culprit == f"{HUB_FILE}:2"
    del BLOCKING.detections[mark:]


async def test_the_hub_sleeping_off_the_loop_passes():
    tick = hub_function()
    mark = BLOCKING.mark()
    await asyncio.to_thread(tick)
    assert BLOCKING.since(mark) == []


def test_the_hub_sleeping_with_no_loop_running_passes():
    tick = hub_function()
    mark = BLOCKING.mark()
    tick()
    assert BLOCKING.since(mark) == []


async def test_a_test_harness_waiting_on_the_loop_is_not_the_hub():
    mark = BLOCKING.mark()
    time.sleep(0)  # noqa: ASYNC251 - the harness's own wait, which the detector lets pass
    assert BLOCKING.since(mark) == []
