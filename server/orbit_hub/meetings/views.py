"""The device's views of its meetings: the list, one recording's document, its turns.

Read inside the caller's snapshot. The list's rows are shared with Deimos's
meetings bridge, which adds its own numbers to them.
"""

import json
from datetime import UTC, datetime

from ..meeting_final_pass import status as final_status
from ..notes.view import as_seen
from ..protocol import MAX_RECORDING_TITLE_CHARACTERS, HubError
from ..transcription_failure import durable_details
from . import dictionaries, repository, turns
from .note_state import progress
from .open_recordings import open_capture, open_recordings, pending_audio, voice_note

# Meetings per page of `GET /v1/recordings`. The Mac's Notes list shows them a
# screen at a time and asks for the next page from the last one's start; 50
# rows are a few KiB, so a page is cheap while a long history never travels
# in one answer. Not a limit on how many meetings are kept.
MAX_LIST_PAGE = 50
# Bytes of turns per page of `GET /v1/recordings/{id}/turns`, as JSON. The
# daemon reads at most 1 MiB per hub answer (`MAX_HUB_HTTP_BODY_BYTES`); a
# quarter of it is about two hours of meeting per page, so a long meeting
# arrives in a few requests and a page never nears that gate. A page always
# carries one turn, so it never stalls; a turn joins at most three groups of
# 20 s (`turns.MERGE_WINDOW_MS`), a few KiB.
MAX_TURN_PAGE_BYTES = 256 * 1024
# Sort key for an open recording whose chunks carry no wall-clock start: after
# every real start, so it is listed first, with the meetings in progress.
_UNKNOWN_START = 2**63


def _manifest(store, device: str, recording: str, body: bytes) -> dict:
    value = json.loads(store.decrypt(body, f"manifest:{device}:{recording}"))
    if not isinstance(value, dict):
        raise TypeError("A manifest is an object")
    return value


def title(manifest: dict, started_at_ms: int) -> str:
    named = manifest.get("title")
    if isinstance(named, str) and named.strip():
        return named.strip()[:MAX_RECORDING_TITLE_CHARACTERS]
    date = datetime.fromtimestamp(started_at_ms / 1000, tz=UTC).strftime("%d/%m/%Y %H:%M")
    return f"Reunión del {date}"


def named(store, db, device: str, recording: str, own: str) -> str:
    """The user's name for the meeting, else its own title."""
    return repository.title(store, db, device, recording) or own


def open_title(notes, db, device: str, recording: str, started: int | None) -> str:
    """An open recording has no manifest yet: the title its note was asked with, if any."""
    inputs = notes.inputs(device, recording, db=db)
    if inputs is not None:
        planned = inputs.title or (inputs.event.title if inputs.event else "")
        if planned.strip():
            return planned.strip()[:MAX_RECORDING_TITLE_CHARACTERS]
    if started is None:
        return "Reunión en curso"
    return title({}, started)


def rows(store, notes, db, device: str) -> list[dict]:
    """Every meeting of the device, closed and open, newest first by start.

    A voice call (`kind: voice_call`) is not a meeting and is left out, and
    so is a voice note an earlier Mac uploaded and never closed
    (`capture.voice_note`). Open rows carry when their last audio arrived, so
    a reader can tell a meeting in progress from one a Mac never closed.
    """
    entries = []
    for row in db.execute(
        "SELECT recording, body FROM recording_manifests WHERE device_id=?", (device,)
    ).fetchall():
        manifest = _manifest(store, device, row["recording"], row["body"])
        started, finalized = manifest.get("started_at_ms"), manifest.get("finalized_at_ms")
        if type(started) is not int or type(finalized) is not int:
            continue
        if manifest.get("kind", "meeting") != "meeting":
            continue
        entries.append(
            {
                "recording_id": row["recording"],
                "title": named(store, db, device, row["recording"], title(manifest, started)),
                "started_at_ms": started,
                "duration_ms": max(0, finalized - started),
                "capture": "closed",
            }
        )
    for recording in open_recordings(db, device):
        if voice_note(store, db, device, recording):
            continue
        capture = open_capture(store, db, device, recording)
        entries.append(
            {
                "recording_id": recording,
                "title": named(
                    store,
                    db,
                    device,
                    recording,
                    open_title(notes, db, device, recording, capture["started_at_ms"]),
                ),
                "started_at_ms": capture["started_at_ms"],
                "duration_ms": capture["audio_ms"],
                "capture": "open",
                "last_audio_received_ms": capture["last_audio_received_ms"],
            }
        )
    entries.sort(
        key=lambda entry: (
            -(_UNKNOWN_START if entry["started_at_ms"] is None else entry["started_at_ms"]),
            entry["recording_id"],
        )
    )
    return entries


