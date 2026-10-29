"""«Silenciar»: the stretches where the Mac muted the microphone.

The Mac sent silence for them, so they hold no words; the closing manifest
says where they are. The transcript says each in one calm line, and the
final pass neither counts them as lost words nor keeps what a model heard in
that silence. A manifest without them reads as before.
"""

from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_grouping import finish_all, put, read_transcript
from test_final_pass_integrity import audio_bytes, run_pass, spoken, word

from orbit_hub.protocol import MAX_MUTED_SPANS

START = 900_000


def two_tracks(store, owner, seconds=5):
    recording = str(uuid4())
    for sequence in range(seconds):
        put(store, owner, recording, "mic_local", sequence)
        put(store, owner, recording, "remote_app", sequence)
    return recording


def manifest(muted=None, seconds=5):
    body = {
        "tracks": [
            {
                "track": track,
                "chunk_count": seconds,
                "last_sequence": seconds - 1,
                "total_frames": seconds * 16000,
            }
            for track in ("mic_local", "remote_app")
        ],
        "gaps": 0,
        "started_at_ms": START,
        "finalized_at_ms": START + seconds * 1000,
    }
    if muted is not None:
        body["muted_spans"] = muted
    return body


def finalized(environment, muted):
    _, store, _ = environment
    owner = device(store)
    recording = two_tracks(store, owner["device_id"])
    with client(environment) as http:
        response = http.post(
            f"/v1/recordings/{recording}/finalize",
            json=manifest(muted),
            headers={"Authorization": "Bearer " + owner["token"]},
        )
        assert response.status_code == 200, response.text
        finish_all(store)
        document = http.get(
            f"/v1/recordings/{recording}",
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    assert document.status_code == 200, document.text
    return store, owner["device_id"], recording, document.json()


def test_the_transcript_says_each_muted_stretch_in_one_calm_line(environment):
    store, owner, recording, document = finalized(environment, [{"start_ms": 1000, "end_ms": 3500}])
    assert document["transcript"]["muted_microphone"] == [{"start_ms": 1000, "end_ms": 3500}]
    transcript = read_transcript(store, owner, recording)
    lines = [line for line in transcript.lines() if line.speaker == "Orbit"]
    assert [(line.at_ms, line.text) for line in lines] == [
        (1000, "Microphone muted · 00:01 – 00:03")
    ]


def test_a_closure_without_muted_stretches_reads_as_before(environment):
    store, owner, recording, document = finalized(environment, None)
    assert "muted_microphone" not in document["transcript"]
    assert not [
        line for line in read_transcript(store, owner, recording).lines() if line.speaker == "Orbit"
    ]
    with store.read_snapshot() as db:
        body = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner, recording),
        ).fetchone()["body"]
    stored = store.decrypt(body, f"manifest:{owner}:{recording}")
    assert b"muted_spans" not in stored, "kept as a Mac without «Silenciar» wrote it"


@pytest.mark.parametrize(
    "muted",
    [
        [{"start_ms": 3000, "end_ms": 1000}],
        [{"start_ms": -1, "end_ms": 1000}],
        [{"start_ms": 0, "end_ms": 1}] * (MAX_MUTED_SPANS + 1),
    ],
)
def test_a_muted_stretch_that_cannot_be_is_refused(environment, muted):
    _, store, _ = environment
    owner = device(store)
    recording = two_tracks(store, owner["device_id"])
    with client(environment) as http:
        response = http.post(
            f"/v1/recordings/{recording}/finalize",
            json=manifest(muted),
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    assert response.status_code == 400, response.text
    assert response.json()["code"] == "invalid_manifest"


async def test_a_muted_stretch_is_not_lost_words_and_its_silence_says_nothing(
    environment, monkeypatch
):
    # A model heard words in the silence the Mac sent: the live draft of the
    # muted first group and one final word inside it. Neither is speech.
    heard_in_silence = "Gracias por ver el vídeo hasta el final."
    spoken_after = [word(25, "Dos."), word(26, "Tres."), word(27, "Cuatro.")]
    *_, state, transcript, _ = await run_pass(
        environment,
        monkeypatch,
        audio_bytes(40, silences=((0, 20),)),
        lambda _: [word(5, "Gracias."), *spoken_after],
        drafts=[heard_in_silence, "Dos tres cuatro."],
        muted=[{"start_ms": 0, "end_ms": 20_000}],
    )
    assert state == {"state": "completed", "error": None}, "nothing of the draft was kept"
    assert [text for _, text in spoken(transcript, "mic_local")] == ["Dos.", "Tres.", "Cuatro."]
