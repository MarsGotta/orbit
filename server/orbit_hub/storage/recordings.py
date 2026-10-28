"""The audio a Mac uploads, chunk by chunk, and the manifest that closes it.

Each chunk is kept encrypted with its transcription job; the manifest is
accepted only when it accounts for every chunk the hub holds. The groups a
recording's chunks are transcribed in are `group_storage.GroupRepository`'s.
"""

import hashlib
import json
import sqlite3

from ..meetings.deletion import refuse_if_deleted
from ..protocol import HubError

_MISMATCH = "Count or sequences do not match; confirmed chunks are missing"


class RecordingRepository:
    def __init__(self, store):
        self.store = store

    def put_chunk(
        self,
        device: str,
        chunk_id: str,
        recording: str,
        track: str,
        sequence: int,
        digest: str,
        body: bytes,
        metadata: dict | None = None,
        *,
        received_at: float = 0,
    ) -> dict:
        if hashlib.sha256(body).hexdigest() != digest:
            raise HubError("hash_mismatch", "The hash does not match received bytes")
        with self.store.transaction() as db:
            _require_active(db, device)
            row = db.execute(
                "SELECT * FROM chunks WHERE device_id=? AND id=?", (device, chunk_id)
            ).fetchone()
            if row:
                self._same_chunk(
                    db, device, chunk_id, row, (digest, recording, track, sequence), metadata
                )
            else:
                refuse_if_deleted(db, device, recording)
                if db.execute(
                    "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
                    (device, recording),
                ).fetchone():
                    raise HubError(
                        "recording_finalized",
                        "Capture is closed; new chunks are not accepted",
                    )
                self._insert_chunk(
                    db,
                    device,
                    chunk_id,
                    (recording, track, sequence, digest),
                    body,
                    metadata,
                    received_at,
                )
        return {"chunk_id": chunk_id, "sha256": digest, "durable": True}

    def _same_chunk(self, db, device, chunk_id, row, identity, metadata) -> None:
        """A chunk sent again must be the one the hub holds, metadata included."""
        job = db.execute(
            "SELECT metadata FROM transcription_jobs WHERE device_id=? AND chunk_id=?",
            (device, chunk_id),
        ).fetchone()
        old_metadata = (
            json.loads(self.store.decrypt(job["metadata"], f"metadata:{device}:{chunk_id}"))
            if job
            else None
        )
        if old_metadata != metadata:
            raise HubError("chunk_conflict", "Capture metadata differs for the same chunk")
        if (row["sha256"], row["recording"], row["track"], row["sequence"]) != identity:
            raise HubError(
                "chunk_conflict", "The chunk already exists with different content or metadata"
            )

    def _insert_chunk(self, db, device, chunk_id, where, body, metadata, received_at) -> None:
        recording, track, sequence, digest = where
        self.store.ensure_capacity(len(body) + len(json.dumps(metadata).encode()) + 1024)
        try:
            db.execute(
                "INSERT INTO chunks VALUES (?,?,?,?,?,?,?,?)",
                (
                    chunk_id,
                    device,
                    recording,
                    track,
                    sequence,
                    digest,
                    self.store.encrypt(body, f"chunk:{device}:{chunk_id}"),
                    len(body),
                ),
            )
        except sqlite3.IntegrityError as exc:
            raise HubError(
                "chunk_conflict", "The sequence already belongs to another chunk"
            ) from exc
        if metadata is not None:
            db.execute(
                "INSERT INTO transcription_jobs(device_id,chunk_id,metadata) VALUES (?,?,?)",
                (
                    device,
                    chunk_id,
                    self.store.encrypt(
                        json.dumps(metadata, sort_keys=True).encode(),
                        f"metadata:{device}:{chunk_id}",
                    ),
                ),
            )
            db.execute("INSERT INTO chunk_receipts VALUES (?,?,?)", (device, chunk_id, received_at))

    def finalize(
        self,
        device: str,
        recording: str,
        manifest: dict,
        *,
        final_pass: bool = False,
        final_pass_error: dict | None = None,
    ) -> dict:
        # The HTTP close asks for the final pass atomically with the manifest.
        # Importers/fixtures of historical recordings retain their draft unless
        # explicitly requested.
        tracks = {track["track"]: track for track in manifest["tracks"]}
        if (
            len(tracks) != len(manifest["tracks"])
            or manifest["finalized_at_ms"] < manifest["started_at_ms"]
        ):
            raise HubError(
                "invalid_manifest", "Duplicate tracks or closure time before recording start"
            )
        with self.store.transaction() as db:
            _require_active(db, device)
            previous = db.execute(
                "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if previous:
                self._same_manifest(device, recording, previous, manifest)
            else:
                refuse_if_deleted(db, device, recording)
                self._check_tracks(db, device, recording, tracks)
                self._insert_manifest(db, device, recording, manifest, final_pass, final_pass_error)
        transcript = self.store.groups.capture(device, recording)
        return {
            "recording_id": recording,
            "finalized": True,
            "durable": True,
            "capture_complete": transcript["capture_complete"],
            "transcript_ready": transcript["transcript_ready"],
        }

    def _same_manifest(self, device, recording, previous, manifest) -> None:
        old = json.loads(self.store.decrypt(previous["body"], f"manifest:{device}:{recording}"))
        # A field the manifest does not carry reads as its default, so
        # a retried closure matches whichever of them it left out.
        for field, default in (("title", None), ("kind", "meeting"), ("event", None)):
            old.setdefault(field, default)
            manifest.setdefault(field, default)
        if old != manifest:
            raise HubError(
                "manifest_conflict", "The recording already has a different closure manifest"
            )

    def _check_tracks(self, db, device, recording, tracks) -> None:
        rows = db.execute(
            """SELECT c.id,c.track,c.sequence,j.metadata FROM chunks c
            LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
            WHERE c.device_id=? AND c.recording=? ORDER BY c.track,c.sequence""",
            (device, recording),
        ).fetchall()
        if {row["track"] for row in rows} != set(tracks):
            raise HubError(
                "manifest_mismatch",
                "Manifest tracks do not match durable chunks",
            )
        for name, declared in tracks.items():
            self._check_track(device, [row for row in rows if row["track"] == name], declared)

    def _check_track(self, device, actual, declared) -> None:
        count = declared["chunk_count"]
        # The hub holds every sequence from 0 to the last one,
        # except those the Mac declared it will never get.
        missing = set(declared.get("missing_sequences") or ())
        if declared["last_sequence"] + 1 - len(missing) != count:
            raise HubError("manifest_mismatch", _MISMATCH)
        expected = [
            sequence for sequence in range(declared["last_sequence"] + 1) if sequence not in missing
        ]
        if len(actual) != count or [row["sequence"] for row in actual] != expected:
            raise HubError("manifest_mismatch", _MISMATCH)
        frames = 0
        for row in actual:
            metadata = (
                json.loads(self.store.decrypt(row["metadata"], f"metadata:{device}:{row['id']}"))
                if row["metadata"]
                else {}
            )
            if not isinstance(metadata.get("frames"), int):
                raise HubError(
                    "manifest_mismatch",
                    "Cannot attest a chunk's frame count",
                )
            frames += metadata["frames"]
        if frames != declared["total_frames"]:
            raise HubError("manifest_mismatch", "Total frame count does not match")

    def _insert_manifest(self, db, device, recording, manifest, final_pass, error) -> None:
        self.store.ensure_capacity(len(json.dumps(manifest).encode()) + 1024)
        db.execute(
            "INSERT INTO recording_manifests(device_id,recording,body) VALUES (?,?,?)",
            (
                device,
                recording,
                self.store.encrypt(
                    json.dumps(manifest, sort_keys=True).encode(),
                    f"manifest:{device}:{recording}",
                ),
            ),
        )
        if final_pass:
            db.execute(
                "INSERT OR IGNORE INTO recording_final_passes(device_id,recording,state,error) "
                "VALUES (?,?,?,?)",
                (
                    device,
                    recording,
                    "failed" if error else "queued",
                    json.dumps(error) if error else None,
                ),
            )


def _require_active(db, device: str) -> None:
    active = db.execute("SELECT revoked FROM devices WHERE id=?", (device,)).fetchone()
    if active is None or active["revoked"]:
        raise HubError("revoked", "Device revoked")