def before(entry: dict, before_ms: int | None, before_id: str | None = None) -> bool:
    """In the page that continues after the row (`before_ms`, `before_id`).

    Rows are ordered newest first and, at the same start, by id (`rows`), so
    the page goes on with what started earlier and with what started at that
    same instant under a later id: two meetings that start together on a
    page's edge are both listed. Without `before_id` (Deimos's bridge) only
    what started earlier. An open one with no start comes only first.
    """
    started = entry["started_at_ms"]
    if started is None:
        return before_ms is None
    if before_ms is None or started < before_ms:
        return True
    return started == before_ms and before_id is not None and entry["recording_id"] > before_id


def transcript_state(db, device: str, recording: str) -> str:
    """none, pending, partial, failed or ready.

    `none`: no audio to transcribe. `partial`: some groups failed for good and
    others have text. Pending work wins over both: it may still change them.
    """
    final = final_status(db, device, recording)
    if final is not None:
        if final["state"] == "completed":
            return "ready"
        if final["state"] in {"queued", "running"}:
            return "pending"
    states = [
        row["state"]
        for row in db.execute(
            "SELECT state FROM transcription_groups WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchall()
    ]
    pending = db.execute(
        """SELECT count(*) AS count FROM chunks c
        LEFT JOIN transcription_group_chunks m
        ON m.device_id=c.device_id AND m.chunk_id=c.id
        LEFT JOIN transcription_jobs j
        ON j.device_id=c.device_id AND j.chunk_id=c.id
        WHERE c.device_id=? AND c.recording=? AND m.chunk_id IS NULL
        AND coalesce(j.state,'pending') NOT IN ('completed','failed')""",
        (device, recording),
    ).fetchone()["count"]
    if pending or any(state in {"pending", "running"} for state in states):
        return "pending"
    completed = sum(state == "completed" for state in states)
    failed = sum(state == "failed" for state in states)
    if completed and failed:
        return "partial"
    if failed:
        return "failed"
    return "ready" if completed else "none"


def device_transcript_state(db, device: str, recording: str) -> str:
    """The Mac's four: a transcript missing a stretch for good is `failed`.

    The document's `failed_groups` say which stretch and why; a `partial` one
    still shows its text.
    """
    state = transcript_state(db, device, recording)
    return "failed" if state == "partial" else state


def note_document(notes, db, device: str, recording: str) -> dict | None:
    try:
        document = notes.document(device, recording, db=db)
    except HubError:
        return None
    seen, edited = None, {"sections": False}
    if document["note"] is not None:
        seen, edited = as_seen(document["note"], document["edits"])
    return {**document, "note": seen, "edited": edited}


def note_summary(notes, db, device: str, recording: str) -> dict:
    document = note_document(notes, db, device, recording)
    if document is None:
        return {"state": "not_requested"}
    summary = {"state": document["state"], **progress(document)}
    if document["note"] is not None:
        summary.update(
            title=document["note"]["title"],
            short_summary=document["note"]["short_summary"],
            revision=document["revision"],
        )
    return summary


def listing(
    store,
    notes,
    db,
    device: str,
    before_ms: int | None,
    limit: int,
    before_id: str | None = None,
    *,
    since_ms: int | None = None,
) -> dict:
    entries = [
        entry
        for entry in rows(store, notes, db, device)
        if before(entry, before_ms, before_id)
        and (
            since_ms is None or entry["started_at_ms"] is None or entry["started_at_ms"] >= since_ms
        )
    ]
    for entry in entries[:limit]:
        entry["transcript"] = transcript_state(db, device, entry["recording_id"])
        entry["note"] = note_summary(notes, db, device, entry["recording_id"])
    return {"meetings": entries[:limit], "more": len(entries) > limit}


def device_listing(store, notes, db, device, before_ms, limit, before_id=None) -> dict:
    page = listing(store, notes, db, device, before_ms, limit, before_id)
    for entry in page["meetings"]:
        entry.pop("last_audio_received_ms", None)
        if entry["transcript"] == "partial":
            entry["transcript"] = "failed"
        note = entry["note"]
        entry["note"] = (
            None
            if note["state"] == "not_requested"
            else {"state": note["state"], "short_summary": note.get("short_summary")}
        )
    return page


def _note(notes, db, device: str, recording: str) -> dict | None:
    document = note_document(notes, db, device, recording)
    if document is None:
        return None
    return {
        "state": document["state"],
        "revision": document["revision"],
        "attempts": document["attempts"],
        "next_attempt_ms": document["next_attempt_ms"],
        "error": document["error"],
        "template_id": document["template_id"],
        "note": document["note"],
        "edited": {"sections": document["edited"]["sections"]},
        "superseded_available": document["superseded_edits"] is not None,
        "updated_at_ms": document["updated_at_ms"],
    }


def speakers(transcript: turns.Transcript) -> list[dict]:
    return [
        {"key": speaker.key, "track": speaker.track, "name": speaker.name}
        for speaker in transcript.speakers
    ]


def heading(store, notes, db, device: str, recording: str) -> dict:
    """A recording's title, start, length and whether it is still open."""
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is not None:
        manifest = _manifest(store, device, recording, row["body"])
        started = manifest["started_at_ms"]
        return {
            "title": named(store, db, device, recording, title(manifest, started)),
            "started_at_ms": started,
            "duration_ms": max(0, manifest["finalized_at_ms"] - started),
            "capture": "closed",
        }
    capture = open_capture(store, db, device, recording)
    return {
        "title": named(
            store,
            db,
            device,
            recording,
            open_title(notes, db, device, recording, capture["started_at_ms"]),
        ),
        "started_at_ms": capture["started_at_ms"],
        "duration_ms": capture["audio_ms"],
        "capture": "open",
    }


def failed_groups(db, device: str, recording: str):
    final = final_status(db, device, recording)
    if final is not None and final["state"] == "completed":
        return []
    return db.execute(
        """SELECT track,first_sequence,last_sequence,error_code,attempts
        FROM transcription_groups WHERE device_id=? AND recording=? AND state='failed'
        ORDER BY first_sequence,track""",
        (device, recording),
    ).fetchall()


def document(store, notes, db, device: str, recording: str, *, default_language) -> dict:
    """One recording as the Mac shows it."""
    repository.require(db, device, recording)
    head = heading(store, notes, db, device, recording)
    transcript = turns.read(store, db, device, recording, default_language=default_language)
    failed = [
        {
            "track": failure["track"],
            "cause": durable_details(failure["error_code"])["cause"],
            "attempts": failure["attempts"],
        }
        for failure in failed_groups(db, device, recording)
    ]
    text, revision = repository.private_notes(store, db, device, recording)
    progress = {
        "state": device_transcript_state(db, device, recording),
        "pending_chunks": pending_audio(store, db, device, recording)["pending_chunks"],
        "failed_groups": failed,
    }
    final = final_status(db, device, recording)
    if final is not None:
        progress["final_pass"] = final
        if final["state"] == "completed":
            progress["pending_chunks"] = 0
    live_keys = {speaker.key for speaker in transcript.speakers}
    changes = repository.speaker_changes(store, db, device, recording)
    # A missing microphone in a remote-only meeting is not a lost voice.
    unmatched = sum(
        key != "mic_local" and repository.canonical_speaker_key(changes, key) not in live_keys
        for key in changes["names"]
    )
    if unmatched:
        progress["unmatched_speaker_names"] = unmatched
    if transcript.missing:
        # Audio the hub will never get, where and how long (null: unknown).
        progress["missing_audio"] = [
            {
                "track": hole.track,
                "at_ms": hole.at_ms,
                "duration_ms": hole.duration_ms,
                "chunks": hole.last_sequence - hole.first_sequence + 1,
            }
            for hole in transcript.missing
        ]
    if transcript.muted:
        # «Silenciar»: where the microphone was muted, from the meeting's start.
        progress["muted_microphone"] = [
            {"start_ms": span.start_ms, "end_ms": span.end_ms} for span in transcript.muted
        ]
    repeating = retranscribing(db, device, recording)
    if repeating is not None:
        progress["retranscribing"] = repeating
    return {
        "recording_id": recording,
        "dictionary": dictionaries.resolve(store, db, device, recording)[1],
        **head,
        "language": transcript.language,
        "speakers": speakers(transcript),
        "transcript": progress,
        "note": _note(notes, db, device, recording),
        "private_notes": {"text": text, "revision": revision},
    }


def retranscribing(db, device: str, recording: str) -> dict | None:
    """While a change of language repeats the meeting, how far it got.

    `total` are the groups the change sent again; `done` those of them back
    with their new text. None once none is left, and for a meeting never
    changed.
    """
    chosen = repository.language(db, device, recording)
    if chosen is None:
        return None
    left = repository.repeating(db, device, recording)
    if not left:
        return None
    language, total = chosen
    return {"language": language, "done": max(0, total - left), "total": total}


def turns_page(transcript: turns.Transcript, offset: int) -> dict:
    total = len(transcript.turns)
    if offset > total:
        raise HubError(
            "invalid_parameters",
            f"offset fuera de rango: {offset}, la grabación tiene {total} turnos",
            numbers={"field": "offset", "actual": offset, "max": total},
        )
    page, used = [], 0
    for turn in transcript.turns[offset:]:
        item = {"at_ms": turn.at_ms, "speaker_key": turn.speaker_key, "text": turn.text}
        size = len(json.dumps(item, ensure_ascii=False, separators=(",", ":")).encode()) + 1
        if page and used + size > MAX_TURN_PAGE_BYTES:
            break
        page.append(item)
        used += size
    following = offset + len(page)
    return {
        "turns": page,
        "next_offset": following if following < total else None,
        "total": total,
    }
