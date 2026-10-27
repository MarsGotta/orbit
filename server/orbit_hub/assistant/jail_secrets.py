"""Bounded, metadata-only discovery of host files to mask in a vendor jail."""

import json
import os
import sys
from dataclasses import dataclass, field
from fnmatch import fnmatchcase
from pathlib import Path
from threading import Event

# Names-only operator measurement: 592,505 entries after exclusions, 207
# matching files (199 readable), at depths 2–8; walking took 3.7–6.9 seconds.
# Over 3x the measured entries and twice the depth allow growth while bounding
# background disk work. These discovery budgets do not bound assistant runtime
# or claim that every host secret is hidden. Registered in constants.json.
MAX_SECRET_DEPTH = 16
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
    limits: set[str] = field(default_factory=set)
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


def walk(folder: Path, depth: int, scan: Scan, omitted: tuple[Path, ...], stop: Event) -> None:
    with os.scandir(folder) as entries:
        while scan.visited < MAX_SECRET_ENTRIES and not stop.is_set():
            entry = next(entries, None)
            if entry is None:
                return
            scan.visited += 1
            try:
                if entry.is_dir(follow_symlinks=False):
                    if entry.name in SKIP_DIRECTORIES:
                        continue
                    path = Path(entry.path)
                    if any(within(path, root) for root in omitted):
                        continue
                    if depth >= MAX_SECRET_DEPTH:
                        scan.limits.add(f"profundidad {MAX_SECRET_DEPTH}")
                    else:
                        walk(path, depth + 1, scan, omitted, stop)
                elif any(fnmatchcase(entry.name, pattern) for pattern in SECRET_NAMES):
                    target = Path(entry.path).resolve()
                    if (
                        not any(within(target, root) for root in omitted)
                        and target.is_file()
                        and os.access(target, os.R_OK)
                    ):
                        scan.paths.add(target)
            except (FileNotFoundError, RuntimeError):
                # Deleted entries and Python 3.11 symlink loops carry no payload.
                continue
        if scan.visited >= MAX_SECRET_ENTRIES:
            scan.limits.add(f"entradas {scan.visited}/{MAX_SECRET_ENTRIES}")


def warn_incomplete(root: Path, scan: Scan) -> None:
    # The general log formatter deliberately drops free text and paths. This
    # operator notice explicitly carries only the configured root and counts,
    # never discovered filenames or payloads. JSON keeps each notice on one line.
    message = (
        f"Búsqueda de secretos incompleta en {root}: límite de {', '.join(sorted(scan.limits))}; "
        f"se ocultan {len(scan.paths)} archivos encontrados tras visitar {scan.visited} entradas. "
        "El asistente continúa; quedan rutas sin revisar."
    )
    print(
        json.dumps(
            {"level": "WARNING", "event": "assistant_secret_scan_incomplete", "message": message},
            ensure_ascii=False,
        ),
        file=sys.stderr,
    )


def scan_root(root: Path, omitted: tuple[Path, ...], stop: Event) -> tuple[Path, ...]:
    if any(within(root, path) for path in omitted):
        return ()
    scan = Scan()
    try:
        walk(root, 0, scan, omitted, stop)
    except FileNotFoundError:
        # A service root that no longer exists exposes no files.
        return ()
    if scan.limits:
        warn_incomplete(root, scan)
    return tuple(sorted(scan.paths))
