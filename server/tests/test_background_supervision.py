"""The hub's own loops run supervised: one that raises is logged and
relaunched, and the others and the hub go on."""

import asyncio
from types import SimpleNamespace

from orbit_hub import background, lifespan, log

LOOPS = ("enforce_revocations", "expire_attachments", "watch_storage")


async def test_every_background_loop_that_raises_is_logged_and_relaunched(monkeypatch):
    monkeypatch.setattr(log, "RELAUNCH_BACKOFF_SECONDS", (0,))
    reported = []
    monkeypatch.setattr(log, "report", lambda *args, **fields: reported.append(fields["task"]))
    runs = {name: 0 for name in LOOPS}
    relaunched = {name: asyncio.Event() for name in LOOPS}

    def loop(name):
        async def run(*_args):
            runs[name] += 1
            if runs[name] == 1:
                raise RuntimeError("fallo sintético del bucle")
            relaunched[name].set()
            await asyncio.Event().wait()

        return run

    for name in LOOPS:
        monkeypatch.setattr(background, name, loop(name))
    hub = SimpleNamespace(store=None, runtimes=None, store_executor=None, now=lambda: 0.0)
    tasks = lifespan.start_loops(hub)
    try:
        await asyncio.wait_for(
            asyncio.gather(*(event.wait() for event in relaunched.values())), timeout=5
        )
        assert runs == {name: 2 for name in LOOPS}
        assert sorted(reported) == sorted(LOOPS)
        assert not any(task.done() for task in tasks), "a raising loop ended its supervisor"
    finally:
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
