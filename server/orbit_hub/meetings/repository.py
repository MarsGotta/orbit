"""What the user adds to a recording on the hub: their name, their voices' names,
their notes, the meeting's name and the language it is transcribed in.

Encrypted like every other piece of a meeting, scoped by device, and read
inside the caller's snapshot (`db`). The writes take the store's writer.
"""

import json
import unicodedata

from ..protocol import MAX_RECORDING_TITLE_CHARACTERS, HubError
from .bounds import MAX_PERSON_NAME_CHARS

SCHEMA = """
CREATE TABLE IF NOT EXISTS recording_final_passes (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    state TEXT NOT NULL, error TEXT, progress TEXT, kept_draft TEXT,
    refresh_voices INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(device_id,recording));
CREATE TABLE IF NOT EXISTS recording_final_transcripts (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    body BLOB NOT NULL, PRIMARY KEY(device_id,recording));
CREATE TABLE IF NOT EXISTS recording_voices (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    body BLOB NOT NULL, PRIMARY KEY(device_id,recording));
CREATE TABLE IF NOT EXISTS device_profiles (
    device_id TEXT PRIMARY KEY REFERENCES devices(id), body BLOB NOT NULL, updated REAL NOT NULL);
CREATE TABLE IF NOT EXISTS recording_speakers (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    body BLOB NOT NULL, updated REAL NOT NULL, PRIMARY KEY(device_id, recording));
CREATE TABLE IF NOT EXISTS recording_private_notes (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    body BLOB NOT NULL, revision INTEGER NOT NULL, updated REAL NOT NULL,
    PRIMARY KEY(device_id, recording));
CREATE TABLE IF NOT EXISTS recording_titles (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    body BLOB NOT NULL, updated REAL NOT NULL, PRIMARY KEY(device_id, recording));
CREATE TABLE IF NOT EXISTS recording_languages (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    language TEXT NOT NULL, repeated INTEGER NOT NULL, updated REAL NOT NULL,
    PRIMARY KEY(device_id, recording));
"""


