"""Consistent copies of the hub's database: every night off the server, and
on the server before each activation.

The hub keeps everything in one SQLite database (`hub.sqlite3`, WAL mode),
and the Mac deletes its audio once the hub says a chunk is durable: without
copies the hub's disk is the only one. A copy is taken with SQLite's online
backup API, so it is consistent with the WAL while the hub keeps writing, and
it is written beside its destination and renamed into place, so a copy is
either whole or absent.

The key is never part of the nightly copy: the database is encrypted with it,
and a copy that travels with its key is as readable as the data it protects.
The operator keeps the key apart (docs/self-hosting.md, "Backups") and
`orbit-hub restore` pairs them again, refusing a key the copy was not written
with. The copy before an activation stays on the server, next to the key it
was taken with, so `activate.sh` can put both back.

Standard library only: `scripts/hub/activate.sh` runs this file with the
server's own `python3 -I`, whichever release (or none) is active.
"""

import argparse
import os
import re
import shutil
import sqlite3
import sys
import tomllib
from datetime import UTC, date, datetime
from pathlib import Path

# How many nightly copies are kept: the last seven days and the last four
# weeks (the first copy of each ISO week). Mars's choice (2026-10-22). What it
# protects: the destination's disk, which holds every copy in full (the
# database's audio is encrypted and does not compress). It is not a limit on
# the hub: the hub deletes nothing.
DAILY_KEPT = 7
WEEKLY_KEPT = 4

DATABASE = "hub.sqlite3"
_DAILY = re.compile(r"^hub-\d{4}-\d{2}-\d{2}\.sqlite3$")
_WEEKLY = re.compile(r"^hub-\d{4}-W\d{2}\.sqlite3$")
# The copy before an activation, under the backups directory it is given.
ACTIVATION = "activation"
ACTIVATION_KEY = "storage.key"
ACTIVATION_LABEL = "FROM"


class BackupError(Exception):
    """A copy or a restore that could not be done, said with its cause."""


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _fsync_file(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _new_private_file(path: Path) -> None:
    path.unlink(missing_ok=True)
    os.close(os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600))


def database_bytes(data_dir: Path) -> int:
    """The database with its WAL: what a copy can grow to at most."""
    total = 0
    for suffix in ("", "-wal"):
        try:
            total += (data_dir / (DATABASE + suffix)).stat().st_size
        except FileNotFoundError:
            continue
    return total


def _same_disk(one: Path, other: Path) -> bool:
    return one.stat().st_dev == other.stat().st_dev


def _require_room(directory: Path, needed: int) -> None:
    free = shutil.disk_usage(directory).free
    if free < needed:
        raise BackupError(
            f"{directory} has {free} bytes free and the copy needs up to {needed}; "
            "free space there first"
        )


def copy_database(source: Path, target: Path) -> int:
    """A consistent copy of `source` at `target`, whole or absent; its size.

    The copy is a standalone file (rollback journal, no WAL beside it), 0600.
    """
    if not source.is_file():
        raise BackupError(f"there is no database at {source}")
    partial = target.with_name(f".{target.name}.partial")
    _new_private_file(partial)
    try:
        origin = sqlite3.connect(source, isolation_level=None)
        try:
            origin.execute("PRAGMA query_only=ON")
            copy = sqlite3.connect(partial, isolation_level=None)
            try:
                origin.backup(copy)
                copy.execute("PRAGMA journal_mode=DELETE")
            finally:
                copy.close()
        finally:
            origin.close()
        _fsync_file(partial)
        os.replace(partial, target)
        _fsync_directory(target.parent)
    except BaseException:
        partial.unlink(missing_ok=True)
        raise
    return target.stat().st_size


def _place(copy: Path, target: Path) -> None:
    """Copy a backup file to `target` (0600), whole or absent."""
    partial = target.with_name(f".{target.name}.partial")
    _new_private_file(partial)
    try:
        with copy.open("rb") as source, partial.open("wb") as written:
            shutil.copyfileobj(source, written, 1024 * 1024)
            written.flush()
            os.fsync(written.fileno())
        os.replace(partial, target)
        _fsync_directory(target.parent)
    except BaseException:
        partial.unlink(missing_ok=True)
        raise


