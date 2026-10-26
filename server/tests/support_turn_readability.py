"""Numbers-only comparison of the old interleaving and current turn reader.

From the server directory (with tests on sys.path), call
`support_turn_readability.compare_recording(store, device, recording)`.
It reads a snapshot, writes nothing and returns only numeric measurements.
"""

from dataclasses import replace
from itertools import pairwise
from statistics import median

from orbit_hub.meetings import turns


def legacy_turns(segments):
    """Freeze the former global-consecutive rule for before/after measurement."""
    result = []
    for segment in sorted(segments, key=lambda s: (s.at_ms, s.track)):
        last = result[-1] if result else None
        if (
            last is not None
            and last.track == segment.track
            and last.speaker_key == segment.speaker_key
            and segment.at_ms - last.at_ms < turns.MERGE_WINDOW_MS
        ):
            result[-1] = replace(last, text=last.text + " " + segment.text)
        else:
            result.append(segment)
    return result


def measurements(items, audio_ms):
    counts = [len(item.text.split()) for item in items]
    changes = sum(a.speaker_key != b.speaker_key for a, b in pairwise(items))
    return {
        "turn_count": len(items),
        "short_turn_count": sum(1 <= count <= 2 for count in counts),
        "median_words_per_turn": median(counts) if counts else 0,
        "speaker_changes_per_minute": changes * 60_000 / audio_ms if audio_ms else 0,
    }


def compare_recording(store, device, recording):
    """Return before/after counts for one recording, with current edits in both.

    Speaker changes use turn start order and transcribed audio duration. Empty
    recordings return zero rates. Neither transcript text nor names are returned.
    """
    with store.read_snapshot() as db:
        transcript = turns.read(store, db, device, recording)
    before = [
        replace(item, text=transcript.correct(item.text))
        for item in legacy_turns(transcript.segments)
    ]
    return {
        "before": measurements(before, transcript.audio_ms),
        "after": measurements(transcript.turns, transcript.audio_ms),
    }
