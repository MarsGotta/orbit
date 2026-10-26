"""A personal hub shares meetings while preserving their recording device.

Public HTTP and signed assistant routes over an isolated encrypted store.
Synthetic completed audio/transcripts and a scripted note model only.
"""

from uuid import uuid4

import pytest
from conftest import device
from support_notes import note_answer, write_note
from test_meeting_views import (
    NOW,
    STARTED,
    TEXTS,
    all_turns,
    bearer,
    hub,
    meeting,
    ready_note,
    running_client,
    signed,
)

from orbit_hub.meetings_bridge import LIST_PATH, NOTE_PATH, TRANSCRIPT_PATH

__all__ = ("hub",)


async def test_other_mac_lists_opens_note_and_transcript(hub):
    app, store, _, _ = hub
    recorder, reader = device(store, "Mac de Alba"), device(store, "Mac de viaje")
    recording = meeting(store, recorder["device_id"], texts=TEXTS)
    await write_note(store, recorder["device_id"], recording)
    async with running_client(app) as http:
        listing = await http.get("/v1/recordings", headers=bearer(reader))
        assert listing.status_code == 200, listing.text
        [row] = listing.json()["meetings"]
        assert row["recording_id"] == recording
        assert row["recorded_on"] == {"device_id": recorder["device_id"], "name": "Mac de Alba"}
        opened = await http.get(f"/v1/recordings/{recording}", headers=bearer(reader))
        assert opened.status_code == 200, opened.text
        assert opened.json()["note"]["note"]["short_summary"] == "Resumen corto."
        assert opened.json()["recorded_on"] == row["recorded_on"]
        note = await http.get(f"/v1/recordings/{recording}/note", headers=bearer(reader))
        assert note.status_code == 200, note.text
        assert note.json()["state"] == "ready"
        turns = await all_turns(http, reader, recording)
        assert turns[0]["text"] == "Abrimos el comité."


async def test_previous_meeting_and_tasks_cross_devices(hub):
    app, store, _, _ = hub
    recorder, reader = device(store), device(store)
    recording = meeting(
        store,
        recorder["device_id"],
        texts=TEXTS,
        event={"calendar_item_id": "weekly-design", "title": "Comité semanal"},
    )
    await write_note(store, recorder["device_id"], recording)
    async with running_client(app) as http:
        answer = await http.get(
            "/v1/series/previous-meeting",
            headers=bearer(reader),
            params={"calendar_item_id": "weekly-design", "before_ms": STARTED + 60_000},
        )
        assert answer.status_code == 200, answer.text
        previous = answer.json()["previous"]
        assert previous is not None
        assert previous["recording_id"] == recording
        assert previous["note"]["action_items"][0]["text"] == "Enviar el acta"


@pytest.mark.parametrize("revoke", [False, True])
async def test_other_mac_deletes_whole_meeting_even_after_recorder_revoked(hub, revoke):
    app, store, _, _ = hub
    recorder, reader = device(store), device(store)
    recording = meeting(store, recorder["device_id"], texts=TEXTS)
    await write_note(store, recorder["device_id"], recording, private_notes="Apunte sintético")
    if revoke:
        store.devices.revoke(recorder["device_id"])
    async with running_client(app) as http:
        listed = await http.get("/v1/recordings", headers=bearer(reader))
        assert [row["recording_id"] for row in listed.json()["meetings"]] == [recording]
        for _ in range(2):
            deleted = await http.delete(f"/v1/recordings/{recording}", headers=bearer(reader))
            assert deleted.status_code == 200, deleted.text
        assert (await http.get("/v1/recordings", headers=bearer(reader))).json()["meetings"] == []
        assert (
            await http.get(f"/v1/recordings/{recording}", headers=bearer(reader))
        ).status_code >= 400
    with store.read_snapshot() as db:
        for table in (
            "chunks",
            "recording_manifests",
            "transcription_groups",
            "recording_notes",
            "recording_private_notes",
            "recording_speakers",
            "recording_final_passes",
        ):
            assert (
                db.execute(
                    f"SELECT count(*) FROM {table} WHERE recording=?", (recording,)
                ).fetchone()[0]
                == 0
            )
        assert (
            db.execute(
                "SELECT device_id FROM deleted_recordings WHERE recording=?", (recording,)
            ).fetchone()[0]
            == recorder["device_id"]
        )


