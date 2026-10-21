"""SIM-93: manual final passes through HTTP, using only synthetic meetings."""

from dataclasses import replace

import pytest
from conftest import device
from support_notes import FakeModel, write_note
from test_final_pass import fake_separator, fake_whisper, final_worker
from test_meeting_language_and_title import routes_only
from test_meeting_views import TEXTS, bearer, meeting

from orbit_hub.app import create_app
from orbit_hub.meeting_final_pass import FinalPassRepository
from orbit_hub.meetings import repository


def app_for(environment, tmp_path, *, configured=True):
    settings, store, adapter = environment
    return create_app(
        replace(settings, diarization=fake_separator(tmp_path) if configured else None),
        store=store,
        adapter=adapter,
        now=lambda: 1000,
        processing_client=FakeModel(),
    )


@pytest.mark.parametrize("previous", [None, "completed", "failed", "queued", "running"])
async def test_manual_final_pass_queues_terminal_states_and_keeps_active_passes(
    environment, tmp_path, previous
):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    if previous is not None:
        with store.transaction() as db:
            db.execute(
                "INSERT INTO recording_final_passes(device_id,recording,state) VALUES (?,?,?)",
                (owner["device_id"], recording, previous),
            )
    async with routes_only(app_for(environment, tmp_path)) as http:
        response = await http.post(f"/v1/recordings/{recording}/final-pass", headers=bearer(owner))
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
        turns = (await http.get(f"/v1/recordings/{recording}/turns", headers=bearer(owner))).json()
    assert response.status_code == 200, response.text
    assert document["transcript"]["final_pass"]["state"] == (
        "running" if previous == "running" else "queued"
    )
    assert [t["text"] for t in turns["turns"]] == [
        "Abrimos el comité.",
        "Gracias, empezamos. Yo envío el acta.",
    ]
    assert document["note"] is None


async def test_manual_final_pass_rewrites_requested_note_once_after_new_text(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)
    identity = owner["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    note_worker = await write_note(store, identity, recording)
    notes = note_worker.repository
    note_worker.executor = None
    notes.request(identity, recording, 1000, True, template_id="one_on_one")
    await note_worker.generate(notes.claim(1000))
    before = notes.document(identity, recording)
    with store.read_snapshot() as db:
        template = notes.inputs(identity, recording, db=db).template_id
    app = app_for(environment, tmp_path)
    async with routes_only(app) as http:
        path = f"/v1/recordings/{recording}/final-pass"
        assert (await http.post(path, headers=bearer(owner))).status_code == 200
        pending = notes.document(identity, recording)
        assert pending["state"] == "waiting_transcript"
        assert pending["revision"] == before["revision"] + 1
        with store.read_snapshot() as db:
            assert notes.inputs(identity, recording, db=db).template_id == template
        assert notes.claim(1000) is None
        assert (await http.post(path, headers=bearer(owner))).status_code == 200
        assert notes.document(identity, recording)["revision"] == pending["revision"]
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, fake_separator(tmp_path)).run_once()
    job = notes.claim(2000)
    assert job is not None
    note_worker.executor = None
    await note_worker.generate(job)
    after = notes.document(identity, recording)
    assert after["state"] == "ready"
    assert after["revision"] > pending["revision"]
    assert after["note"]["template_id"] == "one_on_one"
    prompt = note_worker.client.calls[-1]["messages"][-1]["content"]
    assert "Uno." in prompt and "Gracias, empezamos." not in prompt


@pytest.mark.parametrize(
    "cause,numbers",
    [
        ("recording_still_open", {}),
        ("meeting_audio_unreadable", {"groups": 1, "total": 4}),
        ("diarization_not_configured", {}),
    ],
)
async def test_manual_final_pass_refuses_with_cause_and_numbers(
    environment, tmp_path, cause, numbers
):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(
        store, owner["device_id"], texts=TEXTS, finalize=cause != "recording_still_open"
    )
    if cause == "meeting_audio_unreadable":
        with store.transaction() as db:
            db.execute(
                "UPDATE transcription_groups SET state='failed',error_code='invalid_group_audio' "
                "WHERE id=(SELECT id FROM transcription_groups LIMIT 1)"
            )
    async with routes_only(
        app_for(environment, tmp_path, configured=cause != "diarization_not_configured")
    ) as http:
        response = await http.post(f"/v1/recordings/{recording}/final-pass", headers=bearer(owner))
    assert response.status_code == (400 if cause == "recording_still_open" else 409), response.text
    assert response.json()["code"] == cause
    refusal = response.json().get("refusal", {})
    assert "closed" not in refusal
    assert "configured" not in refusal
    for key, value in numbers.items():
        assert response.json()["refusal"][key] == value
    assert store._db.execute("SELECT count(*) FROM recording_final_passes").fetchone()[0] == 0


async def test_obsolete_voice_names_are_reported_without_reassignment(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)
    identity = owner["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    repository.rename(
        store,
        identity,
        recording,
        {"mic_local": "Alina", "remote_app": "Lucía"},
        {"mic_local", "remote_app"},
        1000,
    )
    FinalPassRepository(store).request(identity, recording)
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, fake_separator(tmp_path)).run_once()
    async with routes_only(app_for(environment, tmp_path)) as http:
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
    assert document["transcript"]["unmatched_speaker_names"] == 1
    assert [(s["key"], s["name"]) for s in document["speakers"]] == [
        ("mic_local", "Alina"),
        ("voice_1", "Persona 1"),
        ("voice_2", "Persona 2"),
    ]
    # A name explicitly merged into another live voice is still assigned.
    keys = {"mic_local", "voice_1", "voice_2"}
    repository.rename(store, identity, recording, {"voice_2": "Pedro"}, keys, 1000)
    repository.rename(store, identity, recording, {"voice_1": "Pedro"}, keys, 1000)
    async with routes_only(app_for(environment, tmp_path)) as http:
        merged = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
    assert merged["transcript"]["unmatched_speaker_names"] == 1


async def test_running_state_is_announced_before_the_final_text_replaces_the_old(
    environment, tmp_path
):
    from orbit_hub.meeting_final_pass import status
    from orbit_hub.meetings import turns

    _, store, _ = environment
    owner = device(store)["device_id"]
    recording = meeting(store, owner, texts=TEXTS)
    repo = FinalPassRepository(store)
    repo.request(owner, recording)
    transcriber, _ = fake_whisper()
    worker = final_worker(store, transcriber, fake_separator(tmp_path))
    assert await worker.run_once()
    with store.read_snapshot() as db:
        old = turns.read(store, db, owner, recording)
    # A failed replacement must keep the successful transcript as well as the voices.
    repo.request(owner, recording)
    broken, _ = fake_whisper(missing_words=True)
    worker.transcriber = broken
    announced = []

    def changed(device, identity):
        with store.read_snapshot() as db:
            announced.append(
                (status(db, device, identity), turns.read(store, db, device, identity))
            )

    worker.changed = changed
    assert await worker.run_once()
    assert [state["state"] for state, _ in announced] == ["running", "failed"]
    assert announced[-1][0]["error"]["code"] == "transcription_words_missing"
    assert all(transcript == old for _, transcript in announced)
