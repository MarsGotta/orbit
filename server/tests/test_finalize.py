import hashlib
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_transcription import wav_fixture

from orbit_hub.protocol import HubError


def recording_fixture(store, owner):
    recording = str(uuid4())
    body = wav_fixture()
    for track in ("mic_local", "remote_app"):
        chunk = recording + "-" + track + "-0"
        store.recordings.put_chunk(
            owner,
            chunk,
            recording,
            track,
            0,
            hashlib.sha256(body).hexdigest(),
            body,
            {"frames": 160, "sample_rate": 16000, "first_sample": 0},
        )
    manifest = {
        "tracks": [
            {"track": track, "chunk_count": 1, "last_sequence": 0, "total_frames": 160}
            for track in ("mic_local", "remote_app")
        ],
        "gaps": 0,
        "started_at_ms": 1000,
        "finalized_at_ms": 2000,
    }
    return recording, manifest


def test_finalize_checks_receipts_and_prevents_late_chunks(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    recording, manifest = recording_fixture(store, owner)
    bad = {
        **manifest,
        "tracks": [{**manifest["tracks"][0], "total_frames": 999}, manifest["tracks"][1]],
    }
    with pytest.raises(HubError) as error:
        store.recordings.finalize(owner, recording, bad)
    assert error.value.code == "manifest_mismatch"
    receipt = store.recordings.finalize(owner, recording, manifest)
    assert receipt == {
        "recording_id": recording,
        "finalized": True,
        "durable": True,
        "capture_complete": True,
        "transcript_ready": False,
    }
    assert store.recordings.finalize(owner, recording, manifest) == receipt
    with pytest.raises(HubError) as error:
        store.recordings.finalize(owner, recording, {**manifest, "gaps": 1})
    assert error.value.code == "manifest_conflict"
    body = wav_fixture()
    with pytest.raises(HubError) as error:
        store.recordings.put_chunk(
            owner,
            str(uuid4()),
            recording,
            "mic_local",
            1,
            hashlib.sha256(body).hexdigest(),
            body,
            {"frames": 160},
        )
    assert error.value.code == "recording_finalized"
    while job := store.groups.claim(3000):
        store.groups.finish(job, "synthetic")
    assert store.groups.capture(owner, recording)["transcript_ready"]


def test_finalize_gaps_and_endpoint_ownership(environment):
    _, store, _ = environment
    owner, other = device(store), device(store)
    recording, manifest = recording_fixture(store, owner["device_id"])
    manifest["gaps"] = 1
    with client(environment) as http:
        other_headers = {"Authorization": "Bearer " + other["token"]}
        headers = {"Authorization": "Bearer " + owner["token"]}
        assert (
            http.post(
                f"/v1/recordings/{recording}/finalize", headers=other_headers, json=manifest
            ).status_code
            == 409
        )
        response = http.post(f"/v1/recordings/{recording}/finalize", headers=headers, json=manifest)
        assert response.status_code == 200 and response.json()["capture_complete"] is False
    transcript = store.groups.capture(owner["device_id"], recording)
    assert transcript["capture_finalized"] is True and transcript["capture_complete"] is False


def test_finalization_retries_a_stored_manifest_without_kind_or_event(environment):
    _, store, _ = environment
    owner = device(store)
    recording, manifest = recording_fixture(store, owner["device_id"])
    first = store.recordings.finalize(owner["device_id"], recording, dict(manifest))
    assert "kind" not in manifest and "event" not in manifest
    with client(environment) as http:
        response = http.post(
            f"/v1/recordings/{recording}/finalize",
            headers={"Authorization": "Bearer " + owner["token"]},
            json={**manifest, "kind": "meeting", "event": None},
        )
    assert response.status_code == 200, response.text
    assert response.json()["recording_id"] == first["recording_id"]
    assert response.json()["finalized"] is True
    assert response.json()["durable"] is True
    assert response.json()["capture_complete"] is True
