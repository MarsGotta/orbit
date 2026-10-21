"""Durable, device-scoped meeting groups over the encrypted chunk repository."""

import json
from contextlib import nullcontext

from .grouping import MAX_GROUP_SOURCES, Source, assemble_wav, group_manifest, select_window
from .log import report
from .meetings import repository as meetings
from .protocol import HubError

SCHEMA = '\nCREATE TABLE IF NOT EXISTS chunk_receipts (\n    device_id TEXT NOT NULL, chunk_id TEXT NOT NULL, received REAL NOT NULL,\n    PRIMARY KEY(device_id,chunk_id),\n    FOREIGN KEY(device_id,chunk_id) REFERENCES chunks(device_id,id));\nCREATE TABLE IF NOT EXISTS transcription_groups (\n    device_id TEXT NOT NULL, id TEXT NOT NULL, recording TEXT NOT NULL, track TEXT NOT NULL,\n    first_sequence INTEGER NOT NULL, last_sequence INTEGER NOT NULL, manifest BLOB NOT NULL,\n    state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,\n    next_attempt REAL NOT NULL DEFAULT 0, error_code TEXT, created REAL NOT NULL,\n    PRIMARY KEY(device_id,id));\nCREATE TABLE IF NOT EXISTS transcription_group_chunks (\n    device_id TEXT NOT NULL, chunk_id TEXT NOT NULL, group_id TEXT NOT NULL,\n    PRIMARY KEY(device_id,chunk_id),\n    FOREIGN KEY(device_id,chunk_id) REFERENCES chunks(device_id,id),\n    FOREIGN KEY(device_id,group_id) REFERENCES transcription_groups(device_id,id));\nCREATE TABLE IF NOT EXISTS group_transcripts (\n    device_id TEXT NOT NULL, group_id TEXT NOT NULL, body BLOB NOT NULL,\n    PRIMARY KEY(device_id,group_id),\n    FOREIGN KEY(device_id,group_id) REFERENCES transcription_groups(device_id,id));\nCREATE INDEX IF NOT EXISTS groups_recording ON transcription_groups(device_id,recording,last_sequence DESC);\nCREATE INDEX IF NOT EXISTS group_members ON transcription_group_chunks(device_id,group_id);\nCREATE INDEX IF NOT EXISTS groups_pending ON transcription_groups(state,next_attempt,created);\n'  # fmt: skip