async def test_assistant_reads_both_macs_and_revoked_recorders(hub):
    app, store, _, _ = hub
    first, second = device(store), device(store)
    recordings = [meeting(store, owner["device_id"], texts=TEXTS) for owner in (first, second)]
    for owner, recording in zip((first, second), recordings, strict=True):
        await write_note(store, owner["device_id"], recording)
    store.devices.revoke(first["device_id"])
    async with running_client(app) as http:
        body, headers = signed(
            LIST_PATH,
            {"device_id": second["device_id"], "since_ms": 0, "limit": 50},
            issued=int(NOW * 1000),
        )
        listed = await http.post(LIST_PATH, content=body, headers=headers)
        assert listed.status_code == 200, listed.text
        assert {row["recording_id"] for row in listed.json()["meetings"]} == set(recordings)
        for recording in recordings:
            for path in (NOTE_PATH, TRANSCRIPT_PATH):
                body, headers = signed(
                    path,
                    {"device_id": second["device_id"], "recording_id": recording},
                    issued=int(NOW * 1000),
                )
                response = await http.post(path, content=body, headers=headers)
                assert response.status_code == 200, response.text
                assert (
                    "Resumen corto." in response.text
                    if path == NOTE_PATH
                    else "Abrimos el comité." in response.json()["text"]
                )


async def test_edits_and_dictionaries_work_from_other_mac(hub):
    app, store, _, _ = hub
    recorder, reader = device(store), device(store)
    recording = meeting(store, recorder["device_id"], texts=TEXTS)
    await write_note(store, recorder["device_id"], recording)
    dictionary = str(uuid4())
    async with running_client(app) as http:
        headers = bearer(reader)
        for suffix, payload in (
            ("title", {"title": "Diseño revisado"}),
            ("private-notes", {"text": "Apunte de viaje", "base_revision": 0}),
            ("speakers", {"names": {"remote_app": "Celia"}}),
        ):
            response = await http.put(
                f"/v1/recordings/{recording}/{suffix}", json=payload, headers=headers
            )
            assert response.status_code == 200, response.text
        response = await http.post(
            "/v1/meeting-dictionaries/orders",
            headers=headers,
            json={
                "action": "correct",
                "recording_id": recording,
                "dictionary_id": dictionary,
                "name": "Diseño",
                "wrong": "comité",
                "correct": "equipo",
            },
        )
        assert response.status_code == 200, response.text
        for owner in (recorder, reader):
            saved = await http.get("/v1/meeting-dictionaries", headers=bearer(owner))
            assert saved.json()["dictionaries"][0]["id"] == dictionary
            opened = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
            assert opened["title"] == "Diseño revisado"
            assert opened["private_notes"]["text"] == "Apunte de viaje"
            assert "equipo" in (await all_turns(http, owner, recording))[0]["text"]
        note = (await http.get(f"/v1/recordings/{recording}/note", headers=headers)).json()
        task = note["note"]["action_items"][0]["id"]
        edited = await http.put(
            f"/v1/recordings/{recording}/note/edits",
            headers=headers,
            json={
                "base_revision": note["revision"],
                "done": {task: True},
            },
        )
        assert edited.status_code == 200, edited.text
        seen = (await http.get(f"/v1/recordings/{recording}", headers=bearer(recorder))).json()
        assert seen["note"]["note"]["action_items"][0]["done"] is True
        response = await http.put(
            f"/v1/recordings/{recording}/language", headers=headers, json={"language": "en"}
        )
        assert response.status_code == 200, response.text
        assert response.json()["requeued"] > 0
        response = await http.post(f"/v1/recordings/{recording}/final-pass", headers=headers)
        # Fixture has no diarization provider: the refusal must be configuration, not ownership.
        assert response.json()["refusal"]["code"] == "diarization_not_configured"


