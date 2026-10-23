"""Deleting one recording of a device and everything made from it.

«Borrar reunión» and «Terminar sin guardar» on the Mac: its audio, its live
transcript, what the final pass made of it, its note with its tasks, its
speakers, its private notes, its title, its language and its choice of
dictionaries leave the hub in one transaction. The dictionaries themselves
stay: they are the device's, shared by the meetings and series that use them.

Only the named recording of the asking device, by its identity, never by a
pattern. The hub keeps every recording in its database, the audio included,
so there are no files to remove. A row in `deleted_recordings` (identities
and a time, no content) refuses a chunk or a closure that arrives late, so
the recording cannot come back half, and answers a second delete as done.
The nightly copy keeps the database from before the delete for its retention
(`docs/self-hosting.md`).
"""

from ..protocol import HubError

SCHEMA = """
CREATE TABLE IF NOT EXISTS deleted_recordings (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    deleted REAL NOT NULL, PRIMARY KEY(device_id, recording));
"""

_CHUNKS = "SELECT id FROM chunks WHERE device_id=? AND recording=?"
_GROUPS = "SELECT id FROM transcription_groups WHERE device_id=? AND recording=?"

# Children before their parents, as the foreign keys require: what hangs
# from a chunk or a group first, then the chunks and groups, then the rest.
_BY_CHUNK = ("transcripts", "transcription_jobs", "chunk_receipts", "transcription_group_chunks")
_BY_GROUP = ("group_transcripts",)
_BY_RECORDING = (
    "transcription_groups",
    "chunks",
    "recording_manifests",
    "recording_final_passes",
    "recording_final_transcripts",
    "recording_voices",
    "recording_speakers",
    "recording_private_notes",
    "recording_titles",
    "recording_languages",
    "recording_notes",
    "recording_dictionaries",
)


def deleted(db, device: str, recording: str) -> bool:
    """The device deleted this recording: nothing of it is taken again."""
    return (
        db.execute(
            "SELECT 1 FROM deleted_recordings WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        is not None
    )


def refuse_if_deleted(db, device: str, recording: str) -> None:
    if deleted(db, device, recording):
        raise HubError(
            "recording_deleted",
            "La grabación se borró; no admite nada más",
            http_status=409,
        )


def delete(store, device: str, recording: str, now: float) -> dict[str, int]:
    """Delete the recording in one transaction; how many rows of each kind went.

    A recording the hub never had, or already deleted, is deleted too: the
    Mac may have discarded it before any of it arrived.
    """
    removed: dict[str, int] = {}
    with store.transaction() as db:
        active = db.execute("SELECT revoked FROM devices WHERE id=?", (device,)).fetchone()
        if active is None or active["revoked"]:
            raise HubError("revoked", "Dispositivo revocado")
        for table in _BY_CHUNK:
            removed[table] = db.execute(
                f"DELETE FROM {table} WHERE device_id=? AND chunk_id IN ({_CHUNKS})",
                (device, device, recording),
            ).rowcount
        for table in _BY_GROUP:
            removed[table] = db.execute(
                f"DELETE FROM {table} WHERE device_id=? AND group_id IN ({_GROUPS})",
                (device, device, recording),
            ).rowcount
        for table in _BY_RECORDING:
            removed[table] = db.execute(
                f"DELETE FROM {table} WHERE device_id=? AND recording=?", (device, recording)
            ).rowcount
        db.execute(
            "INSERT OR IGNORE INTO deleted_recordings(device_id, recording, deleted) VALUES (?,?,?)",
            (device, recording, now),
        )
    return removed
