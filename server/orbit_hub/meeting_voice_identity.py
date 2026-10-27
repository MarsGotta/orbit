"""Calendar bounds and stable user names when a requested pass reclusters voices."""

from bisect import bisect_right
from collections import Counter, defaultdict

from .meetings import muted, repository


def speaker_bound(store, db, device: str, recording: str) -> int | None:
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return None
    manifest = repository._open(store, row["body"], f"manifest:{device}:{recording}")
    attendees = (manifest.get("event") or {}).get("attendees", [])
    if not attendees:
        return None
    # Bound remote clustering by invited people; the owner's separate microphone
    # is excluded. Invitations are a ceiling, never evidence of attendance or
    # identity. The minimum one admits remote speech in an owner-only event.
    # This does not limit microphone speakers, audio duration or transcript size.
    return max(1, sum(not attendee.get("is_self", False) for attendee in attendees))


def _overlap(left: dict, right: dict) -> int:
    return max(0, min(left["end_ms"], right["end_ms"]) - max(left["start_ms"], right["start_ms"]))


def _assignments(old: dict, spans: list[dict]) -> dict[str, dict[str, int]]:
    """Each old word votes for its most overlapping new voice. No model labels
    are identities. Only voices with temporal evidence may inherit old names.
    """
    weights = defaultdict(Counter)
    spans = sorted(spans, key=lambda span: span["start_ms"])
    ends = [span["end_ms"] for span in spans]
    for word in old["words"]:
        if word["track"] != "remote_app" or not spans:
            continue
        # Whisper can timestamp a real word as a point, including at the start
        # of an interval. Treat it as a point inside the half-open voice span.
        probe = {**word, "end_ms": max(word["end_ms"], word["start_ms"] + 1)}
        index = bisect_right(ends, word["start_ms"])
        candidates = []
        while index < len(spans) and spans[index]["start_ms"] < probe["end_ms"]:
            candidates.append(spans[index])
            index += 1
        if candidates:
            target = max(candidates, key=lambda span: _overlap(probe, span))
            weights[target["voice"]][word["voice"]] += len(word["text"].split())
    # A split old voice belongs to the new voice with most of its words.
    owners = {}
    for target, candidates in weights.items():
        for key, count in candidates.items():
            if key not in owners or count > weights[owners[key]][key]:
                owners[key] = target
    return {
        target: {key: count for key, count in candidates.items() if owners[key] == target}
        for target, candidates in weights.items()
    }


def _reidentify(old: dict, result: dict, changes: dict) -> None:
    weights = _assignments(old, result["spans"])
    reserved = set(changes["names"]) | set(changes["same_as"])
    reserved.update(w["voice"] for w in old["words"])
    mapping = {}
    for span in result["spans"]:
        target = span["voice"]
        if target in mapping:
            continue
        candidates = weights.get(target, {})
        if candidates:
            # Keep a named identity first, then the one with most old words.
            # Equal counts use the key for a deterministic tie. Losing names
            # remain encrypted aliases and existing turn overrides still apply.
            winner = max(candidates, key=lambda k: (k in changes["names"], candidates[k], k))
            # Keep separate raw keys when only the user merged them. Collapsing
            # those intervals here would lose their correspondence on the next
            # retranscription and could bring back an explicitly merged voice.
            if len(candidates) > 1:
                winner = repository.canonical_speaker_key(changes, winner)
            mapping[target] = winner
            for key in candidates:
                if key != winner:
                    changes["same_as"][key] = winner
        else:
            index = 1
            while f"voice_{index}" in reserved:
                index += 1
            mapping[target] = f"voice_{index}"
            reserved.add(mapping[target])
    for item in [*result["spans"], *result["words"]]:
        if item.get("track", "remote_app") == "remote_app":
            item["voice"] = mapping.get(item["voice"], item["voice"])


def publish_voices(store, db, job: dict, result: dict) -> None:
    """Replace voices, names and text atomically; failed passes keep the old view.
    Names are read under the writer so an edit during inference is preserved.
    """
    device, recording = job["device_id"], job["recording"]
    old = db.execute(
        "SELECT body FROM recording_final_transcripts WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    changes_blob = None
    if old is not None and job.get("refresh_voices"):
        changes = repository.speaker_changes(store, db, device, recording)
        old_text = repository._open(
            store, old["body"], f"recording-final-transcript:{device}:{recording}"
        )
        _reidentify(old_text, result, changes)
        changes_blob = repository._seal(store, changes, f"recording-speakers:{device}:{recording}")
    voices = repository._seal(
        store, {"spans": result["spans"]}, f"recording-voices:{device}:{recording}"
    )
    text = {key: result[key] for key in ("words", "audio_ms", "language")}
    text["words"] = muted.heard(job.get("muted", ()), text["words"])
    transcript = repository._seal(store, text, f"recording-final-transcript:{device}:{recording}")
    store.ensure_capacity(len(voices) + len(transcript) + len(changes_blob or b""))
    conflict = "DO UPDATE SET body=excluded.body" if job.get("refresh_voices") else "DO NOTHING"
    db.execute(
        "INSERT INTO recording_voices VALUES (?,?,?) ON CONFLICT(device_id,recording) " + conflict,
        (device, recording, voices),
    )
    db.execute(
        "INSERT INTO recording_final_transcripts VALUES (?,?,?) "
        "ON CONFLICT(device_id,recording) DO UPDATE SET body=excluded.body",
        (device, recording, transcript),
    )
    if changes_blob is not None:
        db.execute(
            "INSERT INTO recording_speakers VALUES (?,?,?,0) ON CONFLICT(device_id,recording) "
            "DO UPDATE SET body=excluded.body",
            (device, recording, changes_blob),
        )