def exists(db, device: str, recording: str) -> bool:
    """The hub holds this device's recording: a chunk of it or its final manifest."""
    return (
        db.execute(
            "SELECT 1 FROM chunks WHERE device_id=? AND recording=? LIMIT 1", (device, recording)
        ).fetchone()
        is not None
        or db.execute(
            "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        is not None
    )


def require(db, device: str, recording: str) -> None:
    if not exists(db, device, recording):
        raise HubError("recording_not_found", "The recording is not on the hub")


def person_name(value: str, field: str) -> str:
    """A name as it will be shown and sent to the model: one line, trimmed, bounded."""
    name = " ".join(value.split())
    if not name or any(unicodedata.category(character) == "Cc" for character in value.strip()):
        raise HubError(
            "invalid_parameters",
            f"Invalid name in {field}: empty or containing control characters",
            numbers={"field": field, "min": 1, "max": MAX_PERSON_NAME_CHARS},
        )
    if len(name) > MAX_PERSON_NAME_CHARS:
        raise HubError(
            "invalid_parameters",
            f"Name too long in {field}: {len(name)} characters, maximum {MAX_PERSON_NAME_CHARS}",
            numbers={"field": field, "actual": len(name), "max": MAX_PERSON_NAME_CHARS},
        )
    return name


def _open(store, blob, context: str):
    return json.loads(store.decrypt(blob, context))


def _seal(store, value, context: str) -> bytes:
    payload = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    return store.encrypt(payload, context)


# The owner of the device: how the local microphone is named.


def owner_name(store, db, device: str) -> str | None:
    row = db.execute("SELECT body FROM device_profiles WHERE device_id=?", (device,)).fetchone()
    if row is None:
        return None
    return _open(store, row["body"], f"device-profile:{device}")["owner_name"]


def set_owner_name(store, device: str, name: str, now: float) -> str:
    name = person_name(name, "owner_name")
    sealed = _seal(store, {"owner_name": name}, f"device-profile:{device}")
    with store.transaction() as db:
        store.ensure_capacity(len(sealed))
        db.execute(
            "INSERT INTO device_profiles(device_id, body, updated) VALUES (?,?,?) "
            "ON CONFLICT(device_id) DO UPDATE SET body=excluded.body, updated=excluded.updated",
            (device, sealed, now),
        )
    return name


# The names the user gave the voices of one recording, by speaker key.


def speaker_changes(store, db, device: str, recording: str) -> dict:
    row = db.execute(
        "SELECT body FROM recording_speakers WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return {"names": {}, "same_as": {}, "turns": []}
    value = _open(store, row["body"], f"recording-speakers:{device}:{recording}")
    # Old encrypted rows are a flat key -> name map, migrated lazily on write.
    return (
        value
        if "names" in value and isinstance(value["names"], dict)
        else {"names": value, "same_as": {}, "turns": []}
    )


def canonical_speaker_key(changes: dict, key: str) -> str:
    """Follow the user's explicit voice merges, stopping at a cycle."""
    seen = set()
    while key in changes["same_as"] and key not in seen:
        seen.add(key)
        key = changes["same_as"][key]
    return key


def rename(
    store,
    device: str,
    recording: str,
    names: dict[str, str | None],
    keys: set[str],
    now: float,
    *,
    turn=None,
) -> None:
    """Apply `names` over the stored ones; None returns a voice to its default name.

    `keys` are the recording's speakers: a name for any other key is refused,
    because nothing would ever show it.
    """
    for key in names:
        if turn is None and key not in keys:
            raise HubError(
                "unknown_speaker",
                f"The recording has no such speaker; it has {len(keys)}",
                numbers={"field": "names", "speakers": len(keys)},
            )
    clean = {
        key: None if name is None else person_name(name, f"names.{key}")
        for key, name in names.items()
    }
    with store.transaction() as db:
        require(db, device, recording)
        from . import turns

        # Read and validate inside the write transaction: the final result may
        # replace a draft between the HTTP snapshot and this write.
        transcript = turns.read(store, db, device, recording)
        current = speaker_changes(store, db, device, recording)
        if turn is not None:
            _name_turn(current, transcript, turn, clean)
        else:
            _name_voices(current, transcript, clean)
        sealed = _seal(store, current, f"recording-speakers:{device}:{recording}")
        store.ensure_capacity(len(sealed))
        db.execute(
            "INSERT INTO recording_speakers(device_id, recording, body, updated) VALUES (?,?,?,?) "
            "ON CONFLICT(device_id, recording) DO UPDATE SET body=excluded.body, "
            "updated=excluded.updated",
            (device, recording, sealed, now),
        )


def _name_turn(current: dict, transcript, turn, clean: dict) -> None:
    """Name one turn alone: its voice joins a same-named one of its track, or
    becomes a voice of its own; without a name it returns to its voice's."""
    from . import turns

    selected = next(
        (
            item
            for item in turns.merge_segments(list(transcript.segments))
            if item.at_ms == turn.at_ms and item.speaker_key == turn.speaker_key
        ),
        None,
    )
    if selected is None:
        raise HubError(
            "turn_no_longer_exists",
            "The turn is no longer in the transcript",
            http_status=409,
            numbers={"at_ms": turn.at_ms, "turns": len(transcript.turns)},
        )
    if len(clean) != 1:
        raise HubError(
            "invalid_parameters",
            "A turn allows only one name",
            numbers={"names": len(clean), "max": 1},
        )
    track = selected.track
    end = selected.end_ms or selected.at_ms + 1
    target, name = next(iter(clean.items()))
    identity = f"turn_{track}_{selected.at_ms}_{end}"
    current["turns"] = [
        item
        for item in current["turns"]
        if not (
            item["track"] == track and item["start_ms"] == selected.at_ms and item["end_ms"] == end
        )
    ]
    if name is not None:
        existing = next(
            (s.key for s in transcript.speakers if s.name == name and s.track == track),
            None,
        )
        target = existing or identity
        current["names"][target] = name
        current["turns"].append(
            {"track": track, "start_ms": selected.at_ms, "end_ms": end, "speaker_key": target}
        )


def _name_voices(current: dict, transcript, clean: dict) -> None:
    """Name whole voices: a name another voice of the track has merges them."""
    live_keys = {s.key for s in transcript.speakers}
    if not set(clean) <= live_keys:
        raise HubError(
            "unknown_speaker",
            "The speaker is no longer in the transcript",
            numbers={"speakers": len(live_keys)},
        )
    for key, name in clean.items():
        if name is None:
            current["names"].pop(key, None)
            current["same_as"].pop(key, None)
        else:
            track = next(s.track for s in transcript.speakers if s.key == key)
            existing = next(
                (
                    s.key
                    for s in transcript.speakers
                    if s.key != key and s.name == name and s.track == track
                ),
                None,
            )
            if existing is not None:
                current["same_as"][key] = existing
            current["names"][key] = name


# The user's private notes of one recording, with the revision they were saved at.


def private_notes(store, db, device: str, recording: str) -> tuple[str, int]:
    """The text and its revision; ("", 0) when nothing was saved."""
    row = db.execute(
        "SELECT body, revision FROM recording_private_notes WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return "", 0
    text = _open(store, row["body"], f"recording-private-notes:{device}:{recording}")["text"]
    return text, row["revision"]


def save_private_notes(
    store, device: str, recording: str, text: str, base_revision: int, now: float
) -> int:
    """Save over `base_revision`, or refuse with the current one: never overwrite blind.

    The route has bounded `text` (`notes.models.MAX_PRIVATE_NOTES_CHARS`).
    """
    sealed = _seal(store, {"text": text}, f"recording-private-notes:{device}:{recording}")
    with store.transaction() as db:
        require(db, device, recording)
        row = db.execute(
            "SELECT revision FROM recording_private_notes WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()
        current = 0 if row is None else row["revision"]
        if current != base_revision:
            raise HubError(
                "stale_revision",
                f"Private notes changed: revision {current}, the change started from {base_revision}",
                numbers={"revision": current, "base_revision": base_revision},
            )
        store.ensure_capacity(len(sealed))
        db.execute(
            "INSERT INTO recording_private_notes(device_id, recording, body, revision, updated) "
            "VALUES (?,?,?,?,?) ON CONFLICT(device_id, recording) DO UPDATE SET "
            "body=excluded.body, revision=excluded.revision, updated=excluded.updated",
            (device, recording, sealed, current + 1, now),
        )
    return current + 1


# The name the user gave a meeting. It wins over the manifest's title
# wherever the meeting is named; the note already written keeps its own.


def meeting_title(value: str) -> str | None:
    """A meeting's name as it is shown: one line, trimmed; None when empty."""
    if any(unicodedata.category(character) == "Cc" for character in value):
        raise HubError(
            "invalid_parameters",
            "Invalid title: it contains control characters",
            numbers={"field": "title", "max": MAX_RECORDING_TITLE_CHARACTERS},
        )
    title = " ".join(value.split())
    if len(title) > MAX_RECORDING_TITLE_CHARACTERS:
        raise HubError(
            "invalid_parameters",
            f"Title too long: {len(title)} characters, maximum {MAX_RECORDING_TITLE_CHARACTERS}",
            numbers={
                "field": "title",
                "actual": len(title),
                "max": MAX_RECORDING_TITLE_CHARACTERS,
            },
        )
    return title or None


def title(store, db, device: str, recording: str) -> str | None:
    row = db.execute(
        "SELECT body FROM recording_titles WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return None
    return _open(store, row["body"], f"recording-title:{device}:{recording}")["title"]


def set_title(store, device: str, recording: str, value: str, now: float) -> None:
    """Keep the user's name for the meeting; an empty one gives it back its own."""
    name = meeting_title(value)
    with store.transaction() as db:
        require(db, device, recording)
        if name is None:
            db.execute(
                "DELETE FROM recording_titles WHERE device_id=? AND recording=?",
                (device, recording),
            )
            return
        sealed = _seal(store, {"title": name}, f"recording-title:{device}:{recording}")
        store.ensure_capacity(len(sealed))
        db.execute(
            "INSERT INTO recording_titles(device_id, recording, body, updated) VALUES (?,?,?,?) "
            "ON CONFLICT(device_id, recording) DO UPDATE SET body=excluded.body, "
            "updated=excluded.updated",
            (device, recording, sealed, now),
        )


# The language a recording is transcribed in, once the user chose one.
# Until then it is the configured one (`[meeting_transcription] language`).
# A language code, not content: kept in clear like each group's.


def language(db, device: str, recording: str) -> tuple[str, int] | None:
    """The language the user chose and how many groups that choice repeated."""
    row = db.execute(
        "SELECT language, repeated FROM recording_languages WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    return None if row is None else (row["language"], row["repeated"])


def repeating(db, device: str, recording: str) -> int:
    """Groups with a text that are being transcribed again (a new language).

    A group transcribed for the first time has no text yet; one being
    repeated keeps its old text until the new one replaces it.
    """
    return db.execute(
        """SELECT count(*) AS count FROM transcription_groups g
        JOIN group_transcripts t ON t.device_id=g.device_id AND t.group_id=g.id
        WHERE g.device_id=? AND g.recording=? AND g.state IN ('pending','running')""",
        (device, recording),
    ).fetchone()["count"]


def change_language(
    store, device: str, recording: str, chosen: str, default: str | None, now: float
) -> int:
    """Transcribe the recording in `chosen` from now on, and again what it already has.

    Returns how many groups are transcribed again: those with a text in
    another language, and those already being repeated. A group not
    transcribed yet goes in `chosen` when its turn comes; one being
    transcribed in the old language is put back, so its answer is refused as
    stale and it goes again. The same language repeats nothing.

    The audio of every group is the hub's to read again: no route deletes a
    chunk, and a group's chunks cannot go while the group refers to them
    (foreign keys are on). What the hub can lack is audio it cannot read: a
    group failed for good with `invalid_group_audio`. Then nothing changes,
    and the refusal says how many of how many.
    """
    with store.transaction() as db:
        require(db, device, recording)
        total = db.execute(
            "SELECT count(*) AS count FROM transcription_groups WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchone()["count"]
        unreadable = db.execute(
            "SELECT count(*) AS count FROM transcription_groups WHERE device_id=? AND recording=? "
            "AND state='failed' AND error_code='invalid_group_audio'",
            (device, recording),
        ).fetchone()["count"]
        if unreadable:
            raise HubError(
                "meeting_audio_unreadable",
                f"The language was not changed: the hub cannot read audio for {unreadable} of {total} groups of this meeting",
                http_status=409,
                numbers={"groups": unreadable, "total": total, "cause": "invalid_group_audio"},
            )
        stored = language(db, device, recording)
        previous = stored[0] if stored is not None else default
        if previous != chosen:
            # Sent in the old language: back in the queue, attempts kept, so
            # its late answer no longer matches (`GroupRepository.finish`).
            db.execute(
                "UPDATE transcription_groups SET state='pending', next_attempt=? "
                "WHERE device_id=? AND recording=? AND state='running'",
                (now, device, recording),
            )
        db.execute(
            """UPDATE transcription_groups SET state='pending', attempts=0, error_code=NULL,
            next_attempt=? WHERE device_id=? AND recording=? AND state IN ('completed','failed')
            AND coalesce(language, ?) IS NOT ? AND EXISTS (
                SELECT 1 FROM group_transcripts t
                WHERE t.device_id=transcription_groups.device_id
                AND t.group_id=transcription_groups.id)""",
            (now, device, recording, default, chosen),
        )
        if previous != chosen:
            # Voice intervals stay immutable; only the final text is repeated.
            db.execute(
                "UPDATE recording_final_passes SET state='queued',error=NULL "
                "WHERE device_id=? AND recording=?",
                (device, recording),
            )
        repeated = repeating(db, device, recording)
        db.execute(
            "INSERT INTO recording_languages(device_id, recording, language, repeated, updated) "
            "VALUES (?,?,?,?,?) ON CONFLICT(device_id, recording) DO UPDATE SET "
            "language=excluded.language, repeated=excluded.repeated, updated=excluded.updated",
            (device, recording, chosen, repeated, now),
        )
    return repeated
