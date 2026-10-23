"""How a recording's transcript reads: turns with relative times and named voices.

One rule for every reader of a transcript: the Mac's routes, Deimos's meetings
bridge, the note the model writes and the context of a question asked about a
meeting. When the Mac rebuilt it on its own, it got the times (the chunks'
wall clock), the names («Hablante sin identificar») and the empty groups wrong;
nothing else here may decide them again.

`read` is the only part that touches the store, inside the caller's snapshot;
the rest is pure.
"""

import json
from collections import Counter
from dataclasses import dataclass

from . import dictionaries, repository
from .open_recordings import length_ms, open_capture

# How the local microphone is named before the Mac has said who owns it
# (`PUT /v1/device/profile`). It is the track, not a person.
LOCAL_TRACK_NAME = "Micrófono local"
# How the transcript names a voice nobody has named. The note never makes it
# the owner of a task: a task without a known person has no owner.
UNKNOWN_SPEAKER = "Otra persona"
# Consecutive groups of one voice that start within this window of the turn's
# first group are one turn. A group closes every 20 s of audio
# (`grouping.TARGET_SECONDS`), so a turn joins at most three groups: a minute of
# one person speaking reads as one paragraph, and a longer one breaks at the
# next minute so its time stays near what it says.
MERGE_WINDOW_MS = 60_000
# The tracks a recording can carry (`routes/captures.py`), in the order the
# speakers are listed: the Mac's microphone, then the meeting's audio.
TRACKS = ("mic_local", "remote_app")


@dataclass(frozen=True)
class Speaker:
    key: str
    track: str
    name: str


@dataclass(frozen=True)
class Segment:
    """One transcribed group with words in it, timed from the recording's start."""

    at_ms: int
    track: str
    speaker_key: str
    text: str
    first_index: int
    last_index: int
    language: str | None
    end_ms: int | None = None


@dataclass(frozen=True)
class Turn:
    at_ms: int
    speaker_key: str
    text: str


@dataclass(frozen=True)
class Line:
    """A turn with its speaker's name: what the model and Deimos read."""

    at_ms: int
    speaker: str
    text: str


def speaker_key(track: str, label: str | None) -> str:
    """The recognizer's label when a group has one, else its track.

    The live draft carries no speaker, so its groups say "unknown" and the key
    is the track; the final pass labels each separated voice `voice_N`.
    Renaming is keyed by this, so it never changes for a stored group.
    """
    if label and label != "unknown":
        return label
    return track


def speaker_name(key: str, track: str, renames: dict[str, str], owner: str | None) -> str:
    """The user's name for the voice; else the owner for the local microphone; else nobody."""
    if key in renames:
        return renames[key]
    if track == "mic_local":
        return owner or LOCAL_TRACK_NAME
    if key.startswith("voice_") and key[6:].isdigit():
        return f"Persona {int(key[6:])}"
    return UNKNOWN_SPEAKER


def merge_segments(segments: list[Segment]) -> list[Segment]:
    """One interval per turn, shared by the app, corrections and Deimos."""
    turns: list[Segment] = []
    for segment in sorted(segments, key=lambda s: (s.at_ms, s.track)):
        last = turns[-1] if turns else None
        if (
            last is not None
            and last.speaker_key == segment.speaker_key
            and last.track == segment.track
            and segment.at_ms - last.at_ms < MERGE_WINDOW_MS
        ):
            turns[-1] = Segment(
                last.at_ms,
                last.track,
                last.speaker_key,
                last.text + " " + segment.text,
                last.first_index,
                segment.last_index,
                last.language,
                max(last.end_ms or last.at_ms + 1, segment.end_ms or segment.at_ms + 1),
            )
        else:
            turns.append(segment)
    return turns


def merge(segments: list[Segment]) -> list[Turn]:
    return [Turn(s.at_ms, s.speaker_key, s.text) for s in merge_segments(segments)]


def language_of(segments: list[Segment]) -> str | None:
    """The most frequent language among the groups with words; the first heard on a tie."""
    counted = Counter(segment.language for segment in segments if segment.language)
    return counted.most_common(1)[0][0] if counted else None


def clock(ms: int) -> str:
    seconds = ms // 1000
    return f"{seconds // 3600:02d}:{seconds // 60 % 60:02d}:{seconds % 60:02d}"


def render(lines: list[Line]) -> str:
    """«[hh:mm:ss] Nombre: texto», one line per turn."""
    return "\n".join(f"[{clock(line.at_ms)}] {line.speaker}: {line.text}" for line in lines)


# Who says, in the transcript's lines, that a stretch of audio is missing:
# not a voice of the meeting.
MISSING_AUDIO_SPEAKER = "Orbit"
TRACK_WORDS = {"mic_local": "del micrófono", "remote_app": "de la reunión"}


