"""Bounded, metadata-only discovery of host files to mask in a vendor jail."""

import json
import os
import sys
from collections import deque
from dataclasses import dataclass, field
from fnmatch import fnmatchcase
from pathlib import Path
from threading import Event

# Names-only operator measurement: 592,505 entries after exclusions, 207
# matching files (199 readable), at depths 2–8; walking took 3.7–6.9 seconds.
# Over 3x the measured entries bounds background disk work while allowing
# growth. There is no depth bound: a secret is not safer for being deep, and
# the 16-level bound it replaced left deeper files visible. The walk
# is breadth first, so when this bound is reached every shallower folder has
# been read. It does not bound assistant runtime or claim that every host
# secret is hidden. Registered in constants.json.
MAX_SECRET_ENTRIES = 2_000_000
SKIP_DIRECTORIES = {
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "virtualenv",
    ".cache",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    "dist",
    "build",
}
SECRET_NAMES = (
    ".env*",
    "credentials*",
    ".npmrc",
    ".netrc",
    "*.pem",
    "*.key",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ecdsa_sk",
    "id_ed25519_sk",
    "token",
    "config.toml",
    "auth.json",
    "settings*.json",
)


@dataclass
class Scan:
    visited: int = 0
    limit: str | None = None
    paths: set[Path] = field(default_factory=set)


def within(path: Path, root: Path) -> bool:
    return path == root or root in path.parents


def excluded(path: Path, work: Path, vendor: Path, hidden: list[Path]) -> bool:
    if within(path, vendor) or within(path, Path("/tmp")):
        return True
    # A working folder rebound over an empty workspace tree is visible again:
    # its existing secrets must be masked after that writable bind.
    return (
        not within(path, work)
        and not within(work, path)
        and any(within(path, root) for root in hidden)
    )


def walk(root: Path, scan: Scan, omitted: tuple[Path, ...], stop: Event) -> None:
    # A queue, not recursion: a chain deeper than Python's recursion limit
    # would raise inside the loop below and hide its secrets silently. It
    # holds plain strings, the least memory for the folders still to read.
    folders = deque([str(root)])
    while folders and not stop.is_set():
        folder = folders.popleft()
        try:
            listing = os.scandir(folder)
        except FileNotFoundError:
            if folder == str(root):
                raise
            # Deleted since it was listed: it carries no payload.
            continue
        except PermissionError:
            # The jail runs as the hub's user: a folder it can neither list nor
            # enter exposes nothing. One it can enter could open a guessed name.
            if os.access(folder, os.X_OK):
                raise
            continue
        with listing as entries:
            while not stop.is_set():
                if scan.visited >= MAX_SECRET_ENTRIES:
                    scan.limit = f"entries {scan.visited}/{MAX_SECRET_ENTRIES}"
                    return
                entry = next(entries, None)
                if entry is None:
                    break
                scan.visited += 1
                try:
                    if entry.is_dir(follow_symlinks=False):
                        path = Path(entry.path)
                        if entry.name not in SKIP_DIRECTORIES and not any(
                            within(path, hidden) for hidden in omitted
                        ):
                            folders.append(entry.path)
                    elif any(fnmatchcase(entry.name, pattern) for pattern in SECRET_NAMES):
                        target = Path(entry.path).resolve()
                        if (
                            not any(within(target, hidden) for hidden in omitted)
                            and target.is_file()
                            and os.access(target, os.R_OK)
                        ):
                            scan.paths.add(target)
                except (FileNotFoundError, RuntimeError):
                    # Deleted entries and Python 3.11 symlink loops carry no payload.
                    continue


def incomplete_notice(root: Path, scan: Scan) -> str:
    # The general log formatter deliberately drops free text and paths. This
    # operator notice explicitly carries only the configured root and counts,
    # never discovered filenames or payloads.
    return f"Secret scan incomplete in {root}: limit of {scan.limit}; hiding {len(scan.paths)} files found after visiting {scan.visited} entries. The assistant continues; some paths remain unchecked."


def warn_incomplete(notice: str) -> None:
    # JSON keeps each notice on one line.
    print(
        json.dumps(
            {"level": "WARNING", "event": "assistant_secret_scan_incomplete", "message": notice},
            ensure_ascii=False,
        ),
        file=sys.stderr,
    )


def scan_root(
    root: Path, omitted: tuple[Path, ...], stop: Event
) -> tuple[tuple[Path, ...], str | None]:
    """The readable secret files under `root`, and the notice when a bound cut the walk."""
    if any(within(root, path) for path in omitted):
        return (), None
    scan = Scan()
    try:
        walk(root, scan, omitted, stop)
    except FileNotFoundError:
        # A service root that no longer exists exposes no files.
        return (), None
    notice = incomplete_notice(root, scan) if scan.limit else None
    return tuple(sorted(scan.paths)), notice
