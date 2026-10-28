"""A chunk the hub never got, in the middle of a track, declared by the Mac.

The recording still closes: the manifest names the missing sequences, the
hub checks them against what it holds, transcribes around the hole, and the
transcript and the note say a part is missing, with its time span.
"""

from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_grouping import finish_all, put, read_transcript

from orbit_hub.notes.generation import SYSTEM
from orbit_hub.protocol import MAX_DECLARED_MISSING_CHUNKS, HubError

START = 900_000


def recording_with_a_hole(store, owner):
    """Five one-second chunks per track; the microphone's third never came."""
    recording = str(uuid4())
    for sequence in range(5):
        put(store, owner, recording, "remote_app", sequence)
        if sequence != 2:
            put(store, owner, recording, "mic_local", sequence)
    return recording


def manifest(missing=(2,), *, mic_count=4, last=4):
    mic = {
        "track": "mic_local",
        "chunk_count": mic_count,
        "last_sequence": last,
        "total_frames": mic_count * 16000,
    }
    if missing is not None:
        mic["missing_sequences"] = list(missing)
    return {
        "tracks": [
            mic,
            {
                "track": "remote_app",
                "chunk_count": 5,
                "last_sequence": 4,
                "total_frames": 5 * 16000,
            },
        ],
        "gaps": 1,
        "started_at_ms": START,
        "finalized_at_ms": START + 5000,
    }


def test_a_chunk_missing_in_the_middle_is_declared_and_the_recording_closes(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    recording = recording_with_a_hole(store, owner)
    receipt = store.recordings.finalize(owner, recording, manifest())
    assert receipt["finalized"] is True and receipt["durable"] is True
    # Honest: a declared hole is not a complete capture.
    assert receipt["capture_complete"] is False
    finish_all(store)
    capture = store.groups.capture(owner, recording)
    assert capture["pending_chunks"] == 0, "every chunk around the hole is transcribed"
    assert capture["transcript_ready"] is True
    transcript = read_transcript(store, owner, recording)
    assert [hole.__dict__ for hole in transcript.missing] == [
        {
            "track": "mic_local",
            "first_sequence": 2,
            "last_sequence": 2,
            "at_ms": 2000,
            "duration_ms": 1000,
        }
    ]
    marker = [line for line in transcript.lines() if line.speaker == "Orbit"]
    assert len(marker) == 1 and marker[0].at_ms == 2000
    assert "Falta el audio" in marker[0].text and "1 s" in marker[0].text


def test_the_document_and_the_notes_prompt_say_a_part_is_missing(environment):
    _, store, _ = environment
    owner = device(store)
    recording = recording_with_a_hole(store, owner["device_id"])
    store.recordings.finalize(owner["device_id"], recording, manifest())
    finish_all(store)
    with client(environment) as http:
        response = http.get(
            f"/v1/recordings/{recording}",
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    assert response.status_code == 200, response.text
    document = response.json()
    assert document["transcript"]["missing_audio"] == [
        {"track": "mic_local", "at_ms": 2000, "duration_ms": 1000, "chunks": 1}
    ]
    assert "missing audio" in SYSTEM.lower()


@pytest.mark.parametrize(
    "declared, mic_count, last",
    [
        ((), 4, 4),  # the hole is not declared
        ((1,), 4, 4),  # a chunk the hub holds is called missing
        ((2, 4), 4, 4),  # the last chunk, which the hub holds, is called missing
        ((2,), 4, 3),  # the last sequence held is not the last one declared
        ((2,), 3, 4),  # the count of chunks held is wrong
    ],
)
def test_a_declaration_that_does_not_match_what_the_hub_holds_is_refused(
    environment, declared, mic_count, last
):
    _, store, _ = environment
    owner = device(store)["device_id"]
    recording = recording_with_a_hole(store, owner)
    with pytest.raises(HubError) as error:
        store.recordings.finalize(
            owner, recording, manifest(declared, mic_count=mic_count, last=last)
        )
    assert error.value.code == "manifest_mismatch"


def test_the_route_bounds_and_orders_the_declared_sequences(environment):
    _, store, _ = environment
    owner = device(store)
    recording = recording_with_a_hole(store, owner["device_id"])
    headers = {"Authorization": "Bearer " + owner["token"]}
    too_many = list(range(MAX_DECLARED_MISSING_CHUNKS + 1))
    with client(environment) as http:
        for declared in ([2, 2], [3, 2], [-1], too_many):
            response = http.post(
                f"/v1/recordings/{recording}/finalize",
                headers=headers,
                json=manifest(declared),
            )
            assert response.status_code == 400, (declared[:3], response.text)
            assert response.json()["code"] == "invalid_manifest"
        response = http.post(
            f"/v1/recordings/{recording}/finalize", headers=headers, json=manifest()
        )
        assert response.status_code == 200, response.text


def test_a_manifest_without_missing_sequences_closes_and_is_stored_as_before(environment):
    _, store, _ = environment
    owner = device(store)
    recording = str(uuid4())
    for sequence in range(2):
        put(store, owner["device_id"], recording, "mic_local", sequence)
    old = {
        "tracks": [
            {"track": "mic_local", "chunk_count": 2, "last_sequence": 1, "total_frames": 32000}
        ],
        "gaps": 0,
        "started_at_ms": START,
        "finalized_at_ms": START + 2000,
    }
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        for _ in range(2):
            response = http.post(f"/v1/recordings/{recording}/finalize", headers=headers, json=old)
            assert response.status_code == 200, response.text
    with store.read_snapshot() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner["device_id"], recording),
        ).fetchone()
    stored = store.decrypt(row["body"], f"manifest:{owner['device_id']}:{recording}")
    assert b"missing_sequences" not in stored
