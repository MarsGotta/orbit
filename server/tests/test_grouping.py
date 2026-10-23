"""Real encrypted SQLite and synthetic PCM: grouping/provenance, never ASR quality."""

import hashlib
import io
import wave
from uuid import uuid4

import pytest
from conftest import device
from support_grouping import (
    finalize,
    finish_all,
    pcm_wav,
    put,
    read_transcript,
)

from orbit_hub.grouping import (
    MAX_GROUP_AUDIO_BYTES,
    MAX_GROUP_SOURCES,
    Source,
    assemble_wav,
    contiguous,
    select_window,
)
from orbit_hub.protocol import HubError
from orbit_hub.storage import Store


def test_twenty_seconds_per_track_provenance_once_and_capture_independent_of_asr(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    original = set()
    for sequence in range(20):
        for track in ("mic_local", "remote_app"):
            original.add(put(store, owner, recording, track, sequence)[0])
    receipt = finalize(store, owner, recording, 20)
    assert receipt["capture_complete"] is True and receipt["transcript_ready"] is False
    jobs = finish_all(store)
    assert len(jobs) == 2
    seen = set()
    for job in jobs:
        manifest = job["manifest"]
        assert manifest["flush_reason"] == "target"
        assert manifest["timing_precision"] == "group"
        assert "chunk_id" not in manifest and "source_sha256" not in manifest
        assert len(manifest["sources"]) == 20 and manifest["frames"] == 320000
        assert set(manifest["source_chunk_ids"]).isdisjoint(seen)
        seen.update(manifest["source_chunk_ids"])
        with wave.open(io.BytesIO(job["audio"])) as audio:
            assert audio.getnframes() == 320000 and audio.getframerate() == 16000
    assert seen == original
    capture = store.groups.capture(owner, recording)
    assert capture["capture_complete"] and capture["transcript_ready"]
    assert capture["pending_chunks"] == 0
    transcript = read_transcript(store, owner, recording)
    assert len(transcript.segments) == 2
    texts = [segment.text for segment in transcript.segments]
    for job in jobs:
        assert texts.count(f"ASR GROUP {job['group_id']}") == 1
    assert store._db.execute("SELECT count(*) FROM chunks").fetchone()[0] == 40
    assert store._db.execute("SELECT count(*) FROM transcripts").fetchone()[0] == 0


def test_partial_finalization_flushes_without_waiting_for_deadline(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    for sequence in range(3):
        for track in ("mic_local", "remote_app"):
            put(store, owner, recording, track, sequence)
    assert store.groups.claim(1029) is None
    assert finalize(store, owner, recording, 3)["capture_complete"]
    jobs = finish_all(store, 1029)
    assert len(jobs) == 2
    assert all(job["manifest"]["flush_reason"] == "finalized" for job in jobs)
    assert all(len(job["manifest"]["sources"]) == 3 for job in jobs)


def test_deadline_receipt_survives_reopen_and_duplicate_does_not_extend_it(environment):
    settings, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    put(store, owner, recording, "mic_local", 0, received=1000)
    put(store, owner, recording, "mic_local", 1, received=1010)
    put(store, owner, recording, "mic_local", 0, received=1028)
    reopened = Store(settings.data_dir, settings.key_file)
    try:
        assert reopened.groups.claim(1029) is None
        job = reopened.groups.claim(1030)
        assert job["manifest"]["flush_reason"] == "deadline"
        assert len(job["manifest"]["sources"]) == 2
    finally:
        reopened.close()


def test_recovery_keeps_group_id_sources_and_rejects_stale_attempt(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    for sequence in range(20):
        put(store, owner, recording, "mic_local", sequence)
    first = store.groups.claim(1000)
    store.recover_interrupted()
    second = store.groups.claim(1001)
    assert second["group_id"] == first["group_id"]
    assert second["audio"] == first["audio"] and second["manifest"] == first["manifest"]
    with pytest.raises(HubError, match="obsoleto"):
        store.groups.finish(first, "STALE MUST NOT BECOME FINAL")
    store.groups.fail(first, "stale_failure", 1002)
    store.groups.finish(second, "IMMUTABLE GROUP TEXT")
    store.groups.finish(first, "NO OVERWRITE")
    assert read_transcript(store, owner, recording).segments[0].text == "IMMUTABLE GROUP TEXT"
    assert store.groups.claim(5000) is None


def test_gap_late_arrival_and_clock_pause_never_reopen_or_cross_groups(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    first, _, _ = put(store, owner, recording, "mic_local", 0)
    put(store, owner, recording, "mic_local", 2)
    assert store.groups.claim(1029) is None
    a = store.groups.claim(1030)
    assert a["manifest"]["source_chunk_ids"] == [first]
    assert a["manifest"]["flush_reason"] == "deadline"
    store.groups.finish(a, "before gap")
    put(store, owner, recording, "mic_local", 1)
    b = store.groups.claim(1030)
    assert [source["sequence"] for source in b["manifest"]["sources"]] == [1, 2]
    store.groups.finish(b, "late but separate")
    put(store, owner, recording, "mic_local", 3, monotonic_ns=100_000_000_000)
    put(store, owner, recording, "mic_local", 4, monotonic_ns=110_000_000_000)
    c = store.groups.claim(1000)
    assert [source["sequence"] for source in c["manifest"]["sources"]] == [3]
    assert c["manifest"]["flush_reason"] == "boundary"
    assert read_transcript(store, owner, recording).segments[0].text == "before gap"


def test_out_of_order_lexicographic_uploads_wait_for_missing_sequence(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    # Real daemon backlog IDs sort 0,1,10..19,2..9. Poll STT after every receipt.
    order = sorted(range(20), key=str)
    for sequence in order[:-1]:
        put(store, owner, recording, "mic_local", sequence)
        assert store.groups.claim(1000) is None
    put(store, owner, recording, "mic_local", order[-1])
    job = store.groups.claim(1000)
    assert job["manifest"]["flush_reason"] == "target"
    assert [source["sequence"] for source in job["manifest"]["sources"]] == list(range(20))
    assert store.groups.claim(1000) is None


def test_finalized_window_can_flush_a_prefix_without_crossing_a_sequence_hole():
    sources = [Source(str(sequence), sequence, "a" * 64, 46, {}, 1000) for sequence in (0, 2)]
    assert select_window(sources, now=1000, finalized=False) is None
    selected, reason = select_window(sources, now=1000, finalized=True)
    assert [source.sequence for source in selected] == [0] and reason == "finalized"


def test_terminal_retry_does_not_claim_again_or_mark_capture_incomplete(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    for track in ("mic_local", "remote_app"):
        put(store, owner, recording, track, 0)
    finalize(store, owner, recording, 1)
    # Complete one track, then exhaust exactly five attempts on the other.
    completed = store.groups.claim(1000)
    store.groups.finish(completed, "one track")
    now, identity = 1000, None
    for attempt in range(5):
        job = store.groups.claim(now)
        identity = identity or job["group_id"]
        assert job["group_id"] == identity and job["attempts"] == attempt
        store.groups.fail(job, "synthetic_provider_failure", now)
        if attempt == 0:
            pending = store._db.execute(
                "SELECT state,attempts,next_attempt FROM transcription_groups WHERE id=?",
                (identity,),
            ).fetchone()
            assert pending["state"] == "pending"
            assert pending["attempts"] == 1
            assert int(pending["next_attempt"] * 1000) == 1_010_000
        assert store.groups.claim(now + 1) is None
        now += 400
    assert store.groups.claim(now) is None
    capture = store.groups.capture(owner, recording)
    assert capture["capture_complete"] and not capture["transcript_ready"]
    assert capture["pending_chunks"] == 1
    rows = store._db.execute(
        "SELECT state,attempts FROM transcription_groups WHERE recording=?", (recording,)
    ).fetchall()
    assert {part["state"] for part in rows} == {"completed", "failed"}
    assert next(part for part in rows if part["state"] == "failed")["attempts"] == 5
    assert [segment.text for segment in read_transcript(store, owner, recording).segments] == [
        "one track"
    ]


def test_source_count_payload_format_and_corruption_bounds():
    sources = [
        Source(
            str(i),
            i,
            "0" * 64,
            46,
            {
                "frames": 1,
                "first_sample": i,
                "sample_rate": 16000,
                "channels": 1,
                "monotonic_ns": i * 62500,
            },
            1000,
        )
        for i in range(MAX_GROUP_SOURCES + 1)
    ]
    selected, reason = select_window(sources, now=1000, finalized=False)
    assert len(selected) == MAX_GROUP_SOURCES and reason == "bound"
    large = [
        Source(
            source.chunk_id,
            source.sequence,
            source.sha256,
            MAX_GROUP_AUDIO_BYTES // 2,
            source.metadata,
            source.received_at,
        )
        for source in sources[:2]
    ]
    assert select_window(large, now=1000, finalized=False)[1] == "bound"
    assert contiguous(sources[0], sources[1])
    body = pcm_wav(1)
    metadata = {**sources[0].provenance(), "sha256": hashlib.sha256(body).hexdigest()}
    assert assemble_wav([metadata], [body]) == body
    with pytest.raises(HubError):
        assemble_wav([metadata], [body + b"corrupt"])
    with pytest.raises(HubError):
        assemble_wav([{**metadata, "frames": 2}], [body])
    other_format = pcm_wav(1, channels=2)
    with pytest.raises(HubError):
        assemble_wav(
            [metadata, {"sha256": hashlib.sha256(other_format).hexdigest()}], [body, other_format]
        )


def test_invalid_group_audio_is_terminal_not_forwarded_to_provider(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    chunk, _, _ = put(store, owner, recording, "mic_local", 0)
    with store.transaction() as db:
        db.execute(
            "UPDATE chunks SET body=? WHERE device_id=? AND id=?",
            (store.encrypt(b"synthetic corrupt WAV", f"chunk:{owner}:{chunk}"), owner, chunk),
        )
    assert store.groups.claim(1030) is None
    segment = store._db.execute(
        "SELECT state,error_code FROM transcription_groups WHERE recording=?", (recording,)
    ).fetchone()
    assert segment["state"] == "failed" and segment["error_code"] == "invalid_group_audio"
    assert store.groups.claim(5000) is None


def test_grouping_is_deterministic_per_track_and_epoch(environment):
    _, store, _ = environment
    owner, recording = device(store)["device_id"], str(uuid4())
    chunks = {}
    for track in ("mic_local", "remote_app"):
        for sequence, epoch in enumerate(("epoch-a", "epoch-b")):
            chunks[(track, sequence)] = put(
                store, owner, recording, track, sequence, capture_epoch_id=epoch
            )
    finalize(store, owner, recording, 2)
    jobs = finish_all(store)
    assert len(jobs) == 4
    assert len({job["group_id"] for job in jobs}) == 4
    sealed = {}
    for job in jobs:
        manifest = job["manifest"]
        assert manifest["pipeline_version"] == 2 and manifest["speaker"] == "unknown"
        assert len(manifest["sources"]) == 1
        source = manifest["sources"][0]
        chunk, metadata, _ = chunks[(manifest["track"], source["sequence"])]
        assert source["chunk_id"] == chunk
        assert chunk.startswith(f"{recording}-{manifest['track']}-")
        assert source["capture_epoch_id"] == metadata["capture_epoch_id"]
        assert manifest["source_chunk_ids"] == [chunk]
        sealed.setdefault(manifest["track"], []).append(
            (manifest["sequence"], manifest["flush_reason"])
        )
    for track, groups in sealed.items():
        assert groups == [(0, "boundary"), (1, "finalized")]
