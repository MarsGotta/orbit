"""Read back only the isolated fixture recording; return numbers, never audio."""

from __future__ import annotations

import hashlib
import io
import json
import wave


def inspect_recording(store, recording):
    with store.read_snapshot() as db:
        closed = db.execute(
            "SELECT device_id,body FROM recording_manifests WHERE recording=?", (recording,)
        ).fetchall()
        if len(closed) != 1:
            return {"closed": False}
        device, sealed = closed[0]
        manifest = json.loads(store.decrypt(sealed, f"manifest:{device}:{recording}"))
        result = {
            "closed": True,
            "frames": 0,
            "chunks": 0,
            "complete": manifest.get("gaps") == 0,
            "started_at_ms": manifest["started_at_ms"],
            "finalized_at_ms": manifest["finalized_at_ms"],
        }
        for track in manifest["tracks"]:
            rows = db.execute(
                "SELECT c.id,c.sequence,c.sha256,c.body,j.metadata FROM chunks c "
                "JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id "
                "WHERE c.device_id=? AND c.recording=? AND c.track=? ORDER BY c.sequence",
                (device, recording, track["track"]),
            ).fetchall()
            frames = 0
            shape = None
            for ident, sequence, checksum, sealed, metadata in rows:
                body = store.decrypt(sealed, f"chunk:{device}:{ident}")
                meta = json.loads(store.decrypt(metadata, f"metadata:{device}:{ident}"))
                if hashlib.sha256(body).hexdigest() != checksum:
                    raise ValueError("fixture audio checksum mismatch")
                with wave.open(io.BytesIO(body), "rb") as audio:
                    current = (audio.getnchannels(), audio.getsampwidth(), audio.getframerate())
                    if (
                        current[0] not in (1, 2)
                        or current[1] != 2
                        or not 8000 <= current[2] <= 192000
                        or shape not in (None, current)
                    ):
                        raise ValueError("fixture audio has invalid or inconsistent PCM format")
                    shape = current
                    count = audio.getnframes()
                    pcm = audio.readframes(count)
                    if (
                        count != meta["frames"]
                        or len(pcm) != count * audio.getnchannels() * audio.getsampwidth()
                    ):
                        raise ValueError("fixture audio frames missing or unreadable")
                    frames += count
                if sequence != result.get("track_sequence", 0):
                    result["complete"] = False
                result["track_sequence"] = sequence + 1
            result.pop("track_sequence", None)
            result["complete"] &= (
                not track.get("missing_sequences")
                and len(rows) == track["chunk_count"]
                and len(rows) == track["last_sequence"] + 1
                and frames == track["total_frames"]
                and frames > 0
            )
            result["frames"] += frames
            result["chunks"] += len(rows)
        result["complete"] &= result["frames"] > 0
        return result


def read(config, recording):
    from orbit_hub.config import Settings
    from orbit_hub.storage import Store

    settings = Settings.load(config)
    store = Store(settings.data_dir, settings.key_file)
    try:
        return inspect_recording(store, recording)
    finally:
        store.close()
