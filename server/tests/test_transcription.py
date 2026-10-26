import asyncio
import hashlib
from uuid import uuid4

from conftest import device
from support_app import client
from support_grouping import read_transcript
from support_transcription import (
    insert_job,
    wav_fixture,
)

from orbit_hub.meeting_transcription import Transcription
from orbit_hub.transcription import TranscriptionWorker


class MeetingTranscriberFake:
    def __init__(self, audio):
        self.audio = audio

    async def transcribe(self, audio, language=None):
        self.audio.append(audio)
        return Transcription("Transcripción de reunión sintética", "es")


def test_jobs_recover_and_transcript_is_immutable(environment):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    chunk, recording = insert_job(store, owner)
    job = store.groups.claim(1000)
    assert store.groups.claim(1000) is None
    store.recover_interrupted()
    recovered = store.groups.claim(1001)
    assert recovered["chunk_id"] == chunk
    store.groups.finish(recovered, "SYNTHETIC-TRANSCRIPT-CANARY")
    store.groups.finish(recovered, "must not overwrite")
    transcript = read_transcript(store, owner, recording)
    assert transcript.segments[0].text == "SYNTHETIC-TRANSCRIPT-CANARY"
    assert store.groups.capture(owner, recording)["capture_finalized"] is False
    for path in settings.data_dir.iterdir():
        if path.is_file():
            assert b"SYNTHETIC-TRANSCRIPT-CANARY" not in path.read_bytes()
    other = device(store)
    with client(environment) as http:
        shared = http.get(
            f"/v1/recordings/{recording}/turns",
            headers={"Authorization": "Bearer " + other["token"]},
        )
    assert shared.status_code == 200
    assert shared.json()["turns"][0]["text"] == "SYNTHETIC-TRANSCRIPT-CANARY"
    assert job["audio"] == wav_fixture()


def test_failed_job_backoff_and_revocation_preserves_the_persons_work(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    insert_job(store, owner)
    job = store.groups.claim(1000)
    store.groups.fail(job, "transcription_failed", 1000)
    assert store.groups.claim(1001) is None
    assert store.groups.claim(1011)["attempts"] == 1
    store.recover_interrupted()
    store.devices.revoke(owner)
    retried = store.groups.claim(2000)
    assert retried is not None
    assert retried["device_id"] == owner and retried["attempts"] == 2


async def test_interactive_has_priority_and_worker_stays_serial(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    _, recording = insert_job(store, owner)
    worker = TranscriptionWorker(
        store,
        adapter,
        lambda: 1000,
        meetings=MeetingTranscriberFake(adapter.audio),
    )
    task = asyncio.create_task(worker.transcribe(owner, b"interactive-fixture"))
    await asyncio.sleep(0)
    worker.start()
    assert await task == "Transcripción sintética"
    assert adapter.audio[0] == b"interactive-fixture"
    assert adapter.audio[1] == wav_fixture()
    assert store.groups.capture(owner, recording)["pending_chunks"] == 0
    await worker.close()


def test_upload_metadata_conflict(environment):
    _, store, _ = environment
    owner = device(store)
    body, recording = wav_fixture(), str(uuid4())
    chunk = recording + "-mic_local-0"
    headers = {
        "Authorization": "Bearer " + owner["token"],
        "X-Orbit-Recording": recording,
        "X-Orbit-Track": "mic_local",
        "X-Orbit-Sequence": "0",
        "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
        "X-Orbit-First-Sample": "0",
        "X-Orbit-Frames": "160",
        "X-Orbit-Sample-Rate": "16000",
        "X-Orbit-Channels": "1",
        "X-Orbit-Monotonic-Ns": "1000",
        "X-Orbit-Started-At-Ms": "2000",
    }
    with client(environment) as http:
        response = http.put(f"/v1/chunks/{chunk}", content=body, headers=headers)
        assert response.status_code == 200 and response.json()["durable"]
        assert http.put(f"/v1/chunks/{chunk}", content=body, headers=headers).status_code == 200
        changed = {**headers, "X-Orbit-Monotonic-Ns": "1001"}
        assert http.put(f"/v1/chunks/{chunk}", content=body, headers=changed).status_code == 409


async def test_an_interactive_utterance_wakes_the_idle_worker(environment, monkeypatch):
    # Disable polling: only an enqueue notification can release the worker.
    monkeypatch.setattr("orbit_hub.transcription.PREPARE_TICK_SECONDS", None)
    _, store, adapter = environment
    owner = device(store)["device_id"]
    idle = asyncio.Event()

    class ObservedSignal(asyncio.Event):
        async def wait(self):
            idle.set()
            return await super().wait()

    worker = TranscriptionWorker(
        store,
        adapter,
        lambda: 1000,
        meetings=MeetingTranscriberFake(adapter.audio),
        prepare_signal=ObservedSignal(),
    )
    worker.start()
    try:
        await asyncio.wait_for(idle.wait(), 2)
        # The worker is waiting for work, not already draining this utterance.
        # Two seconds is only the neighboring scheduling watchdog: the safety
        # tick cannot release this request, even on a slow test host.
        assert (
            await asyncio.wait_for(worker.transcribe(owner, b"wake-fixture"), 2)
            == "Transcripción sintética"
        )
        assert adapter.audio == [b"wake-fixture"]
    finally:
        await worker.close()
