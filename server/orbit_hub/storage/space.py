"""What the hub's database uses, the limit it may grow to, and admission.

Every write that brings content is admitted here first (`ensure_capacity`,
called inside its transaction through `Store.ensure_capacity`); a refusal
names the rule that fired with its numbers, and nothing is ever deleted to
stay under the limit.
"""

import errno
import shutil
import sqlite3
from collections.abc import Callable
from pathlib import Path

from ..protocol import HubError

# What the hub's database may grow to when no `storage_quota_bytes` is set:
# half of what it and the disk's free space add up to, once this floor is
# left free. The other half is room for one whole copy of the database beside
# it: `scripts/hub/activate.sh` takes a consistent copy on the server before
# every activation, and a database grown into that room could never be copied
# again. The floor protects the rest of the server, which keeps writing after
# the hub stops admitting audio: a full root filesystem stops journald,
# package updates and logins. Why 5 GiB: journald alone keeps up to 4 GiB by
# default (`SystemMaxUse`, 10 % of the filesystem capped at 4 GiB), and 1 GiB
# more covers a package update. What it is not: a retention rule. Nothing is
# ever deleted to stay under it; the hub refuses new content with its numbers
# and the Mac keeps its copy.
STORAGE_FREE_FLOOR_BYTES = 5 * 1024**3
# From this share of the limit on, the hub tells the Macs that it is nearly
# full (`ready.storage`, `storage.status`). What it protects: time to act
# before meetings stop reaching the hub. Why 80 %: Mars's choice (2026-10-22);
# at the production pace measured that day (the database grew about 0.8 GB in
# a day) the last fifth of an 80 GB limit is about three weeks.
STORAGE_WARNING_PERCENT = 80


def is_storage_full(failure: BaseException | None) -> bool:
    """Whether a failure, or one it was raised from, is the disk being full.

    SQLite says so with `SQLITE_FULL` ("database or disk is full"), which a
    failure re-raised from Python code may carry only as its text; the file
    system with ENOSPC or EDQUOT.
    """
    seen = 0
    while failure is not None and seen < 8:
        if isinstance(failure, sqlite3.Error):
            code = getattr(failure, "sqlite_errorcode", None)
            if (code is not None and code & 0xFF == sqlite3.SQLITE_FULL) or (
                "database or disk is full" in str(failure)
            ):
                return True
        if isinstance(failure, OSError) and failure.errno in (errno.ENOSPC, errno.EDQUOT):
            return True
        failure = failure.__cause__ or failure.__context__
        seen += 1
    return False


def disk_free(path: Path) -> int:
    """What the disk holding `path` has free: what sets the hub's limit when no
    quota does. The tests replace it, so the host's own disk never decides one."""
    return shutil.disk_usage(path).free


class SpaceRepository:
    """The database file's size against its limit.

    `quota_bytes` caps the database (`storage_quota_bytes`); without it the
    disk sets the limit (`STORAGE_FREE_FLOOR_BYTES`). `free_space` measures
    what the disk has free; tests give their own.
    """

    def __init__(self, path: Path, quota_bytes: int | None, free_space: Callable[[], int] | None):
        self.path = path
        self.quota_bytes = quota_bytes
        self.free_space = free_space or (lambda: disk_free(path.parent))

    def status(self) -> dict:
        """What the database uses, its limit and what is left, with whether
        it is nearly full (`STORAGE_WARNING_PERCENT`) and what sets the limit:
        `disk` (`STORAGE_FREE_FLOOR_BYTES`) or `quota` (`storage_quota_bytes`)."""
        used = self.physical_bytes()
        disk_limit = max(0, (used + self.free_space() - STORAGE_FREE_FLOOR_BYTES) // 2)
        if self.quota_bytes is not None and self.quota_bytes <= disk_limit:
            limit, source = self.quota_bytes, "quota"
        else:
            limit, source = disk_limit, "disk"
        return {
            "used_bytes": used,
            "max_bytes": limit,
            "remaining_bytes": max(0, limit - used),
            "limit": source,
            "nearly_full": used * 100 >= limit * STORAGE_WARNING_PERCENT,
        }

    def full(self) -> HubError:
        """The refusal for a write the disk itself refused, with its numbers."""
        try:
            free = self.free_space()
        except OSError:
            free = None
        numbers = {"used_bytes": self.physical_bytes()}
        if free is not None:
            numbers["free_bytes"] = free
        return HubError(
            "storage_full",
            "The server disk is full and nothing was saved; free space on the server",
            numbers=numbers,
        )

    def physical_bytes(self) -> int:
        """Observed SQLite, WAL and shared-memory bytes, not just audio payloads."""
        total = 0
        for path in (self.path, Path(str(self.path) + "-wal"), Path(str(self.path) + "-shm")):
            try:
                total += path.stat().st_size
            except FileNotFoundError:
                continue  # SQLite can remove a checkpointed sidecar.
        return total

    def ensure_capacity(self, payload_bytes: int) -> None:
        """Conservative admission check, called inside the writer transaction.

        SQLite page/index/checkpoint growth is not a precise payload-size function.
        Reserve three copies plus 256 KiB; a filesystem quota is still required for
        a strict physical ceiling. Revocation and existing-job state changes remain
        available at the admission ceiling; no content is deleted or rotated.
        The limit is `status`'s: the configured quota, or what the disk leaves
        (`STORAGE_FREE_FLOOR_BYTES`).
        """
        payload_bytes = max(0, payload_bytes)
        reserve = payload_bytes * 3 + 256 * 1024
        status = self.status()
        used, limit = status["used_bytes"], status["max_bytes"]
        if used + reserve > limit:
            # The refusal says the rule that fired: what the write brought, what
            # admission reserves for it, what the database already uses, the
            # limit, what is left of it and what sets it (`disk` or `quota`).
            message = (
                "Server quota reached; raise storage_quota_bytes or free space. "
                if status["limit"] == "quota"
                else "Server space exhausted; free space on the server. "
            )
            raise HubError(
                "quota_exceeded",
                message + "The Mac retains its copy",
                numbers={
                    "bytes": payload_bytes,
                    "reserved_bytes": reserve,
                    "used_bytes": used,
                    "max_bytes": limit,
                    "remaining_bytes": status["remaining_bytes"],
                    "limit": status["limit"],
                },
            )
