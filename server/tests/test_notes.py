"""The note of a recording: requested by the Mac, written when the transcript is complete."""

import asyncio
from concurrent.futures import ThreadPoolExecutor
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_grouping import finalize, put
from support_notes import FakeModel, configuration, note_answer

from orbit_hub.app import create_app
from orbit_hub.meetings import repository as meetings
from orbit_hub.notes import NoteRepository, NotesWorker
from orbit_hub.processing import ProcessingError
from orbit_hub.protocol import HubError

NOW = 2000.0


def recording(
    store, owner, *, count=2, text="Hablamos del presupuesto.", transcribed=True, final=True
):
    identity, epoch = str(uuid4()), str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(count):
            put(store, owner, identity, track, sequence, capture_epoch_id=epoch)
    if final:
        finalize(store, owner, identity, count)
    if transcribed:
        while job := store.claim_transcription(1000):
            store.finish_transcription(job, text)
    return identity


@pytest.fixture
def notes(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    executor = ThreadPoolExecutor(max_workers=1)
    model = FakeModel()
    clock = {"now": NOW}
    worker = NotesWorker(
        NoteRepository(store),
        model.configuration,
        lambda: clock["now"],
        signal=asyncio.Event(),
        store_executor=executor,
        client=model,
    )
    worker.repository.initialize()
    yield worker, store, owner, model, clock
    executor.shutdown()


async def step(worker, clock):
    """Run the worker once on whatever is due, as its loop would."""
    job = await worker._run(worker.repository.claim, clock["now"])
    if job is not None:
        await worker.generate(job)
    return job


def request(worker, owner, recording_id, *, owner_name=None, template_id=None):
    """Ask for the note as the hub does; the owner's name is the device's profile (SIM-40)."""
    if owner_name is not None:
        meetings.set_owner_name(worker.repository.store, owner, owner_name, NOW)
    return worker.repository.request(
        owner, recording_id, NOW, worker.configured, template_id=template_id
    )


async def test_a_note_waits_for_the_transcript_then_is_written(notes):
    worker, store, owner, model, clock = notes
    identity = recording(store, owner, transcribed=False)
    assert request(worker, owner, identity, owner_name="Alina")["state"] == "waiting_transcript"
    await step(worker, clock)
    document = worker.repository.document(owner, identity)
    assert (document["state"], model.calls) == ("waiting_transcript", [])
    assert document["next_attempt_ms"] == int((NOW + 10) * 1000)
    while job := store.claim_transcription(1000):
        store.finish_transcription(job, "Hablamos del presupuesto.")
    clock["now"] += 10
    await step(worker, clock)
    document = worker.repository.document(owner, identity)
    assert document["state"] == "ready" and document["revision"] == 1
    assert document["note"]["sections"][0]["name"] == "Resumen"
    assert document["note"]["action_items"][0]["owner"] == "Lucía"
    assert [call["purpose"] for call in model.calls] == ["note"]
    transcript = model.calls[0]["messages"][1]["content"]
    assert "Alina: Hablamos del presupuesto." in transcript
    assert "Otra persona: Hablamos del presupuesto." in transcript


async def test_a_long_meeting_is_written_by_parts_and_joined(notes):
    worker, store, owner, model, clock = notes
    worker.client.configuration = configuration(context_tokens=4096, max_completion_tokens=256)
    identity = recording(store, owner, count=12, text="palabra " * 900)
    request(worker, owner, identity)
    await step(worker, clock)
    purposes = [call["purpose"] for call in model.calls]
    assert purposes[-1] == "note_reduce" and purposes.count("note_part") >= 2
    document = worker.repository.document(owner, identity)
    assert document["state"] == "ready"
    assert document["note"]["parts"] == purposes.count("note_part")


async def test_a_transient_failure_is_retried_later_with_its_cause(notes):
    worker, store, owner, model, clock = notes
    identity = recording(store, owner)
    model.script("note", ProcessingError("provider_timeout", timeout_seconds=300))
    request(worker, owner, identity)
    await step(worker, clock)
    document = worker.repository.document(owner, identity)
    assert document["state"] == "queued"
    assert document["error"] == {"code": "provider_timeout", "timeout_seconds": 300, "attempt": 1}
    assert document["next_attempt_ms"] == int((NOW + 30) * 1000)
    clock["now"] += 30
    await step(worker, clock)
    assert worker.repository.document(owner, identity)["state"] == "ready"


async def test_a_failure_that_needs_a_person_is_reported_once_with_its_numbers(notes):
    worker, store, owner, model, clock = notes
    identity = recording(store, owner)
    model.script(
        "note",
        ProcessingError(
            "incomplete_completion", completion_tokens=8192, max_completion_tokens=8192
        ),
    )
    request(worker, owner, identity)
    await step(worker, clock)
    clock["now"] += 3600
    assert await step(worker, clock) is None
    document = worker.repository.document(owner, identity)
    assert document["state"] == "failed"
    assert document["error"] == {
        "code": "incomplete_completion",
        "completion_tokens": 8192,
        "max_completion_tokens": 8192,
        "attempt": 1,
    }


async def test_an_empty_transcript_is_not_sent_to_the_model(notes):
    worker, store, owner, model, clock = notes
    identity = recording(store, owner, text="   ")
    request(worker, owner, identity)
    await step(worker, clock)
    document = worker.repository.document(owner, identity)
    assert document["error"]["code"] == "transcript_empty" and model.calls == []


async def test_without_a_processing_model_the_note_says_so(notes):
    worker, store, owner, _model, clock = notes
    worker.client = None
    identity = recording(store, owner)
    assert request(worker, owner, identity)["state"] == "not_configured"
    assert await step(worker, clock) is None


async def test_an_unknown_template_is_named(notes):
    worker, store, owner, _model, clock = notes
    identity = recording(store, owner)
    request(worker, owner, identity, template_id="no-existe")
    await step(worker, clock)
    error = worker.repository.document(owner, identity)["error"]
    assert error == {"code": "note_template_not_found", "template_id": "no-existe", "attempt": 1}


def test_a_recording_of_another_device_is_not_found(notes):
    worker, store, owner, _model, _clock = notes
    other = device(store, name="otro")["device_id"]
    identity = recording(store, other)
    with pytest.raises(HubError) as caught:
        request(worker, owner, identity)
    assert caught.value.code == "recording_not_found"


async def test_edits_need_the_current_revision(notes):
    worker, store, owner, _model, clock = notes
    identity = recording(store, owner)
    request(worker, owner, identity)
    await step(worker, clock)
    from orbit_hub.notes.models import EditRequest

    edited = worker.repository.edit(
        owner, identity, EditRequest(base_revision=1, title="Mi título"), NOW
    )
    assert edited["revision"] == 2 and edited["edits"]["title"] == "Mi título"
    with pytest.raises(HubError) as caught:
        worker.repository.edit(owner, identity, EditRequest(base_revision=1, title="Otro"), NOW)
    assert caught.value.code == "note_revision_changed"
    assert "revisión 2" in caught.value.message and "de 1" in caught.value.message


async def test_regenerating_keeps_done_marks_and_sets_text_edits_aside(notes):
    worker, store, owner, model, clock = notes
    identity = recording(store, owner)
    request(worker, owner, identity)
    await step(worker, clock)
    from orbit_hub.notes.models import EditRequest

    task = worker.repository.document(owner, identity)["note"]["action_items"][0]["id"]
    worker.repository.edit(
        owner,
        identity,
        EditRequest(base_revision=1, title="Mío", done={task: True}),
        NOW,
    )
    regenerated = worker.repository.request(owner, identity, NOW, True, template_id="project")
    assert regenerated["state"] == "waiting_transcript"
    assert regenerated["superseded_edits"] == {"title": "Mío", "sections": None}
    model.script("note", note_answer(sections=5))
    await step(worker, clock)
    document = worker.repository.document(owner, identity)
    assert document["note"]["template_id"] == "project"
    assert document["edits"] == {"done": {task: True}}


async def test_a_note_being_written_when_the_hub_stopped_is_written_again(notes):
    worker, store, owner, _model, clock = notes
    identity = recording(store, owner)
    request(worker, owner, identity)
    worker.repository.claim(NOW)
    assert worker.repository.recover_interrupted(NOW) == 1
    await step(worker, clock)
    assert worker.repository.document(owner, identity)["state"] == "ready"


async def test_the_routes_request_read_and_edit_a_note(environment):
    settings, store, adapter = environment
    model = FakeModel()
    app = create_app(settings, store=store, adapter=adapter, processing_client=model)
    owner = device(store)
    identity = recording(store, owner["device_id"])
    headers = {"Authorization": "Bearer " + owner["token"]}
    async with app.router.lifespan_context(app):
        transport = httpx.ASGITransport(app=app)
        async with httpx.AsyncClient(transport=transport, base_url="https://localhost") as client:
            await client.put(
                f"/v1/recordings/{identity}/private-notes",
                json={"text": "- presupuesto", "base_revision": 0},
                headers=headers,
            )
            answer = await client.post(f"/v1/recordings/{identity}/note", json={}, headers=headers)
            assert answer.status_code == 202
            for _ in range(200):
                document = (
                    await client.get(f"/v1/recordings/{identity}/note", headers=headers)
                ).json()
                if document["state"] == "ready":
                    break
                await asyncio.sleep(0.01)
            assert document["state"] == "ready"
            assert "- presupuesto" in model.calls[0]["messages"][1]["content"]
            edited = await client.put(
                f"/v1/recordings/{identity}/note/edits",
                json={"base_revision": document["revision"], "title": "Nuevo"},
                headers=headers,
            )
            assert edited.json()["edits"]["title"] == "Nuevo"
            wrong = await client.post(
                f"/v1/recordings/{identity}/note",
                json={"template_id": "No vale"},
                headers=headers,
            )
            assert wrong.status_code == 400
            assert "template_id" in wrong.json()["message"]
            templates = await client.get("/v1/note-templates", headers=headers)
            assert {t["id"] for t in templates.json()["templates"]} >= {"general", "one_on_one"}
            missing = await client.get(f"/v1/recordings/{uuid4()}/note", headers=headers)
            assert missing.status_code == 404


async def test_notes_asked_without_a_model_are_written_once_one_is_configured(notes):
    worker, store, owner, _model, clock = notes
    client, worker.client = worker.client, None
    identity = recording(store, owner)
    assert request(worker, owner, identity)["state"] == "not_configured"
    worker.client = client
    worker.start()
    try:
        document = worker.repository.document(owner, identity)
        assert document["state"] == "waiting_transcript"
        assert document["revision"] == 1
        await step(worker, clock)
        assert worker.repository.document(owner, identity)["state"] == "ready"
    finally:
        await worker.close()