def schema(database: Path) -> tuple:
    """What a release's code sees of a database: its tables, indexes and version."""
    connection = sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)
    try:
        objects = connection.execute(
            "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name"
        ).fetchall()
        version = connection.execute("PRAGMA user_version").fetchone()[0]
    finally:
        connection.close()
    return version, objects


# --- Every night, off the server ------------------------------------------------


def nightly(data_dir: Path, destination: Path, today: date) -> list[str]:
    """Today's copy in `destination`, rotated; what was done, one line each.

    `destination` must exist and be on another disk than the data: it is
    usually another machine's disk mounted there, and a missing mount (or a
    directory created in its place) would put the copies on the server's own
    disk. Only copies this function named are ever removed.
    """
    if not destination.is_dir():
        raise BackupError(
            f"the destination {destination} does not exist or is not a directory; "
            "if it is another machine's disk, check that it is mounted"
        )
    if _same_disk(destination, data_dir):
        raise BackupError(
            f"{destination} is on the same disk as {data_dir}: a copy there does not survive "
            "that disk. If another machine's disk should be mounted there, it is not"
        )
    said = []
    daily, weekly = destination / "daily", destination / "weekly"
    for directory in (daily, weekly):
        directory.mkdir(mode=0o700, exist_ok=True)
    _require_room(daily, database_bytes(data_dir))
    target = daily / f"hub-{today.isoformat()}.sqlite3"
    size = copy_database(data_dir / DATABASE, target)
    said.append(f"copied {data_dir / DATABASE} to {target} ({size} bytes)")
    year, week, _ = today.isocalendar()
    week_copy = weekly / f"hub-{year}-W{week:02d}.sqlite3"
    if not week_copy.exists():
        try:
            os.link(target, week_copy)
        except OSError:
            # A destination that cannot hard-link (some network mounts).
            _place(target, week_copy)
        _fsync_directory(weekly)
        said.append(f"kept {week_copy} as this week's copy")
    for directory, pattern, kept in ((daily, _DAILY, DAILY_KEPT), (weekly, _WEEKLY, WEEKLY_KEPT)):
        names = sorted(path.name for path in directory.iterdir() if pattern.match(path.name))
        for name in names[:-kept]:
            (directory / name).unlink()
            said.append(f"removed {directory / name}, older than the {kept} kept")
    return said


def restore_copy(copy: Path, data_dir: Path) -> int:
    """Put a copy in an empty data directory; its size.

    The caller opens it with the key (`orbit-hub restore`), which refuses a
    key the copy was not written with.
    """
    if not copy.is_file():
        raise BackupError(f"there is no copy at {copy}")
    database = data_dir / DATABASE
    if database.exists() or database.is_symlink():
        raise BackupError(
            f"{data_dir} already holds a database; restore into an empty data directory "
            "(move the current one aside first)"
        )
    data_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    _require_room(data_dir, copy.stat().st_size)
    _place(copy, database)
    return database.stat().st_size


# --- Before each activation, on the server ------------------------------------------


def activation_label(backups: Path) -> str | None:
    """The version the copy before the last activation was taken from."""
    try:
        text = (backups / ACTIVATION / ACTIVATION_LABEL).read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    found = re.search(r"(?m)^version=(.+)$", text)
    return found.group(1) if found else None


def activation_copy(data_dir: Path, key_file: Path, backups: Path, version: str, now: datetime):
    """The database and its key as release `version` left them, in
    `backups/activation`, replacing the copy of the activation before."""
    database = data_dir / DATABASE
    if not database.exists():
        return [f"there is no database at {database} yet: nothing to copy"]
    backups.mkdir(mode=0o700, parents=True, exist_ok=True)
    _require_room(backups, database_bytes(data_dir))
    staged = backups / f".{ACTIVATION}.new"
    shutil.rmtree(staged, ignore_errors=True)
    staged.mkdir(mode=0o700)
    try:
        size = copy_database(database, staged / DATABASE)
        _place(key_file, staged / ACTIVATION_KEY)
        label = staged / ACTIVATION_LABEL
        _new_private_file(label)
        label.write_text(
            f"version={version}\ntaken_at={now.astimezone(UTC).strftime('%Y-%m-%dT%H:%M:%SZ')}\n",
            encoding="utf-8",
        )
        _fsync_file(label)
        _fsync_directory(staged)
        current, retired = backups / ACTIVATION, backups / f".{ACTIVATION}.old"
        shutil.rmtree(retired, ignore_errors=True)
        if current.exists():
            os.replace(current, retired)
        os.replace(staged, current)
        _fsync_directory(backups)
        shutil.rmtree(retired, ignore_errors=True)
    except BaseException:
        shutil.rmtree(staged, ignore_errors=True)
        raise
    return [f"copied the database ({size} bytes) and its key as {version} left them to {current}"]


