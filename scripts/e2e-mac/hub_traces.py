"""What the test hub keeps of one recording, read from its own database.

The hub keeps every recording in its SQLite database, the audio included
(server/orbit_hub/meetings/deletion.py): a recording it deleted has no row
left in any table that names it, by its `recording` column or through its
chunks and transcription groups, and leaves one tombstone in
`deleted_recordings` (identities and a time, no content). Every table is
asked, not a list of them, so a table the hub adds later is counted too.

The database is opened read-only, as a second reader beside the running
test hub (WAL lets both read); nothing here writes to it. Only counts leave
it, never a row.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

TOMBSTONES = "deleted_recordings"


def database(hub: Path) -> Path:
    """The test hub's database under its data directory (isolated_hub.config_text)."""
    return hub / "data" / "hub.sqlite3"


def kept(path: Path, recording: str) -> dict:
    """`{"rows": {table: count}, "deleted": bool}` for `recording`: how many
    rows of each table still name it, and whether its tombstone is there."""
    connection = sqlite3.connect(f"{path.resolve().as_uri()}?mode=ro", uri=True, timeout=5)
    try:
        # A test meeting of a few minutes has a few hundred chunks, far
        # under SQLite's 32 766 bound parameters of one statement.
        by = {
            "recording": [recording],
            "chunk_id": _ids(connection, "chunks", recording),
            "group_id": _ids(connection, "transcription_groups", recording),
        }
        rows: dict[str, int] = {}
        tables = connection.execute("SELECT name FROM sqlite_master WHERE type='table'")
        for (table,) in tables.fetchall():
            columns = {row[1] for row in connection.execute(f'PRAGMA table_info("{table}")')}
            named = [
                (column, values) for column, values in by.items() if column in columns and values
            ]
            if not named:
                continue
            # A row that names it twice (a group's chunk) is counted once.
            where = " OR ".join(f"{column} IN ({','.join('?' * len(v))})" for column, v in named)
            arguments = [value for _, values in named for value in values]
            (found,) = connection.execute(
                f'SELECT count(*) FROM "{table}" WHERE {where}', arguments
            ).fetchone()
            if found:
                rows[table] = found
    finally:
        connection.close()
    deleted = rows.pop(TOMBSTONES, 0) > 0
    return {"rows": rows, "deleted": deleted}


def _ids(connection: sqlite3.Connection, table: str, recording: str) -> list[str]:
    query = f'SELECT id FROM "{table}" WHERE recording=?'
    return [row[0] for row in connection.execute(query, (recording,))]