@dataclass(frozen=True)
class MissingAudio:
    """Chunks of one track the hub will never get, as the Mac declared them.

    `at_ms` is where the hole starts from the meeting's start and
    `duration_ms` how long it is; each is None when the chunks around it do
    not say (unknown, never zero).
    """

    track: str
    first_sequence: int
    last_sequence: int
    at_ms: int | None
    duration_ms: int | None

    def line(self, fallback_ms: int) -> Line:
        span = (
            f" ({max(1, round(self.duration_ms / 1000))} s)" if self.duration_ms is not None else ""
        )
        return Line(
            self.at_ms if self.at_ms is not None else fallback_ms,
            MISSING_AUDIO_SPEAKER,
            " ".join(
                f"Falta el audio {TRACK_WORDS.get(self.track, '')} de este tramo{span}: "
                "no llegó al hub.".split()
            ),
        )


def _runs(sequences: list[int]) -> list[tuple[int, int]]:
    runs: list[tuple[int, int]] = []
    for sequence in sorted(sequences):
        if runs and runs[-1][1] == sequence - 1:
            runs[-1] = (runs[-1][0], sequence)
        else:
            runs.append((sequence, sequence))
    return runs


def missing_audio(store, db, device: str, recording: str, started: int | None):
    """The holes the closing manifest declared, with their span when known.

    A hole starts where the chunk before it ends and ends where the chunk
    after it starts; one at a track's start begins with the meeting, one at
    its end lasts until the closure.
    """
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return ()
    manifest = json.loads(store.decrypt(row["body"], f"manifest:{device}:{recording}"))

    def chunk_metadata(track: str, sequence: int) -> dict | None:
        found = db.execute(
            """SELECT c.id, j.metadata FROM chunks c
            LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
            WHERE c.device_id=? AND c.recording=? AND c.track=? AND c.sequence=?""",
            (device, recording, track, sequence),
        ).fetchone()
        if found is None or found["metadata"] is None:
            return None
        return json.loads(store.decrypt(found["metadata"], f"metadata:{device}:{found['id']}"))

    def wall(metadata: dict | None, *, end: bool) -> int | None:
        if metadata is None or type(metadata.get("started_at_ms")) is not int:
            return None
        if not end:
            return metadata["started_at_ms"]
        frames, rate = metadata.get("frames"), metadata.get("sample_rate")
        if type(frames) is not int or type(rate) is not int or rate <= 0:
            return None
        return metadata["started_at_ms"] + frames * 1000 // rate

    holes = []
    for track in manifest.get("tracks", ()):
        for first, last in _runs(list(track.get("missing_sequences") or ())):
            begins = (
                manifest.get("started_at_ms")
                if first == 0
                else wall(chunk_metadata(track["track"], first - 1), end=True)
            )
            ends = (
                manifest.get("finalized_at_ms")
                if last == track["last_sequence"]
                else wall(chunk_metadata(track["track"], last + 1), end=False)
            )
            known = type(begins) is int and type(started) is int
            holes.append(
                MissingAudio(
                    track=track["track"],
                    first_sequence=first,
                    last_sequence=last,
                    at_ms=max(0, begins - started) if known else None,
                    duration_ms=max(0, ends - begins)
                    if type(begins) is int and type(ends) is int
                    else None,
                )
            )
    return tuple(holes)


@dataclass(frozen=True)
class Transcript:
    started_at_ms: int | None
    segments: tuple[Segment, ...]
    turns: tuple[Turn, ...]
    speakers: tuple[Speaker, ...]
    # The recording's language: None only with nothing transcribed and no
    # configured language.
    language: str | None
    # Groups read, those whose text is empty included: what a note says it
    # was written from (`GeneratedNote.segments`).
    groups: int
    # Where the last group read ends, from the start: how long the
    # transcribed meeting is, for the note's length.
    audio_ms: int
    # The name the local microphone carries, whether or not it spoke: who
    # records the meeting, as the note's prompt calls them.
    local_name: str
    final: bool = False
    dictionary_terms: tuple[dict, ...] = ()
    # Stretches of audio the hub never got, declared by the Mac at closure.
    missing: tuple[MissingAudio, ...] = ()

    def correct(self, text: str) -> str:
        return dictionaries.substitute(text, self.dictionary_terms)

    def name(self, key: str) -> str:
        for speaker in self.speakers:
            if speaker.key == key:
                return speaker.name
        return UNKNOWN_SPEAKER

    def lines(self) -> list[Line]:
        """The turns, and where audio is missing a line that says so."""
        spoken = [Line(turn.at_ms, self.name(turn.speaker_key), turn.text) for turn in self.turns]
        holes = [hole.line(self.audio_ms) for hole in self.missing]
        return sorted(spoken + holes, key=lambda line: line.at_ms)


def started_at(store, db, device: str, recording: str) -> int | None:
    """The manifest's start; while the recording is open, its first chunk's.

    None when neither is known (chunks of an old Mac carry no wall clock):
    unknown, not zero, though every turn then reads from 0.
    """
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is not None:
        manifest = json.loads(store.decrypt(row["body"], f"manifest:{device}:{recording}"))
        if type(manifest.get("started_at_ms")) is int:
            return manifest["started_at_ms"]
    return open_capture(store, db, device, recording)["started_at_ms"]