def activation_plan(data_dir: Path, backups: Path, version: str) -> str:
    """What activating `version` does with the database:

    - `restore`: the copy was taken from `version` and the database has
      another schema since, so `version`'s code needs the copy back;
    - `keep`: the copy was taken from `version` and the schema is the same,
      so the database keeps everything written since;
    - `copy`: anything else; a new copy is taken first.
    """
    copy = backups / ACTIVATION / DATABASE
    database = data_dir / DATABASE
    if activation_label(backups) != version or not copy.is_file() or not database.exists():
        return "copy"
    return "keep" if schema(copy) == schema(database) else "restore"


def activation_restore(data_dir: Path, key_file: Path, backups: Path, now: datetime) -> list[str]:
    """Put back the copy before the last activation, with the hub stopped.

    The database it replaces is moved aside, never deleted: what was written
    since the copy is still there.
    """
    copy_dir = backups / ACTIVATION
    if key_file.read_bytes() != (copy_dir / ACTIVATION_KEY).read_bytes():
        raise BackupError(
            f"the key in {key_file} is not the one the copy in {copy_dir} was taken with; "
            "nothing was touched"
        )
    stamp = now.astimezone(UTC).strftime("%Y%m%dT%H%M%SZ")
    aside = backups / f"set-aside-{stamp}"
    aside.mkdir(mode=0o700)
    moved = []
    try:
        for suffix in ("", "-wal", "-shm"):
            path = data_dir / (DATABASE + suffix)
            if path.exists():
                shutil.move(path, aside / path.name)
                moved.append(path)
        _fsync_directory(aside)
        _fsync_directory(data_dir)
        _place(copy_dir / DATABASE, data_dir / DATABASE)
    except BaseException:
        # The database goes back where it was: a hub started on an empty
        # data directory would begin a new, empty one.
        for path in moved:
            shutil.move(aside / path.name, path)
        raise
    return [
        f"put back the database as {activation_label(backups)} left it ({copy_dir})",
        f"the database it replaced, with everything written since the copy, is in {aside}",
    ]


# --- Run as a script, by activate.sh ----------------------------------------------


def configured_paths(config: Path) -> tuple[Path, Path]:
    """`data_dir` and `key_file` as the hub reads them from its configuration."""
    with config.open("rb") as handle:
        values = tomllib.load(handle)
    try:
        return Path(values["data_dir"]).expanduser(), Path(values["key_file"]).expanduser()
    except KeyError as missing:
        raise BackupError(f"{config} does not say {missing.args[0]}") from None


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The copy of the Orbit hub's database around an activation"
    )
    parser.add_argument("--config", type=Path, required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("activation-copy", "activation-plan", "activation-restore"):
        command = commands.add_parser(name)
        command.add_argument("backups", type=Path)
        command.add_argument("--version", required=name != "activation-restore")
    args = parser.parse_args(arguments)
    try:
        data_dir, key_file = configured_paths(args.config)
        now = datetime.now(UTC)
        if args.command == "activation-copy":
            said = activation_copy(data_dir, key_file, args.backups, args.version, now)
        elif args.command == "activation-plan":
            said = [activation_plan(data_dir, args.backups, args.version)]
        else:
            said = activation_restore(data_dir, key_file, args.backups, now)
    except (OSError, sqlite3.Error, tomllib.TOMLDecodeError, BackupError) as failure:
        print(f"Orbit hub backup: {failure}", file=sys.stderr)
        return 1
    for line in said:
        print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
