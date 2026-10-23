"""Shared fixtures and helpers for test_transcription."""

import hashlib
from uuid import uuid4

from support_audio import wav_bytes

__all__ = (
    "insert_job",
    "wav_fixture",
)


def wav_fixture():
    return wav_bytes(b"\x00\x01" * 160)


def insert_job(store, owner, sequence=0):
    body, chunk, recording = wav_fixture(), str(uuid4()), str(uuid4())
    metadata = {
        "first_sample": 160,
        "frames": 160,
        "sample_rate": 16000,
        "channels": 1,
        "monotonic_ns": 1000,
        "started_at_ms": 2000,
    }
    store.recordings.put_chunk(
        owner,
        chunk,
        recording,
        "mic_local",
        sequence,
        hashlib.sha256(body).hexdigest(),
        body,
        metadata,
    )
    return chunk, recording
