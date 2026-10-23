"""The hub's own loops, which no request starts: each runs supervised."""

import asyncio

from . import attachments
from .protocol import emit

# How often the hub looks at its storage to tell the connected Macs when it
# becomes nearly full, or stops being (`storage.status`). What it protects:
# the Mac's warning, which `ready` alone would only refresh on a reconnection.
# Why a minute: a look is three `stat` calls and one `statvfs`, and the last
# fifth of the limit takes days to fill, not minutes. It limits nothing.
STORAGE_CHECK_SECONDS = 60.0


async def enforce_revocations(runtimes, store) -> None:
    # A viewer can disconnect while its upstream turn remains active. Revocation
    # therefore belongs to the server lifespan, never to an individual socket.
    while True:
        revoked = [
            (device, runtime)
            for device, runtime in list(runtimes.items())
            if not store.devices.is_active(device)
        ]
        if revoked:
            await asyncio.gather(*(runtime.revoke() for _, runtime in revoked))
            for device, runtime in revoked:
                if runtimes.get(device) is runtime:
                    runtimes.pop(device)
        await asyncio.sleep(1)


async def expire_attachments(store, store_executor, now) -> None:
    # Files nobody sent, every device's: the Mac that uploaded one may
    # never upload again (`attachments.UNBOUND_SWEEP_SECONDS`).
    loop = asyncio.get_running_loop()
    while True:
        await loop.run_in_executor(store_executor, store.attachments.expire, int(now() * 1000))
        await asyncio.sleep(attachments.UNBOUND_SWEEP_SECONDS)


async def watch_storage(store, runtimes) -> None:
    # Said only when the level changes: every `ready` carries it anyway.
    loop = asyncio.get_running_loop()
    last = None
    while True:
        status = await loop.run_in_executor(None, store.space.status)
        if last is not None and status["nearly_full"] != last:
            runtimes.offer_all(emit("storage.status", status))
        last = status["nearly_full"]
        await asyncio.sleep(STORAGE_CHECK_SECONDS)
