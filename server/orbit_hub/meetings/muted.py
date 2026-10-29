"""Mute: where the Mac muted the microphone, as its closure declared it.

The Mac sent silence for these stretches (same frames, same timing), so the
microphone track holds no words there. The transcript says each one in a
calm line, and the final pass neither counts it as lost words nor keeps what
a model heard in that silence. Times are milliseconds from the recording's
start, the clock every transcript line uses.
"""

import json
from dataclasses import dataclass

# Who says, in the transcript's lines, that the microphone was muted: not a
# voice of the meeting (as for missing audio).
SPEAKER = "Orbit"


@dataclass(frozen=True)
class MutedSpan:
    start_ms: int
    end_ms: int

    def covers(self, at_ms: int) -> bool:
        return self.start_ms <= at_ms < self.end_ms

    def holds(self, start_ms: int, end_ms: int) -> bool:
        return self.start_ms <= start_ms and end_ms <= self.end_ms

    def text(self) -> str:
        return f"Microphone muted · {_mmss(self.start_ms)} – {_mmss(self.end_ms)}"


def _mmss(ms: int) -> str:
    seconds = max(0, ms) // 1000
    return f"{seconds // 60:02d}:{seconds % 60:02d}"


def of_manifest(manifest: dict) -> tuple[MutedSpan, ...]:
    """The spans a closing manifest declared; none from a Mac before them."""
    return tuple(
        MutedSpan(span["start_ms"], span["end_ms"]) for span in manifest.get("muted_spans") or ()
    )


def read(store, db, device: str, recording: str) -> tuple[MutedSpan, ...]:
    """The spans of a closed recording; none while it is open."""
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return ()
    return of_manifest(json.loads(store.decrypt(row["body"], f"manifest:{device}:{recording}")))


def heard(spans, words):
    """The final words, less the microphone's that start in a muted stretch:
    what the silence the Mac sent made a model hear is not speech."""
    return [
        word
        for word in words
        if word["track"] != "mic_local" or not any(span.covers(word["start_ms"]) for span in spans)
    ]


def spoken(spans, groups):
    """The draft groups `(start_ms, end_ms, text)` not wholly inside a muted
    stretch: those can be compared with the final words."""
    return [group for group in groups if not any(span.holds(group[0], group[1]) for span in spans)]
