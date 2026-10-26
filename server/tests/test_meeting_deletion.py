"""Deleting a recording on the hub, and everything made from it.

«Borrar reunión» and «Terminar sin guardar» both end in `DELETE
/v1/recordings/{id}`: the audio, the live transcript, what the final pass
started, the note with its tasks, the speakers, the private notes, the title,
the language and the meeting's choice of dictionaries leave the hub in one
transaction. The dictionaries themselves, the series' among them, stay. A
tombstone keeps a late chunk or closure from bringing it back, and makes a
second delete a success. Another meeting of the device and any meeting of
another device are untouched.

Synthetic meetings over the real store and routes; the note model is a fake.
"""

import json
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_client_e2e import tcp_server
from support_grouping import put
from support_notes import FakeModel
from support_waits import wait_for
from test_meeting_views import (
    NOW,
    STARTED,
    TEXTS,
    bearer,
    meeting,
    ready_note,
    running_client,
    upload_headers,
    wav_with,
)
from websockets.asyncio.client import connect

from orbit_hub.app import create_app
from orbit_hub.meeting_final_pass import FinalPassRepository


@pytest.fixture
def hub(environment):
    settings, store, adapter = environment
    settings = replace(
        settings,
        meeting_transcription={**settings.meeting_transcription.model_dump(), "language": "es"},
    )
    app = create_app(
        settings,
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: NOW,
        processing_client=FakeModel(),
    )
    return app, store


def traces(store, device_id: str, recording: str) -> dict[str, int]:
    """How many rows of every table name this recording, by table.

    Every table with a `recording` column is counted by it; the tables keyed
    by chunk or by transcription group are counted through them, so a row
    left behind by any of them shows here whatever its table.
    """
    counts = {}
    with store.read_snapshot() as db:
        chunks = [
            row["id"]
            for row in db.execute(
                "SELECT id FROM chunks WHERE device_id=? AND recording=?", (device_id, recording)
            )
        ]
        groups = [
            row["id"]
            for row in db.execute(
                "SELECT id FROM transcription_groups WHERE device_id=? AND recording=?",
                (device_id, recording),
            )
        ]
        tables = [
            row["name"] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")
        ]
        for table in tables:
            columns = {row["name"] for row in db.execute(f"PRAGMA table_info({table})")}
            if "device_id" not in columns:
                continue
            for column, values in (
                ("recording", [recording]),
                ("chunk_id", chunks),
                ("group_id", groups),
            ):
                if column not in columns or not values:
                    continue
                marks = ",".join("?" * len(values))
                found = db.execute(
                    f"SELECT count(*) FROM {table} WHERE device_id=? AND {column} IN ({marks})",
                    (device_id, *values),
                ).fetchone()[0]
                if found:
                    counts[table] = counts.get(table, 0) + found
    counts.pop("deleted_recordings", None)
    return counts


async def dressed(http, store, paired):
    """A finished meeting with everything a person adds to one on the hub."""
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    headers = bearer(paired)
    written = await http.post(f"/v1/recordings/{recording}/note", json={}, headers=headers)
    assert written.status_code == 202, written.text
    await ready_note(http, paired, recording)
    document = (await http.get(f"/v1/recordings/{recording}", headers=headers)).json()
    speaker = document["speakers"][0]["key"]
    for method, path, body in (
        ("PUT", "speakers", {"names": {speaker: "Bruno Lago"}}),
        ("PUT", "private-notes", {"text": "- Ojo con el plazo", "base_revision": 0}),
        ("PUT", "title", {"title": "Revisión de Lumen"}),
    ):
        answer = await http.request(
            method, f"/v1/recordings/{recording}/{path}", json=body, headers=headers
        )
        assert answer.status_code == 200, answer.text
    dictionary = str(uuid4())
    for order in (
        {"action": "save", "dictionary_id": dictionary, "name": "Lumen"},
        {"action": "select", "recording_id": recording, "dictionary_id": dictionary},
        {
            "action": "correct",
            "recording_id": recording,
            "dictionary_id": dictionary,
            "wrong": "lumen",
            "correct": "Lumen",
        },
    ):
        answer = await http.post("/v1/meeting-dictionaries/orders", json=order, headers=headers)
        assert answer.status_code == 200, answer.text
    final = FinalPassRepository(store)
    assert final.request(paired["device_id"], recording, configured=True)
    job = final.claim()
    final.complete(job, {"spans": [], "words": [], "audio_ms": 0, "language": "es"})
    return recording, dictionary


async def test_a_deleted_meeting_leaves_nothing_on_the_hub_and_the_rest_stays(hub):
    app, store = hub
    paired, other = device(store), device(store, name="otro Mac")
    async with running_client(app) as http:
        recording, dictionary = await dressed(http, store, paired)
        kept = meeting(store, paired["device_id"], texts=TEXTS)
        theirs = meeting(store, other["device_id"], texts=TEXTS)
        before = traces(store, paired["device_id"], recording)
        kept_before = traces(store, paired["device_id"], kept)
        theirs_before = traces(store, other["device_id"], theirs)
        deleted = await http.delete(f"/v1/recordings/{recording}", headers=bearer(paired))
        again = await http.delete(f"/v1/recordings/{recording}", headers=bearer(paired))
        document = await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))
        listed = (await http.get("/v1/recordings", headers=bearer(paired))).json()
        library = (await http.get("/v1/meeting-dictionaries", headers=bearer(paired))).json()
    # Everything a meeting gathers was there to be deleted.
    for table in (
        "chunks",
        "transcription_jobs",
        "chunk_receipts",
        "transcription_groups",
        "group_transcripts",
        "transcription_group_chunks",
        "recording_manifests",
        "recording_notes",
        "recording_speakers",
        "recording_private_notes",
        "recording_titles",
        "recording_dictionaries",
        "recording_final_passes",
        "recording_final_transcripts",
        "recording_voices",
    ):
        assert before.get(table), (table, before)
    assert deleted.status_code == 200, deleted.text
    assert deleted.json() == {"recording_id": recording, "deleted": True}
    assert again.status_code == 200 and again.json() == deleted.json()
    assert traces(store, paired["device_id"], recording) == {}
    assert document.status_code == 404 and document.json()["code"] == "recording_not_found"
    assert {row["recording_id"] for row in listed["meetings"]} == {kept, theirs}
    # The other meeting, the other Mac's, and the dictionary with its
    # correction are the hub's still.
    assert traces(store, paired["device_id"], kept) == kept_before
    assert traces(store, other["device_id"], theirs) == theirs_before
    [saved] = library["dictionaries"]
    assert saved["id"] == dictionary and saved["terms"][0]["correct"] == "Lumen"


