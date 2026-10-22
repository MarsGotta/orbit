"""Bounded reads of a paired device's meetings.

The assistant sees the meetings unless its agent has the `meetings` access
group off (Settings › Agents, `assistant.access`): a configuration of that
agent's tools, not a sharing record. The «Reuniones» switch was retired on
2026-09-25, so nothing here reads a sharing record; the private routes are
authenticated and the device is the one the request names.
An existing ``meeting_sharing`` table, written by earlier versions, is left
where it is and never read; so is ``deimos_meeting_notifications``, from the
notice Deimos got when a transcript finished, retired on 2026-10-15.
"""

import json
from time import monotonic

from ..meetings import repository as meetings
from ..meetings import turns, views
from ..meetings.note_state import progress
from ..meetings.open_recordings import pending_audio
from ..notes import NoteRepository
from ..private_auth import Authenticator, read_key
from ..transcription_failure import durable_details
from .models import (
    HEADER_PREFIX,
    LIST_PATH,
    NOTE_PATH,
    PROTOCOL,
    TRANSCRIPT_PATH,
    ListRequest,
    MeetingsBridgeConfiguration,
    MeetingsBridgeError,
    NoteRequest,
    TranscriptRequest,
)
from .page import cut

# Errors reading a stored manifest or transcript: a damaged row is the hub's
# fault, answered as `unavailable` without its content.
_DAMAGED = (KeyError, ValueError, TypeError, UnicodeError)

# How long a note in progress takes, for the advice given to Deimos: the worst
# measured note (4 h of meeting) took 44 s on qwen3.8-flash-next.
NOTE_WRITE_MS = 45_000
# A transcript group closes every 20 s of audio (grouping.TARGET_SECONDS).
TRANSCRIPT_GROUP_MS = 20_000


