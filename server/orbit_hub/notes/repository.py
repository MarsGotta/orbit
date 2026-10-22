"""Durable notes of recordings, encrypted at rest, scoped by device.

One row per recording. `state` says where the note is; `revision` grows on
every change the user or the model makes, so an edit based on an older version is
refused instead of silently overwriting it.
"""

import json
from contextlib import nullcontext

from ..meeting_final_pass import note_ready
from ..meetings import repository as meetings
from ..protocol import HubError
from .custom_templates import SCHEMA as TEMPLATES_SCHEMA
from .models import EditRequest, EventContext, GeneratedNote, NoteEdits, NoteInputs

SCHEMA = """
CREATE TABLE IF NOT EXISTS recording_notes (
    device_id TEXT NOT NULL, recording TEXT NOT NULL,
    state TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0, next_attempt REAL NOT NULL DEFAULT 0,
    error TEXT, inputs BLOB NOT NULL, note BLOB, edits BLOB, superseded BLOB,
    requested REAL NOT NULL, updated REAL NOT NULL,
    PRIMARY KEY(device_id, recording));
CREATE INDEX IF NOT EXISTS recording_notes_work ON recording_notes(state, next_attempt);
"""

WORK_STATES = ("waiting_transcript", "queued")
# A note waits for the last transcript group; the transcription worker signals
# each finished group, and this delay only bounds how often a note whose
# transcript is not complete is looked at again.
WAIT_TRANSCRIPT_SECONDS = 10
# Due notes looked at per claim. A note whose transcript is not complete is
# put back without being claimed, so one claim may pass over several; eight
# keeps one claim's snapshot short while a ready note never waits behind more
# than a few unfinished ones (the rest are due again on the next tick).
CLAIM_CANDIDATES = 8


