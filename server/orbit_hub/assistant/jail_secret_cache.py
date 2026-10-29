"""Hub-owned per-root snapshots; no tree walks on warm assistant admission."""

import asyncio
import json
import os
import tempfile
import time
from pathlib import Path
from threading import Event

from ..log import report
from ..secret_file import read_secret
from . import jail_secrets

# The measured 592,505-entry tree took 3.7–6.9 seconds to walk. Refresh every
# ten minutes to pick up new secrets without imposing that I/O on each start.
# This is discovery freshness, not the lifetime of a running jail's mounts.
SECRET_SCAN_REFRESH_SECONDS = 10 * 60
# A reported cold hub restart took 63 s. Five synthetic 600,200-entry walks
# took 2.365–2.446 s here, but page caches could not be dropped. Use twice the
# observed cold duration, rather than extrapolating from this warm filesystem.
# Bounds only the first admission's wait; a timeout never cancels discovery.
INITIAL_SCAN_WAIT_SECONDS = 126
# Bounds startup snapshot parsing, not discovery or masking. 16 MiB leaves
# hundreds of times the space needed for the measured 200-path snapshot.
# Larger snapshots fall back to discovery, without reducing its budgets.
MAX_SECRET_SNAPSHOT_BYTES = 16 * 1024 * 1024


def load_snapshot(path: Path, roots: tuple[Path, ...], omitted: tuple[Path, ...]):
    try:
        saved = json.loads(read_secret(path, MAX_SECRET_SNAPSHOT_BYTES))
        if not isinstance(saved, dict) or saved.get("omitted") != list(map(str, omitted)):
            return {}
        entries = saved["roots"]
        now = time.time()
        results = {}
        for root in roots:
            entry = entries.get(str(root))
            if entry is None:
                continue
            stamp, paths = entry["scanned_at"], entry["paths"]
            if not isinstance(stamp, (int, float)) or isinstance(stamp, bool):
                continue
            if not 0 <= now - stamp < SECRET_SCAN_REFRESH_SECONDS:
                continue
            if not isinstance(paths, list) or not all(
                isinstance(value, str) and Path(value).is_absolute() and "\0" not in value
                for value in paths
            ):
                continue
            results[root] = (stamp, tuple(Path(value) for value in paths))
        return results
    except (OSError, ValueError, TypeError, KeyError, AttributeError):
        # Missing, old or unusable cache: admission waits for discovery.
        return {}


def save_snapshot(path: Path, snapshot: dict) -> None:
    payload = json.dumps(snapshot).encode()
    if len(payload) > MAX_SECRET_SNAPSHOT_BYTES:
        raise ValueError("Secret scan snapshot exceeds its startup read budget")
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as handle:
        temporary = Path(handle.name)
        try:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
            os.replace(temporary, path)
            directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            temporary.unlink(missing_ok=True)


class SecretScanUnavailable(Exception):
    pass


async def refresh_delay() -> None:
    await asyncio.sleep(SECRET_SCAN_REFRESH_SECONDS)


class SecretCache:
    def __init__(self, roots: tuple[Path, ...], data_dir: Path):
        self.roots = roots
        self.omitted = (data_dir.resolve(), Path("/tmp"))
        self.snapshot_path = data_dir / "secret-scan.json"
        self.results: dict[Path, tuple[Path, ...]] = {}
        self.scanned_at: dict[Path, float] = {}
        self.saving = asyncio.Lock()
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
        self.scanned_at.clear()
        self.failed.clear()
        self.attempted.clear()
        self.tasks = [asyncio.create_task(self._run())]

    async def _run(self) -> None:
        saved = await asyncio.to_thread(load_snapshot, self.snapshot_path, self.roots, self.omitted)
        for root, (stamp, paths) in saved.items():
            self.results[root] = paths
            self.scanned_at[root] = stamp
        if len(self.results) == len(self.roots):
            self.ready.set()
        await asyncio.gather(*(self._watch(root) for root in self.roots))

    async def _persist(self) -> None:
        # Serialize writers so parallel roots cannot overwrite a newer snapshot.
        async with self.saving:
            snapshot = {
                "omitted": list(map(str, self.omitted)),
                "roots": {
                    str(root): {"scanned_at": self.scanned_at[root], "paths": list(map(str, paths))}
                    for root, paths in self.results.items()
                },
            }
            saving = asyncio.create_task(
                asyncio.to_thread(save_snapshot, self.snapshot_path, snapshot)
            )
            try:
                await asyncio.shield(saving)
            except asyncio.CancelledError:
                await saving
                raise
            except (OSError, ValueError):
                # Discovery succeeded: a disk failure must not discard its masks.
                report("assistant_secret_snapshot_failed", "secret_snapshot_write_failed")

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
                self.scanned_at[root] = time.time()
                self.failed.discard(root)
                await self._persist()
            self.attempted.add(root)
            if len(self.attempted) == len(self.roots):
                self.ready.set()
            await refresh_delay()
