"""Manual final passes through HTTP, using only synthetic meetings."""

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
        assistant=adapter,
        speech=adapter,
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


async def test_manual_final_pass_keeps_the_note_and_rewriting_it_keeps_the_edited_version(
    environment, tmp_path
):
    from orbit_hub.notes.models import EditRequest, Section

    _, store, _ = environment
    owner = device(store)
    identity = owner["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    note_worker = await write_note(store, identity, recording)
    notes = note_worker.repository
    note_worker.executor = None
    notes.request(identity, recording, 1000, True, template_id="one_on_one")
    await note_worker.generate(notes.claim(1000))
    written = notes.document(identity, recording)
    edited_by_hand = (Section(name="Resumen", markdown="Lo que escribí a mano."),)
    edited = notes.edit(
        identity,
        recording,
        EditRequest(base_revision=written["revision"], sections=edited_by_hand),
        1000,
    )
    app = app_for(environment, tmp_path)
    async with routes_only(app) as http:
        path = f"/v1/recordings/{recording}/final-pass"
        for _ in range(2):
            assert (await http.post(path, headers=bearer(owner))).status_code == 200
            # Transcribing again does not touch the note, nor ask for a new one.
            assert notes.document(identity, recording) == edited
            assert notes.claim(1000) is None
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, fake_separator(tmp_path)).run_once()
    assert notes.document(identity, recording) == edited
    assert notes.claim(2000) is None

    # «Rehacer resumen» is its own order; the version edited by hand is kept.
    async with routes_only(app) as http:
        rewrite = await http.post(
            f"/v1/recordings/{recording}/note",
            headers=bearer(owner),
            json={"template_id": "one_on_one"},
        )
    assert rewrite.status_code == 202, rewrite.text
    job = notes.claim(2000)
    assert job is not None
    note_worker.executor = None
    await note_worker.generate(job)
    after = notes.document(identity, recording)
    assert after["state"] == "ready"
    assert after["revision"] > edited["revision"]
    assert after["note"]["template_id"] == "one_on_one"
    assert after["superseded_edits"]["sections"] == [s.model_dump() for s in edited_by_hand]
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
        ("voice_1", "Person 1"),
        ("voice_2", "Person 2"),
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


async def test_a_running_pass_reports_its_progress_and_the_time_it_has_left(environment, tmp_path):
    from orbit_hub.meeting_final_pass import status

    _, store, _ = environment
    owner = device(store)["device_id"]
    recording = meeting(store, owner, texts=TEXTS)
    repo = FinalPassRepository(store)
    repo.request(owner, recording)
    with store.read_snapshot() as db:
        # Queued: no work done yet, so no fraction and no estimate.
        assert status(db, owner, recording) == {"state": "queued", "error": None}
    transcriber, _ = fake_whisper()
    ticks = iter(range(0, 10_000, 10))
    worker = final_worker(store, transcriber, fake_separator(tmp_path))
    worker.clock = lambda: float(next(ticks))
    announced = []

    def changed(device, identity):
        with store.read_snapshot() as db:
            announced.append(status(db, device, identity))

    worker.changed = changed
    assert await worker.run_once()
    running = [state for state in announced if state["state"] == "running"]
    # Started, then after each track's only 30-second window (two tracks).
    progress = [state.get("progress") for state in running]
    assert progress[0] == {"done_ms": 0, "total_ms": 60_000}
    # The microphone's 30 s, then the remote track's 30.3 s (it starts 300 ms
    # later); each track's estimate becomes its measured length once read.
    assert [(p["done_ms"], p["total_ms"]) for p in progress[1:]] == [
        (30_000, 60_000),
        (60_300, 60_300),
    ]
    # The time left is the time taken so far, scaled to what is left.
    first = progress[1]
    taken_ms = first["remaining_ms"] * first["done_ms"] / (first["total_ms"] - first["done_ms"])
    assert taken_ms > 0
    assert progress[-1]["remaining_ms"] == 0
    assert announced[-1] == {"state": "completed", "error": None}
