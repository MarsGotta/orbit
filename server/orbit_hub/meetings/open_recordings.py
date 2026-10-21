"""What the hub knows of a recording the Mac has not closed yet.

A recording is open while the hub holds its chunks and no final manifest. Open
is the hub's fact, not a claim that someone is still talking: a Mac that went
offline mid-meeting, or one that never closed a recording, leaves it open too.
That is why every open recording carries when its last audio arrived, so the
reader can tell a meeting in progress from one left unclosed.

Everything here reads inside the caller's snapshot and decrypts only chunk
metadata (timing), never audio or text.
"""

import json

# A recording the hub holds: one probe per distinct recording through the
# chunks' (device_id, recording, track, sequence) index, instead of a scan of
# every chunk the device ever uploaded.
_RECORDINGS = """
WITH RECURSIVE seen(recording) AS (
    SELECT min(recording) FROM chunks WHERE device_id=:device
    UNION ALL
    SELECT (SELECT min(recording) FROM chunks WHERE device_id=:device AND recording>seen.recording)
    FROM seen WHERE seen.recording IS NOT NULL
)
SELECT recording FROM seen
WHERE recording IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM recording_manifests m WHERE m.device_id=:device AND m.recording=seen.recording)
"""


def open_recordings(db, device: str) -> list[str]:
    return [row["recording"] for row in db.execute(_RECORDINGS, {"device": device}).fetchall()]


def _metadata(store, device: str, chunk_id: str, blob) -> dict:
    if blob is None:
        return {}
    value = json.loads(store.decrypt(blob, f"metadata:{device}:{chunk_id}"))
    return value if isinstance(value, dict) else {}


def length_ms(metadata: dict) -> int:
    """How long a chunk or a group lasts, from its frames and rate; 0 when unknown."""
    frames, rate = metadata.get("frames"), metadata.get("sample_rate")
    if type(frames) is int and type(rate) is int and rate > 0 and frames >= 0:
        return frames * 1000 // rate
    return 0


def _edge(db, device: str, recording: str, track: str, order: str):
    # `order` is one of two literals written below, never input.
    return db.execute(
        f"""SELECT c.id, j.metadata, r.received FROM chunks c
        LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
        LEFT JOIN chunk_receipts r ON r.device_id=c.device_id AND r.chunk_id=c.id
        WHERE c.device_id=? AND c.recording=? AND c.track=?
        ORDER BY c.sequence {order} LIMIT 1""",
        (device, recording, track),
    ).fetchone()


# A voice note an earlier Mac uploaded (`recording_kind: voice_turn`) is one
# microphone chunk holding the whole note, and it is never closed. A meeting's
# chunk is sealed as soon as it holds a second of audio, and one capture frame
# holds at most another (`orbit-daemon/src/capture.rs`), so it never holds
# two. Two seconds or more in the first microphone chunk of an open recording
# with no other chunk is therefore a voice note, never a meeting. A note
# shorter than that cannot be told from a meeting's first second and stays
# listed. The Mac no longer uploads voice notes.
VOICE_NOTE_MIN_SECONDS = 2


def voice_note(store, db, device: str, recording: str) -> bool:
    """An open recording that is an earlier Mac's voice note, not a meeting."""
    rows = db.execute(
        """SELECT c.id, c.track, c.sequence, j.metadata FROM chunks c
        LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
        WHERE c.device_id=? AND c.recording=? LIMIT 2""",
        (device, recording),
    ).fetchall()
    if len(rows) != 1 or rows[0]["track"] != "mic_local" or rows[0]["sequence"] != 0:
        return False
    metadata = _metadata(store, device, rows[0]["id"], rows[0]["metadata"])
    frames, rate = metadata.get("frames"), metadata.get("sample_rate")
    return (
        type(frames) is int
        and type(rate) is int
        and rate > 0
        and frames >= VOICE_NOTE_MIN_SECONDS * rate
    )


def open_capture(store, db, device: str, recording: str) -> dict:
    """When it started, how much audio arrived and when the last of it did.

    `started_at_ms` is None when no chunk carries a wall-clock start (an old
    Mac's chunks): unknown, not zero.
    """
    tracks = [
        row["track"]
        for row in db.execute(
            "SELECT DISTINCT track FROM chunks WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchall()
    ]
    starts, ends, received = [], [], []
    for track in tracks:
        first = _edge(db, device, recording, track, "ASC")
        last = _edge(db, device, recording, track, "DESC")
        opening = _metadata(store, device, first["id"], first["metadata"])
        closing = _metadata(store, device, last["id"], last["metadata"])
        if type(opening.get("started_at_ms")) is int:
            starts.append(opening["started_at_ms"])
        if type(closing.get("started_at_ms")) is int:
            ends.append(closing["started_at_ms"] + length_ms(closing))
        if last["received"] is not None:
            received.append(int(last["received"] * 1000))
    started = min(starts) if starts else None
    return {
        "started_at_ms": started,
        "audio_ms": max(0, max(ends) - started) if started is not None and ends else 0,
        "last_audio_received_ms": max(received) if received else None,
    }


def pending_audio(store, db, device: str, recording: str) -> dict:
    """Audio the hub already holds and has not transcribed yet.

    A chunk is pending while it has no group, or its group is waiting or being
    transcribed. A failed group is not pending: it is reported apart, with its
    cause. The two tracks run side by side, so the meeting time still missing
    is the larger of the two tracks, not their sum. Audio the Mac has not
    uploaded is not here, and the answer says so.
    """
    rows = db.execute(
        """SELECT c.id, c.track, j.metadata FROM chunks c
        JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
        LEFT JOIN transcription_group_chunks m ON m.device_id=c.device_id AND m.chunk_id=c.id
        LEFT JOIN transcription_groups g ON g.device_id=m.device_id AND g.id=m.group_id
        WHERE c.device_id=? AND c.recording=?
        AND ((m.group_id IS NULL AND j.state NOT IN ('completed','failed'))
             OR g.state IN ('pending','running'))""",
        (device, recording),
    ).fetchall()
    per_track: dict[str, int] = {}
    for row in rows:
        length = length_ms(_metadata(store, device, row["id"], row["metadata"]))
        per_track[row["track"]] = per_track.get(row["track"], 0) + length
    return {
        "pending_chunks": len(rows),
        "pending_audio_ms": max(per_track.values(), default=0),
    }