class GroupRepository:
    """Shares Store's writer transactions and encryption; never owns another connection."""

    def __init__(self, store):
        self.store = store

    def source(self, row) -> Source:
        metadata = json.loads(
            self.store.decrypt(row["metadata"], f"metadata:{row['device_id']}:{row['id']}")
        )
        return Source(
            row["id"], row["sequence"], row["sha256"], row["size"], metadata, row["received"]
        )

    def manifest(self, row) -> dict:
        return json.loads(
            self.store.decrypt(row["manifest"], f"group:{row['device_id']}:{row['id']}")
        )

    def seal(
        self,
        db,
        device,
        recording,
        track,
        sources,
        now,
        reason,
    ):
        manifest = group_manifest(device, recording, track, sources)
        manifest["flush_reason"] = reason
        encoded, group_id = json.dumps(manifest).encode(), manifest["group_id"]
        self.store.ensure_capacity(len(encoded) + len(sources) * 256)
        db.execute(
            "INSERT INTO transcription_groups(device_id,id,recording,track,first_sequence,last_sequence,manifest,state,created) VALUES (?,?,?,?,?,?,?,?,?)",
            (
                device,
                group_id,
                recording,
                track,
                sources[0].sequence,
                sources[-1].sequence,
                self.store.encrypt(encoded, f"group:{device}:{group_id}"),
                "pending",
                now,
            ),
        )
        for source in sources:
            db.execute(
                "INSERT INTO transcription_group_chunks VALUES (?,?,?)",
                (device, source.chunk_id, group_id),
            )
            db.execute(
                "UPDATE transcription_jobs SET state='grouped' WHERE device_id=? AND chunk_id=?",
                (device, source.chunk_id),
            )

    def prepare(self, now: float) -> None:
        """Bound work per poll; existing memberships make sealing/recovery idempotent."""
        with self.store.transaction() as db:
            tracks = db.execute(
                """SELECT DISTINCT c.device_id,c.recording,c.track
                FROM chunks c JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
                LEFT JOIN transcription_group_chunks m ON m.device_id=c.device_id AND m.chunk_id=c.id
                JOIN devices d ON d.id=c.device_id
                WHERE m.chunk_id IS NULL AND j.state IN ('pending','running') AND d.revoked=0
                ORDER BY c.device_id,c.recording,c.track LIMIT 8"""
            ).fetchall()
            for track in tracks:
                device, recording, name = track["device_id"], track["recording"], track["track"]
                rows = db.execute(
                    """SELECT c.device_id,c.id,c.sequence,c.sha256,c.size,c.recording,c.track,j.metadata,coalesce(r.received,0) AS received
                    FROM chunks c JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
                    LEFT JOIN chunk_receipts r ON r.device_id=c.device_id AND r.chunk_id=c.id
                    LEFT JOIN transcription_group_chunks m ON m.device_id=c.device_id AND m.chunk_id=c.id
                    WHERE c.device_id=? AND c.recording=? AND c.track=? AND m.chunk_id IS NULL
                    AND j.state IN ('pending','running') ORDER BY c.sequence LIMIT ?""",
                    (device, recording, name, MAX_GROUP_SOURCES + 1),
                ).fetchall()
                finalized = (
                    db.execute(
                        "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
                        (device, recording),
                    ).fetchone()
                    is not None
                )
                selected = select_window(
                    [self.source(row) for row in rows], now=now, finalized=finalized
                )
                if selected:
                    sources, reason = selected
                    self.seal(db, device, recording, name, sources, now, reason)

    def claim(self, now: float) -> dict | None:
        self.prepare(now)
        with self.store.transaction() as db:
            row = db.execute(
                """SELECT g.* FROM transcription_groups g JOIN devices d ON d.id=g.device_id
                WHERE g.state='pending' AND g.next_attempt<=? AND d.revoked=0
                ORDER BY g.next_attempt,g.created,g.first_sequence LIMIT 1""",
                (now,),
            ).fetchone()
            if row is None:
                return None
            manifest = self.manifest(row)
            db.execute(
                "UPDATE transcription_groups SET state='running',attempts=attempts+1 WHERE device_id=? AND id=?",
                (row["device_id"], row["id"]),
            )
            # The language the user chose for the recording, read as the
            # group is claimed; None sends the configured one.
            chosen = db.execute(
                "SELECT language FROM recording_languages WHERE device_id=? AND recording=?",
                (row["device_id"], row["recording"]),
            ).fetchone()
        job = {
            "device_id": row["device_id"],
            "recording": row["recording"],
            "group_id": row["id"],
            "attempts": row["attempts"],
            "manifest": manifest,
            "language": None if chosen is None else chosen["language"],
        }
        if len(manifest["sources"]) == 1:
            job["chunk_id"] = manifest["sources"][0]["chunk_id"]
        try:
            bodies = []
            with self.store._lock:
                for source in manifest["sources"]:
                    body = self.store._db.execute(
                        "SELECT body FROM chunks WHERE device_id=? AND id=?",
                        (row["device_id"], source["chunk_id"]),
                    ).fetchone()[0]
                    bodies.append(
                        self.store.decrypt(body, f"chunk:{row['device_id']}:{source['chunk_id']}")
                    )
            job["audio"] = assemble_wav(manifest["sources"], bodies)
        except Exception:  # noqa: BLE001 - source corruption cannot escape as plaintext/provider diagnostics
            report(
                event="group_audio_invalid",
                code="invalid_group_audio",
                device=row["device_id"],
            )
            self.fail(job, "invalid_group_audio", now, terminal=True)
            return None
        return job

    def finish(self, job: dict, text: str, language: str | None = None) -> None:
        """Keep the group's text and the language it is in.

        `language` is the one the transcriber gave, else the one it was sent;
        None when neither is known. It goes in its own column, so the text's
        body stays `{"text"}`, the shape the archive reader checks. The text is
        kept even when empty: an empty group is audio without speech, and the
        readers leave it out. A group transcribed again in another language
        has its old text replaced.
        """
        device, group_id = job["device_id"], job["group_id"]
        encoded = json.dumps({"text": text}).encode()
        with self.store.transaction() as db:
            current = db.execute(
                "SELECT state,attempts FROM transcription_groups WHERE device_id=? AND id=?",
                (device, group_id),
            ).fetchone()
            if current is None:
                raise HubError("group_not_found", "Grupo no disponible")
            if current["state"] == "completed":
                return
            if current["state"] != "running" or current["attempts"] != job["attempts"] + 1:
                raise HubError(
                    "stale_group_attempt", "Resultado de un intento de transcripción obsoleto"
                )
            self.store.ensure_capacity(len(encoded) + 1024)
            db.execute(
                "INSERT INTO group_transcripts VALUES (?,?,?) "
                "ON CONFLICT(device_id,group_id) DO UPDATE SET body=excluded.body",
                (
                    device,
                    group_id,
                    self.store.encrypt(encoded, f"group-transcript:{device}:{group_id}"),
                ),
            )
            db.execute(
                "UPDATE transcription_groups SET state='completed',error_code=NULL,language=? WHERE device_id=? AND id=?",
                (language, device, group_id),
            )

    def fail(self, job: dict, code: str, now: float, *, terminal=False) -> None:
        attempts = job["attempts"] + 1
        with self.store.transaction() as db:
            db.execute(
                "UPDATE transcription_groups SET state=?,error_code=?,next_attempt=? WHERE device_id=? AND id=? AND state='running' AND attempts=?",
                (
                    "failed" if terminal or attempts >= 5 else "pending",
                    code,
                    now + min(300, 2**attempts * 5),
                    job["device_id"],
                    job["group_id"],
                    attempts,
                ),
            )

    def retry(self, device: str, recording: str, now: float) -> int:
        """Requeue exhausted groups for one device-owned recording."""
        with self.store.transaction() as db:
            meetings.require(db, device, recording)
            return db.execute(
                """UPDATE transcription_groups
                SET state='pending',attempts=0,error_code=NULL,next_attempt=?
                WHERE device_id=? AND recording=? AND state='failed'""",
                (now, device, recording),
            ).rowcount

    def capture(self, device: str, recording: str, *, db=None) -> dict:
        # An explicitly supplied connection belongs to the caller's read-only
        # snapshot, never to the live writer. The normal path retains its lock.
        reader = db if db is not None else self.store._db
        with nullcontext() if db is not None else self.store._lock:
            rows = reader.execute(
                """SELECT c.track,count(*) AS total,sum(CASE WHEN g.state='completed' OR (m.group_id IS NULL AND j.state='completed') THEN 0 ELSE 1 END) AS pending
                FROM chunks c LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
                LEFT JOIN transcription_group_chunks m ON m.device_id=c.device_id AND m.chunk_id=c.id
                LEFT JOIN transcription_groups g ON g.device_id=m.device_id AND g.id=m.group_id
                WHERE c.device_id=? AND c.recording=? GROUP BY c.track""",
                (device, recording),
            ).fetchall()
            row = reader.execute(
                "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
        if not rows and row is None:
            raise HubError("recording_not_found", "Grabación no disponible para este dispositivo")
        manifest = (
            json.loads(self.store.decrypt(row["body"], f"manifest:{device}:{recording}"))
            if row
            else None
        )
        missing = sorted({"mic_local", "remote_app"} - {row["track"] for row in rows})
        pending = sum(row["pending"] for row in rows)
        return {
            "recording_id": recording,
            "pending_chunks": pending,
            "capture_finalized": manifest is not None,
            "capture_complete": manifest is not None and manifest["gaps"] == 0 and not missing,
            "transcript_ready": manifest is not None and pending == 0,
        }