async def test_a_recording_deleted_while_open_takes_no_late_chunk_or_closure(hub):
    app, store = hub
    paired = device(store)
    recording = str(uuid4())
    put(store, paired["device_id"], recording, "mic_local", 0)
    async with running_client(app) as http:
        deleted = await http.delete(f"/v1/recordings/{recording}", headers=bearer(paired))
        body = wav_with(3)
        late = await http.put(
            f"/v1/chunks/{recording}-mic_local-1",
            content=body,
            headers=upload_headers(paired, recording, "mic_local", 1, body, STARTED),
        )
        closed = await http.post(
            f"/v1/recordings/{recording}/finalize",
            json={
                "tracks": [
                    {"track": "mic_local", "chunk_count": 1, "last_sequence": 0, "total_frames": 1}
                ],
                "gaps": 0,
                "started_at_ms": STARTED,
                "finalized_at_ms": STARTED + 1_000,
            },
            headers=bearer(paired),
        )
        listed = (await http.get("/v1/recordings", headers=bearer(paired))).json()
    assert deleted.status_code == 200, deleted.text
    assert late.status_code == 409 and late.json()["code"] == "recording_deleted", late.text
    assert closed.status_code == 409 and closed.json()["code"] == "recording_deleted", closed.text
    assert listed["meetings"] == []
    assert traces(store, paired["device_id"], recording) == {}


async def test_a_recording_the_hub_never_had_is_deleted_too_and_stays_away(hub):
    """«Terminar sin guardar» with the hub away: nothing reached it, and the
    order still succeeds when it comes back; a chunk sent late is refused."""
    app, store = hub
    paired = device(store)
    recording = str(uuid4())
    async with running_client(app) as http:
        deleted = await http.delete(f"/v1/recordings/{recording}", headers=bearer(paired))
        body = wav_with(1)
        late = await http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers=upload_headers(paired, recording, "mic_local", 0, body, STARTED),
        )
    assert deleted.status_code == 200 and deleted.json()["deleted"] is True
    assert late.status_code == 409 and late.json()["code"] == "recording_deleted"


async def test_another_device_deletes_the_persons_meeting(hub):
    app, store = hub
    paired, other = device(store), device(store, name="otro Mac")
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    assert traces(store, paired["device_id"], recording)
    async with running_client(app) as http:
        foreign = await http.delete(f"/v1/recordings/{recording}", headers=bearer(other))
        mine = await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))
    assert foreign.status_code == 200
    assert mine.status_code == 404
    assert traces(store, paired["device_id"], recording) == {}


async def test_a_bad_identifier_is_refused_with_its_field(hub):
    app, store = hub
    paired = device(store)
    async with running_client(app) as http:
        bad = await http.delete("/v1/recordings/not-a-uuid", headers=bearer(paired))
        anonymous = await http.delete(f"/v1/recordings/{uuid4()}")
    assert bad.status_code == 400 and bad.json()["refusal"]["field"] == "recording_id"
    assert anonymous.status_code == 401


async def test_a_final_pass_that_ends_after_the_delete_brings_nothing_back(hub):
    _app, store = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    final = FinalPassRepository(store)
    assert final.request(paired["device_id"], recording, configured=True)
    job = final.claim()
    from orbit_hub.meetings.deletion import delete

    delete(store, paired["device_id"], recording, NOW)
    final.complete(job, {"spans": [], "words": [], "audio_ms": 0, "language": "es"})
    assert traces(store, paired["device_id"], recording) == {}


async def test_ready_says_it_deletes_and_the_owner_is_told_to_read_again(environment):
    settings, store, adapter = environment
    paired, other = device(store), device(store, name="otro Mac")
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    app = create_app(
        replace(
            settings,
            allow_insecure_localhost=True,
            allowed_hosts=("127.0.0.1", "localhost"),
        ),
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: NOW,
        processing_client=FakeModel(),
    )
    async with tcp_server(app) as url, httpx.AsyncClient(base_url=url) as http:
        ws_url = url.replace("http://", "ws://") + "/v1/ws"
        async with (
            connect(ws_url, additional_headers=bearer(paired)) as mine,
            connect(ws_url, additional_headers=bearer(paired)) as second,
            connect(ws_url, additional_headers=bearer(other)) as theirs,
        ):
            readies = [json.loads(await socket.recv()) for socket in (mine, second, theirs)]
            deleted = await http.delete(f"/v1/recordings/{recording}", headers=bearer(paired))
            assert deleted.status_code == 200, deleted.text
            for socket in (mine, second):
                frame = json.loads(await wait_for(socket.recv(), "el aviso del borrado"))
                assert frame["event"] == "recording.changed"
                assert frame["data"] == {"recording_id": recording}
    assert all(ready["data"]["recording_delete"] is True for ready in readies)