class NoteRepository:
    """Shares Store's writer and encryption; never owns another connection."""

    def __init__(self, store):
        self.store = store

    def initialize(self) -> None:
        """The notes' tables and their custom templates."""
        with self.store.transaction() as db:
            for schema in (SCHEMA, TEMPLATES_SCHEMA):
                for statement in filter(None, (s.strip() for s in schema.split(";"))):
                    db.execute(statement)

    def recover_interrupted(self, now: float) -> int:
        """A note being written when the hub stopped is written again."""
        with self.store.transaction() as db:
            return db.execute(
                "UPDATE recording_notes SET state='queued', next_attempt=?, updated=? "
                "WHERE state='generating'",
                (now, now),
            ).rowcount

    def requeue_unconfigured(self, now: float) -> int:
        """A model is configured now: notes asked for without one are written.

        The user's edits and inputs stay; only the state moves, and the note waits
        for its transcript like a new request.
        """
        with self.store.transaction() as db:
            return db.execute(
                "UPDATE recording_notes SET state='waiting_transcript', next_attempt=?, "
                "attempts=0, error=NULL, revision=revision+1, updated=? "
                "WHERE state='not_configured'",
                (now, now),
            ).rowcount

    def _seal(self, kind: str, device: str, recording: str, value) -> bytes:
        payload = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
        return self.store.encrypt(payload, f"note-{kind}:{device}:{recording}")

    def _open(self, kind: str, row, device: str, recording: str):
        blob = row[kind]
        if blob is None:
            return None
        return json.loads(self.store.decrypt(blob, f"note-{kind}:{device}:{recording}"))

    def planned(self, db, device: str, recording: str, template_id: str | None) -> NoteInputs:
        """What the note is asked with: the template, and the meeting's title and event.

        Both come from the recording's final manifest; a recording
        still open has neither yet.
        """
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        manifest = (
            json.loads(self.store.decrypt(row["body"], f"manifest:{device}:{recording}"))
            if row is not None
            else {}
        )
        event = manifest.get("event")
        return NoteInputs(
            template_id=template_id,
            title=manifest.get("title") or "",
            event=EventContext.model_validate(event) if isinstance(event, dict) else None,
        )

    def request(
        self,
        device: str,
        recording: str,
        now: float,
        configured: bool,
        *,
        template_id: str | None = None,
        regenerate: bool = True,
    ) -> dict:
        """Ask for the note of a recording.

        `regenerate` writes it again over any note there is (another template,
        or "write it again"); without it, as at finalize, a note already asked
        for is left as it is.
        """
        state = "waiting_transcript" if configured else "not_configured"
        with self.store.transaction() as db:
            meetings.require(db, device, recording)
            row = db.execute(
                "SELECT * FROM recording_notes WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if row is not None and not regenerate:
                return self.document(device, recording, db=db)
            inputs = self._seal(
                "inputs",
                device,
                recording,
                self.planned(db, device, recording, template_id).model_dump(mode="json"),
            )
            self.store.ensure_capacity(len(inputs))
            if row is None:
                db.execute(
                    "INSERT INTO recording_notes(device_id,recording,state,revision,inputs,"
                    "requested,updated) VALUES (?,?,?,0,?,?,?)",
                    (device, recording, state, inputs, now, now),
                )
            else:
                edits = row["edits"]
                db.execute(
                    "UPDATE recording_notes SET state=?, attempts=0, next_attempt=0, error=NULL, "
                    "inputs=?, superseded=coalesce(?, superseded), edits=?, revision=revision+1, "
                    "updated=? WHERE device_id=? AND recording=?",
                    (
                        state,
                        inputs,
                        self._keep_text_edits(row, device, recording) if edits else None,
                        self._only_done(row, device, recording) if edits else None,
                        now,
                        device,
                        recording,
                    ),
                )
        return self.document(device, recording)

    def _keep_text_edits(self, row, device: str, recording: str) -> bytes | None:
        edits = self._open("edits", row, device, recording) or {}
        if edits.get("title") is None and edits.get("sections") is None:
            return None
        kept = {"title": edits.get("title"), "sections": edits.get("sections")}
        # Corrections an earlier regeneration set aside stay, newest first
        # under `earlier`: a second regeneration must not drop them. The
        # latest set stays on top, where the Mac restores it from. Not
        # bounded: each set is one person's corrections, made by hand.
        before = self._open("superseded", row, device, recording)
        if before is not None:
            earlier = before.pop("earlier", [])
            kept["earlier"] = [before, *earlier]
        return self._seal("superseded", device, recording, kept)

    def _only_done(self, row, device: str, recording: str) -> bytes | None:
        """Done marks follow a regenerated task with the same text; text edits do not."""
        edits = self._open("edits", row, device, recording) or {}
        if not edits.get("done"):
            return None
        return self._seal("edits", device, recording, {"done": edits["done"]})

    def wake_waiting(self) -> None:
        """A transcript changed: reconsider readiness on the event, not its safety tick."""
        with self.store.transaction() as db:
            db.execute("UPDATE recording_notes SET next_attempt=0 WHERE state='waiting_transcript'")

    def claim(self, now: float) -> dict | None:
        """The next due note whose transcript is complete, now `generating`.

        A due note still waiting for its transcript is not claimed: it stays
        `waiting_transcript` and is looked at again in
        `WAIT_TRANSCRIPT_SECONDS`. Claiming it would show it as being written
        while it only waits, and flip it back a moment later.
        """
        with self.store.transaction() as db:
            candidates = db.execute(
                "SELECT device_id, recording, state FROM recording_notes "
                "WHERE state IN (?,?) AND next_attempt<=? ORDER BY next_attempt, requested LIMIT ?",
                (*WORK_STATES, now, CLAIM_CANDIDATES),
            ).fetchall()
            row = None
            for candidate in candidates:
                try:
                    ready = note_ready(
                        self.store, db, candidate["device_id"], candidate["recording"]
                    )
                except HubError:
                    # The recording is gone: claimed, so that writing it fails
                    # the note with its cause instead of skipping it for ever.
                    ready = True
                if ready:
                    row = candidate
                    break
                db.execute(
                    "UPDATE recording_notes SET next_attempt=? WHERE device_id=? AND recording=?",
                    (now + WAIT_TRANSCRIPT_SECONDS, candidate["device_id"], candidate["recording"]),
                )
            if row is None:
                return None
            db.execute(
                "UPDATE recording_notes SET state='generating', updated=? "
                "WHERE device_id=? AND recording=?",
                (now, row["device_id"], row["recording"]),
            )
            full = db.execute(
                "SELECT * FROM recording_notes WHERE device_id=? AND recording=?",
                (row["device_id"], row["recording"]),
            ).fetchone()
        device, recording = full["device_id"], full["recording"]
        return {
            "device_id": device,
            "recording": recording,
            "attempts": full["attempts"],
            "revision": full["revision"],
            "inputs": NoteInputs.model_validate(self._open("inputs", full, device, recording)),
        }

    def wait_transcript(self, job: dict, retry_at: float, now: float) -> None:
        self._set_state(job, "waiting_transcript", retry_at, now)

    def _set_state(self, job: dict, state: str, retry_at: float, now: float) -> None:
        with self.store.transaction() as db:
            db.execute(
                "UPDATE recording_notes SET state=?, next_attempt=?, updated=? "
                "WHERE device_id=? AND recording=? AND state='generating'",
                (state, retry_at, now, job["device_id"], job["recording"]),
            )

    def complete(self, job: dict, note: GeneratedNote, now: float) -> None:
        device, recording = job["device_id"], job["recording"]
        sealed = self._seal("note", device, recording, note.model_dump(mode="json"))
        self.store.ensure_capacity(len(sealed))
        with self.store.transaction() as db:
            row = db.execute(
                "SELECT edits FROM recording_notes WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            edits = None
            if row is not None and row["edits"] is not None:
                current = self._open("edits", row, device, recording) or {}
                ids = {item.id for item in note.action_items}
                done = {k: v for k, v in (current.get("done") or {}).items() if k in ids}
                current["done"] = done
                edits = self._seal("edits", device, recording, current)
            db.execute(
                "UPDATE recording_notes SET state='ready', note=?, edits=?, error=NULL, "
                "attempts=attempts+1, revision=revision+1, updated=? "
                "WHERE device_id=? AND recording=? AND state='generating'",
                (sealed, edits, now, device, recording),
            )

    def fail(self, job: dict, error: dict, now: float, retry_at: float | None) -> None:
        with self.store.transaction() as db:
            db.execute(
                "UPDATE recording_notes SET state=?, next_attempt=?, error=?, "
                "attempts=attempts+1, updated=? "
                "WHERE device_id=? AND recording=? AND state='generating'",
                (
                    "queued" if retry_at is not None else "failed",
                    retry_at or 0,
                    json.dumps(error, separators=(",", ":")),
                    now,
                    job["device_id"],
                    job["recording"],
                ),
            )

    def edit(self, device: str, recording: str, request: EditRequest, now: float) -> dict:
        with self.store.transaction() as db:
            row = db.execute(
                "SELECT * FROM recording_notes WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if row is None or row["note"] is None:
                raise HubError("note_not_ready", "La nota todavía no está escrita")
            if row["revision"] != request.base_revision:
                raise HubError(
                    "note_revision_changed",
                    f"La nota cambió: revisión {row['revision']}, la edición partía de "
                    f"{request.base_revision}",
                )
            current = NoteEdits.model_validate(self._open("edits", row, device, recording) or {})
            merged = NoteEdits(
                title=request.title if request.title is not None else current.title,
                sections=request.sections if request.sections is not None else current.sections,
                done={**current.done, **request.done},
            )
            sealed = self._seal("edits", device, recording, merged.model_dump(mode="json"))
            db.execute(
                "UPDATE recording_notes SET edits=?, revision=revision+1, updated=? "
                "WHERE device_id=? AND recording=?",
                (sealed, now, device, recording),
            )
        return self.document(device, recording)

    def document(self, device: str, recording: str, *, db=None) -> dict:
        reader = db if db is not None else self.store._db
        with self.store._lock if db is None else nullcontext():
            row = reader.execute(
                "SELECT * FROM recording_notes WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
        if row is None:
            raise HubError(
                "note_not_requested", "Nadie ha pedido todavía la nota de esta grabación"
            )
        inputs = self._open("inputs", row, device, recording) or {}
        return {
            "recording_id": recording,
            "state": row["state"],
            "revision": row["revision"],
            "attempts": row["attempts"],
            "next_attempt_ms": int(row["next_attempt"] * 1000)
            if row["state"] in WORK_STATES
            else None,
            "error": json.loads(row["error"]) if row["error"] else None,
            "template_id": inputs.get("template_id"),
            "note": self._open("note", row, device, recording),
            "edits": self._open("edits", row, device, recording),
            "superseded_edits": self._open("superseded", row, device, recording),
            "updated_at_ms": int(row["updated"] * 1000),
        }

    def inputs(self, device: str, recording: str, *, db) -> NoteInputs | None:
        """What the note was asked with, read in the caller's snapshot.

        An open recording has no manifest yet, so its listed title comes from
        here (`meetings.views.open_title`). None means nobody asked for the note.
        """
        row = db.execute(
            "SELECT inputs FROM recording_notes WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        if row is None:
            return None
        return NoteInputs.model_validate(self._open("inputs", row, device, recording))
