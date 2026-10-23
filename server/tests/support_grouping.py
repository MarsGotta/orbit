"""Shared fixtures and helpers for test_grouping."""

import hashlib

from support_audio import wav_bytes

__all__ = (
    "finalize",
    "finish_all",
    "pcm_wav",
    "put",
)


def pcm_wav(frames=16000, rate=16000, channels=1):
    return wav_bytes(b"\x00\x01" * frames * channels, rate=rate, channels=channels)


def put(store, owner, recording, track, sequence, *, received=1000, frames=16000, **changes):
    body = pcm_wav(frames)
    metadata = {
        "first_sample": sequence * frames,
        "frames": frames,
        "sample_rate": 16000,
        "channels": 1,
        "monotonic_ns": 1_000_000_000 + sequence * frames * 1_000_000_000 // 16000,
        "started_at_ms": 900000 + sequence * 1000,
        "wall_time_precision": "estimated",
        **changes,
    }
    chunk = f"{recording}-{track}-{sequence}"
    store.recordings.put_chunk(
        owner,
        chunk,
        recording,
        track,
        sequence,
        hashlib.sha256(body).hexdigest(),
        body,
        metadata,
        received_at=received,
    )
    return chunk, metadata, body


def finalize(store, owner, recording, count, *, tracks=("mic_local", "remote_app"), gaps=0):
    return store.recordings.finalize(
        owner,
        recording,
        {
            "tracks": [
                {
                    "track": track,
                    "chunk_count": count,
                    "last_sequence": count - 1,
                    "total_frames": count * 16000,
                }
                for track in tracks
            ],
            "gaps": gaps,
            "started_at_ms": 1000,
            "finalized_at_ms": 1000 + count * 1000,
        },
    )


def finish_all(store, now=1000):
    jobs = []
    while job := store.groups.claim(now):
        store.groups.finish(job, f"ASR GROUP {job['group_id']}")
        jobs.append(job)
    return jobs


def read_transcript(store, owner, recording):
    from orbit_hub.meetings.turns import read

    with store.read_snapshot() as db:
        return read(store, db, owner, recording)