def read(
    store, db, device: str, recording: str, *, default_language: str | None = None
) -> Transcript:
    """The recording's transcript as it reads now, in the caller's snapshot.

    `default_language` is the configured one (`[meeting_transcription]
    language`): what a group stored before the hub kept a language reads as.
    Only `Transcript.language` depends on it.

    Every group with a text is read: a group being transcribed again in
    another language keeps its old text until the new one arrives,
    so the meeting never reads emptier while it is repeated.
    """
    started = started_at(store, db, device, recording)
    rows = db.execute(
        """SELECT g.id, g.track, g.first_sequence, g.last_sequence, g.manifest, g.language,
        t.body FROM transcription_groups g
        JOIN group_transcripts t ON t.device_id=g.device_id AND t.group_id=g.id
        WHERE g.device_id=? AND g.recording=?
        ORDER BY g.first_sequence, g.track""",
        (device, recording),
    ).fetchall()
    segments, keys, end = [], {}, 0
    for row in rows:
        group = json.loads(store.decrypt(row["manifest"], f"group:{device}:{row['id']}"))
        body = json.loads(store.decrypt(row["body"], f"group-transcript:{device}:{row['id']}"))
        wall = group.get("started_at_ms")
        at_ms = max(0, wall - started) if type(wall) is int and type(started) is int else 0
        end = max(end, at_ms + length_ms(group))
        label = group.get("speaker")
        key = speaker_key(row["track"], label if isinstance(label, str) else None)
        keys.setdefault(key, row["track"])
        text = " ".join(str(body.get("text") or "").split())
        if not text:
            continue
        language = row["language"]
        segments.append(
            Segment(
                at_ms=at_ms,
                track=row["track"],
                speaker_key=key,
                text=text,
                first_index=row["first_sequence"],
                last_index=row["last_sequence"],
                language=language if isinstance(language, str) else default_language,
                end_ms=at_ms + length_ms(group),
            )
        )
    final = db.execute(
        "SELECT body FROM recording_final_transcripts WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if final is not None:
        result = repository._open(
            store, final["body"], f"recording-final-transcript:{device}:{recording}"
        )
        segments, keys = [], {}
        end = result["audio_ms"]
        chosen = repository.language(db, device, recording)
        for word in result["words"]:
            key, track = word["voice"], word["track"]
            keys.setdefault(key, track)
            text = " ".join(word["text"].split())
            if text:
                segments.append(
                    Segment(
                        word["start_ms"],
                        track,
                        key,
                        text,
                        0,
                        0,
                        chosen[0] if chosen else result.get("language") or default_language,
                        word["end_ms"],
                    )
                )
    changes = repository.speaker_changes(store, db, device, recording)

    changed = []
    for segment in segments:
        key = repository.canonical_speaker_key(changes, segment.speaker_key)
        for override in changes["turns"]:
            if (
                segment.track == override["track"]
                and override["start_ms"] <= segment.at_ms < override["end_ms"]
            ):
                key = repository.canonical_speaker_key(changes, override["speaker_key"])
        changed.append(
            Segment(
                segment.at_ms,
                segment.track,
                key,
                segment.text,
                segment.first_index,
                segment.last_index,
                segment.language,
                segment.end_ms,
            )
        )
    segments = changed
    keys = {repository.canonical_speaker_key(changes, key): track for key, track in keys.items()}
    for segment in segments:
        keys.setdefault(segment.speaker_key, segment.track)
    tracks = {
        item["track"]
        for item in db.execute(
            "SELECT DISTINCT track FROM chunks WHERE device_id=? AND recording=?",
            (device, recording),
        ).fetchall()
    }
    for track in tracks:
        if final is None or track == "mic_local":
            keys.setdefault(track, track)
    renames = changes["names"]
    owner = repository.owner_name(store, db, device)
    ordered = sorted(
        keys.items(),
        key=lambda item: (
            item[0] not in TRACKS,
            TRACKS.index(item[0]) if item[0] in TRACKS else 0,
            item[0],
        ),
    )
    speakers = tuple(
        Speaker(key, track, speaker_name(key, track, renames, owner)) for key, track in ordered
    )
    in_order = sorted(segments, key=lambda s: (s.at_ms, s.track))
    chosen = repository.language(db, device, recording)
    vocabularies, _ = dictionaries.resolve(store, db, device, recording)
    terms = dictionaries.combined(vocabularies)
    return Transcript(
        started_at_ms=started,
        segments=tuple(in_order),
        turns=tuple(
            Turn(t.at_ms, t.speaker_key, dictionaries.substitute(t.text, terms))
            for t in merge(in_order)
        ),
        dictionary_terms=tuple(terms),
        speakers=speakers,
        # The user's choice; else what the groups are in; else the configured one.
        language=(chosen[0] if chosen else None) or language_of(in_order) or default_language,
        groups=len(rows),
        audio_ms=end,
        local_name=speaker_name("mic_local", "mic_local", renames, owner),
        final=final is not None,
        missing=missing_audio(store, db, device, recording, started),
    )
