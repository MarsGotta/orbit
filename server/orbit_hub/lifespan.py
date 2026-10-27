"""The hub's start and stop: its lock, its workers and its loops, in order."""

import asyncio
import fcntl
import os
from contextlib import asynccontextmanager

from . import background, error_reports
from .composition import Hub
from .log import supervised


def lifespan(hub: Hub, owned_store: bool, release: str):
    @asynccontextmanager
    async def running(app):
        settings = hub.settings
        if settings.error_reports is not None:
            error_reports.start(settings.error_reports, release=release)
        descriptor = os.open(
            settings.data_dir / "hub.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600
        )
        loops = []
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            hub.store.recover_interrupted()
            settings.assistant_jail_cache.start()
            hub.device_bridge.start()
            hub.meetings_bridge.start()
            hub.transcriptions.start()
            hub.final_passes.start()
            hub.notes.start()
            loops.extend(start_loops(hub))
            yield
        finally:
            physical = asyncio.create_task(shutdown(hub, owned_store, descriptor, loops))
            cancelled = False
            while not physical.done():
                try:
                    await asyncio.shield(physical)
                except asyncio.CancelledError:
                    cancelled = True
            # Retrieve the actual shutdown outcome first. An internal failure
            # must not be mistaken for cancellation of this external waiter.
            physical.result()
            if cancelled:
                raise asyncio.CancelledError

    return running


def start_loops(hub: Hub) -> list[asyncio.Task]:
    store, runtimes = hub.store, hub.runtimes
    return [
        asyncio.create_task(
            supervised(
                "enforce_revocations", lambda: background.enforce_revocations(runtimes, store)
            )
        ),
        asyncio.create_task(
            supervised(
                "expire_attachments",
                lambda: background.expire_attachments(store, hub.store_executor, hub.now),
            )
        ),
        asyncio.create_task(
            supervised("watch_storage", lambda: background.watch_storage(store, runtimes))
        ),
    ]


async def shutdown(hub: Hub, owned_store: bool, descriptor: int, loops) -> None:
    # This task, including Store.close below, has a single owner and is
    # never cancelled by a lifespan waiter.
    try:
        for task in loops:
            task.cancel()
        await asyncio.gather(*loops, return_exceptions=True)
        await hub.final_passes.close()
        await hub.transcriptions.close()
        await hub.notes.close()
        await hub.sign_ins.close()
        await hub.runtimes.close()
    finally:
        await hub.settings.assistant_jail_cache.close()
        try:
            hub.device_bridge.close()
            hub.meetings_bridge.close()
        finally:
            try:
                hub.store_executor.shutdown(wait=False)
                os.close(descriptor)
                if owned_store:
                    hub.store.close()
            finally:
                if hub.settings.error_reports is not None:
                    error_reports.stop()
