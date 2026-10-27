"""Hub-owned per-root snapshots; no tree walks on warm assistant admission."""

import asyncio
from pathlib import Path
from threading import Event

from ..log import report
from . import jail_secrets

# The measured 592,505-entry tree took 3.7–6.9 seconds to walk. Refresh every
# ten minutes to pick up new secrets without imposing that I/O on each start.
# This is discovery freshness, not the lifetime of a running jail's mounts.
SECRET_SCAN_REFRESH_SECONDS = 10 * 60
# More than four times the worst observed 6.9 s walk, allowing startup load.
# Bounds only the first admission's wait; a timeout never cancels discovery.
INITIAL_SCAN_WAIT_SECONDS = 30


class SecretScanUnavailable(Exception):
    pass


async def refresh_delay() -> None:
    await asyncio.sleep(SECRET_SCAN_REFRESH_SECONDS)


class SecretCache:
    def __init__(self, roots: tuple[Path, ...], data_dir: Path):
        self.roots = roots
        self.omitted = (data_dir.resolve(), Path("/tmp"))
        self.results: dict[Path, tuple[Path, ...]] = {}
        self.failed: set[Path] = set()
        self.attempted: set[Path] = set()
        self.tasks: list[asyncio.Task] = []
        self.ready = asyncio.Event()
        self.stop = Event()

    def start(self) -> None:
        if self.tasks or not self.roots:
            return
        self.ready = asyncio.Event()
        self.stop = Event()
        self.results.clear()
        self.failed.clear()
        self.attempted.clear()
        self.tasks = [asyncio.create_task(self._watch(root)) for root in self.roots]

    async def wait_ready(self) -> None:
        if not self.roots:
            return
        self.start()
        if not self.ready.is_set():
            try:
                await asyncio.wait_for(self.ready.wait(), INITIAL_SCAN_WAIT_SECONDS)
            except TimeoutError:
                raise SecretScanUnavailable("secret_scan_timeout") from None
        self.paths()

    def paths(self) -> tuple[Path, ...]:
        if self.failed:
            raise SecretScanUnavailable("secret_scan_failed")
        if any(root not in self.results for root in self.roots):
            raise SecretScanUnavailable("secret_scan_not_ready")
        return tuple(sorted({path for paths in self.results.values() for path in paths}))

    async def close(self) -> None:
        self.stop.set()
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        self.tasks.clear()

    async def _watch(self, root: Path) -> None:
        while True:
            scanning = asyncio.create_task(
                asyncio.to_thread(jail_secrets.scan_root, root, self.omitted, self.stop)
            )
            try:
                paths = await asyncio.shield(scanning)
            except asyncio.CancelledError:
                await scanning
                raise
            except Exception:  # noqa: BLE001 - retain the last snapshot; retry next cycle
                if root not in self.results:
                    self.failed.add(root)
                report("assistant_secret_scan_failed", "secret_scan_failed")
            else:
                self.results[root] = paths
                self.failed.discard(root)
            self.attempted.add(root)
            if len(self.attempted) == len(self.roots):
                self.ready.set()
            await refresh_delay()