class MeetingsBridgeService:
    def __init__(
        self, store, configuration, now, *, default_language: str | None = None, access=None
    ):
        self.store, self.now = store, now
        # `assistant.access.AgentAccess`: every read here is the `meetings`
        # group's, refused for an agent that has it off. None checks nothing.
        self.access = access
        # `[meeting_transcription] language`, for groups stored without one.
        self.default_language = default_language
        self.configuration = (
            MeetingsBridgeConfiguration.model_validate(configuration)
            if configuration is not None
            else None
        )
        self.list_auth = self.transcript_auth = self.note_auth = None
        # Read only: the notes worker writes them. Created here too so that a
        # bridge read never meets a missing table on a hub that has none yet.
        self.notes = NoteRepository(store)
        self.notes.initialize()

    def start(self) -> None:
        if self.configuration is None:
            return
        key = read_key(self.configuration.credential_file, MeetingsBridgeError)
        common = {
            "key_id": self.configuration.key_id,
            "key": key,
            "protocol": PROTOCOL,
            "header_prefix": HEADER_PREFIX,
            "failure": MeetingsBridgeError,
        }
        self.list_auth = Authenticator(path=LIST_PATH, **common)
        self.transcript_auth = Authenticator(path=TRANSCRIPT_PATH, **common)
        self.note_auth = Authenticator(path=NOTE_PATH, **common)

    def close(self) -> None:
        self.list_auth = self.transcript_auth = self.note_auth = None

    def list_meetings(self, headers, body: bytes) -> dict:
        """Newest first by start, the open ones included, `limit` at a time.

        `more` says whether older meetings in the window were left for the next
        call, which continues with `before_ms` set to the last one's start.
        """
        if self.list_auth is None:
            raise MeetingsBridgeError(503, "unavailable")
        self.list_auth.admit(headers, body, int(self.now() * 1000), monotonic())
        try:
            request = ListRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise MeetingsBridgeError(400, "invalid_request") from None
        if self.access is not None:
            self.access.require(request.agent_id, "meetings")
        device = request.device_id
        with self.store.read_snapshot() as db:
            try:
                page = views.listing(
                    self.store,
                    self.notes,
                    db,
                    device,
                    request.before_ms,
                    request.limit,
                    since_ms=request.since_ms,
                )
            except _DAMAGED:
                raise MeetingsBridgeError(503, "unavailable") from None
            # Deimos's contract has no `none`: nothing to transcribe yet is pending.
            for entry in page["meetings"]:
                if entry["transcript"] == "none":
                    entry["transcript"] = "pending"
        return page

    def note(self, headers, body: bytes) -> dict:
        """The note as the user sees it, or its state with the numbers that explain it."""
        if self.note_auth is None:
            raise MeetingsBridgeError(503, "unavailable")
        self.note_auth.admit(headers, body, int(self.now() * 1000), monotonic())
        try:
            request = NoteRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise MeetingsBridgeError(400, "invalid_request") from None
        if self.access is not None:
            self.access.require(request.agent_id, "meetings")
        device, recording = request.device_id, request.recording_id
        with self.store.read_snapshot() as db:
            is_open = self._recording(db, device, recording) is None
            document = views.note_document(self.notes, db, device, recording)
            if document is None:
                raise MeetingsBridgeError(404, "note_not_requested")
            owner = meetings.owner_name(self.store, db, device)
            private, _revision = meetings.private_notes(self.store, db, device, recording)
            numbers = progress(document)
            if document["note"] is None and document["state"] == "waiting_transcript":
                numbers["capture"] = "open" if is_open else "closed"
                numbers["pending_chunks"] = pending_audio(self.store, db, device, recording)[
                    "pending_chunks"
                ]
        if document["note"] is None:
            self._refuse_note(document["state"], numbers)
        return {
            "recording_id": recording,
            "state": document["state"],
            "revision": document["revision"],
            "progress": numbers,
            "note": document["note"],
            "edited": document["edited"],
            "owner_name": owner,
            "private_notes": private,
            "superseded_edits": document["superseded_edits"] is not None,
            "updated_at_ms": document["updated_at_ms"],
        }

    def _refuse_note(self, state: str, numbers: dict) -> None:
        """A note not written yet: its state's own code, with its numbers.

        A note in progress is not a refusal to retry never: it says when asking
        again is worth it. Writing a note measured 23–44 s for meetings of one
        to four hours and a transcript group
        closes every 20 s of audio, so those are the waits Deimos is given.
        """
        if state == "failed":
            raise MeetingsBridgeError(503, "note_failed", numbers)
        if state == "queued":
            next_ms = numbers.get("retry_at_ms")
            wait = next_ms - int(self.now() * 1000) if isinstance(next_ms, int) else None
            numbers["retry_after_ms"] = max(1000, wait) if wait is not None else NOTE_WRITE_MS
            raise MeetingsBridgeError(503, "note_queued", numbers)
        if state == "generating":
            numbers["retry_after_ms"] = NOTE_WRITE_MS
            raise MeetingsBridgeError(503, "note_generating", numbers)
        if state == "waiting_transcript":
            numbers["retry_after_ms"] = TRANSCRIPT_GROUP_MS
            raise MeetingsBridgeError(503, "note_waiting_transcript", numbers)
        if state == "not_configured":
            raise MeetingsBridgeError(503, "note_not_configured", numbers)
        raise MeetingsBridgeError(503, "unavailable")

    def _recording(self, db, device: str, recording: str):
        """The final manifest row, None while the recording is open; 404 if unknown."""
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        if row is None and (
            db.execute(
                "SELECT 1 FROM chunks WHERE device_id=? AND recording=? LIMIT 1",
                (device, recording),
            ).fetchone()
            is None
        ):
            raise MeetingsBridgeError(404, "recording_not_found")
        return row

    def transcript(self, headers, body: bytes) -> dict:
        """The transcript with its speakers named, a page at a time.

        A recording still open answers with what is transcribed so far and how
        much audio the hub holds untranscribed. The turns and their names are
        the ones the Mac shows (`meetings.turns`): the user's names for the voices,
        the device owner's for the local microphone, and the answer says when
        the Mac has not said who that is yet.
        """
        if self.transcript_auth is None:
            raise MeetingsBridgeError(503, "unavailable")
        self.transcript_auth.admit(headers, body, int(self.now() * 1000), monotonic())
        try:
            request = TranscriptRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise MeetingsBridgeError(400, "invalid_request") from None
        if self.access is not None:
            self.access.require(request.agent_id, "meetings")
        device, recording = request.device_id, request.recording_id
        live = None
        with self.store.read_snapshot() as db:
            final = self._recording(db, device, recording)
            try:
                transcript = turns.read(
                    self.store, db, device, recording, default_language=self.default_language
                )
            except _DAMAGED:
                raise MeetingsBridgeError(503, "unavailable") from None
            failed_rows = views.failed_groups(db, device, recording)
            incomplete_groups = []
            for failed in failed_rows:
                details = durable_details(failed["error_code"])
                incomplete_groups.append(
                    {
                        "track": failed["track"],
                        "first_index": failed["first_sequence"],
                        "last_index": failed["last_sequence"],
                        **details,
                        "attempts": failed["attempts"],
                    }
                )
            if final is None:
                live = pending_audio(self.store, db, device, recording)
            else:
                self._refuse_unfinished(db, device, recording, transcript, failed_rows)
            named = meetings.owner_name(self.store, db, device) is not None
            from ..meeting_final_pass import status

            final_pass = status(db, device, recording)
        text = self._render(transcript, request.format)
        if request.after_index > len(text):
            raise MeetingsBridgeError(
                400,
                "after_index_out_of_range",
                {"after_index": request.after_index, "total_chars": len(text)},
            )
        page = cut(text[request.after_index :])
        end = request.after_index + len(page)
        next_index = end if end < len(text) else None
        answer = {
            "final_pass": final_pass,
            "text": page,
            "after_index": request.after_index,
            "next_after_index": next_index,
            "total_chars": len(text),
            "incomplete_groups": incomplete_groups,
            "capture": "closed" if live is None else "open",
            "speaker_names": "profile" if named else "tracks",
        }
        if live is not None:
            answer.update(live)
        return answer

    def _refuse_unfinished(self, db, device, recording, transcript, failed_rows) -> None:
        """A closed recording whose transcript has no text yet says why."""
        if failed_rows and not transcript.segments:
            details = durable_details(failed_rows[0]["error_code"])
            details["attempts"] = failed_rows[0]["attempts"]
            raise MeetingsBridgeError(503, "transcription_failed", details)
        if views.transcript_state(db, device, recording) not in ("pending", "none"):
            return
        active = db.execute(
            """SELECT state,error_code,attempts,next_attempt FROM transcription_groups
            WHERE device_id=? AND recording=? AND state IN ('pending','running')
            ORDER BY first_sequence LIMIT 1""",
            (device, recording),
        ).fetchone()
        if active is None:
            raise MeetingsBridgeError(
                503, "transcription_pending", {"cause": "transcription_group_not_ready"}
            )
        details = durable_details(active["error_code"])
        if active["error_code"] is None:
            details["cause"] = (
                "transcription_in_progress"
                if active["state"] == "running"
                else "transcription_queued"
            )
        details["attempts"] = active["attempts"]
        details["phase"] = active["state"]
        if active["state"] == "pending":
            details["retry_at_ms"] = int(active["next_attempt"] * 1000)
        raise MeetingsBridgeError(503, "transcription_pending", details)

    @staticmethod
    def _render(transcript: turns.Transcript, format: str) -> str:
        """`text`: one line per turn, «[hh:mm:ss] Nombre: texto», as the Mac and
        the note's model read it. `segments`: one JSON object per transcribed
        group per line. A long line can span pages; offsets count characters
        of the rendered text in both formats."""
        if format == "text":
            return turns.render(transcript.lines())
        return "\n".join(
            json.dumps(
                {
                    "at_ms": segment.at_ms,
                    **(
                        {"end_ms": segment.end_ms, "speaker_key": segment.speaker_key}
                        if transcript.final
                        else {}
                    ),
                    "track": segment.track,
                    "speaker": transcript.name(segment.speaker_key),
                    "first_index": segment.first_index,
                    "last_index": segment.last_index,
                    "text": transcript.correct(segment.text),
                },
                ensure_ascii=False,
                separators=(",", ":"),
            )
            for segment in (
                turns.merge_segments(list(transcript.segments))
                if transcript.final
                else transcript.segments
            )
        )