async def test_series_template_chosen_on_one_mac_applies_to_other_macs_recording(hub):
    app, store, model, _ = hub
    model.script("note", note_answer(5))
    recorder, reader = device(store), device(store)
    recording = meeting(
        store,
        recorder["device_id"],
        texts=TEXTS,
        event={"calendar_item_id": "weekly-design", "title": "Diseño semanal"},
    )
    async with running_client(app) as http:
        saved = await http.put(
            "/v1/note-template-links",
            headers=bearer(reader),
            json={
                "calendar_item_id": "weekly-design",
                "template_id": "project",
            },
        )
        assert saved.status_code == 200, saved.text
        requested = await http.post(
            f"/v1/recordings/{recording}/note", headers=bearer(reader), json={}
        )
        assert requested.status_code == 202, requested.text
        await ready_note(http, reader, recording)
        note = (await http.get(f"/v1/recordings/{recording}/note", headers=bearer(reader))).json()
        assert note["note"]["template_id"] == "project"
        opened = (await http.get(f"/v1/recordings/{recording}", headers=bearer(reader))).json()
        assert opened["event"]["calendar_item_id"] == "weekly-design"


async def test_revocation_does_not_prevent_requested_retranscription(hub):
    import asyncio

    import httpx

    app, store, _, _ = hub
    recorder, reader = device(store), device(store)
    recording = meeting(store, recorder["device_id"], texts=TEXTS)
    store.devices.revoke(recorder["device_id"])
    # No lifespan worker competes for the queued job; exercise HTTP admission
    # followed by the public worker claim, without a provider call.
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost"
    ) as http:
        response = await http.put(
            f"/v1/recordings/{recording}/language", headers=bearer(reader), json={"language": "en"}
        )
        assert response.status_code == 200, response.text
        assert response.json()["requeued"] > 0
    job = await asyncio.to_thread(store.groups.claim, NOW)
    assert job is not None, (
        "The person's queued meeting must run even if its recording Mac was revoked"
    )
    assert job["device_id"] == recorder["device_id"]


async def test_assistant_pages_both_macs_even_when_meetings_start_together(hub):
    app, store, _, _ = hub
    first, second = device(store), device(store)
    recordings = {meeting(store, owner["device_id"], texts=TEXTS) for owner in (first, second)}
    async with running_client(app) as http:
        request = {"device_id": second["device_id"], "since_ms": 0, "limit": 1}
        body, headers = signed(LIST_PATH, request, issued=int(NOW * 1000))
        first_page = (await http.post(LIST_PATH, content=body, headers=headers)).json()
        assert first_page["more"] is True
        last = first_page["meetings"][-1]
        request.update(before_ms=last["started_at_ms"], before_id=last["recording_id"])
        body, headers = signed(LIST_PATH, request, issued=int(NOW * 1000))
        response = await http.post(LIST_PATH, content=body, headers=headers)
        assert response.status_code == 200, response.text
        assert {last["recording_id"], response.json()["meetings"][0]["recording_id"]} == recordings
        assert response.json()["more"] is False


async def test_shared_calendar_context_preserves_explicit_event_times(hub):
    app, store, _, _ = hub
    recorder, reader = device(store), device(store)
    recording = meeting(
        store,
        recorder["device_id"],
        texts=TEXTS,
        event={
            "calendar_item_id": "archived-design",
            "title": "Diseño archivado",
            "starts_at_ms": 0,
            "ends_at_ms": 1,
        },
    )
    async with running_client(app) as http:
        response = await http.get(f"/v1/recordings/{recording}", headers=bearer(reader))
        assert response.status_code == 200, response.text
        assert response.json()["event"]["starts_at_ms"] == 0
        assert response.json()["event"]["ends_at_ms"] == 1
