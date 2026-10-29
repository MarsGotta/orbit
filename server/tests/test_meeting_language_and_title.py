"""A meeting's language and its name, on the hub.

The language: Whisper receives the recording's own language (the configured
one until the user picks another); picking another on a finished meeting
repeats every group in it with the audio the hub keeps, then writes the note
again in it, with the user's edits set aside. The name: the user's name for a
meeting wins over
the manifest's in the list, the document, Deimos's list and the next note, and
the note already written is left as it is.

Everything runs through the real routes over the real store and the real
workers. The note model is a fake (`support_notes.FakeModel`) and the meeting
STT a loopback service this test serves; nothing reaches Hermes, a real
Whisper or a Mac.
"""

import asyncio
import json
from contextlib import asynccontextmanager
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device
from fastapi import FastAPI, Request
from support_client_e2e import tcp_server
from support_grouping import put
from support_notes import FakeModel
from support_waits import wait_for
from test_meeting_views import (
    KEY,
    KEY_ID,
    NOW,
    STARTED,
    TEXTS,
    TITLE,
    all_turns,
    bearer,
    meeting,
    ready_note,
    running_client,
    signed,
    transcriber,
    wav_with,
)
from websockets.asyncio.client import connect

from orbit_hub.app import create_app
from orbit_hub.meetings_bridge import LIST_PATH

ENGLISH = "We go over the budget."


class LanguageStt:
    """A loopback meeting STT that keeps the `language` each request carried.

    It answers only once `gate` is open, so a test reads the meeting while its
    groups are being repeated without racing the worker.
    """

    def __init__(self, text=ENGLISH, *, open_gate=True):
        self.app = FastAPI()
        self.languages: list[str | None] = []
        self.gate = asyncio.Event()
        if open_gate:
            self.gate.set()

        @self.app.post("/v1/audio/transcriptions")
        async def transcriptions(request: Request):
            form = await request.form()
            language = form.get("language")
            self.languages.append(language)
            await self.gate.wait()
            # Whisper names its language in words in `verbose_json`.
            return {"text": text, "language": "english" if language == "en" else "spanish"}


@pytest.fixture
def hub(environment, tmp_path):
    """The hub: Spanish configured, Deimos's bridge on, a fake note model."""
    settings, store, adapter = environment
    key = tmp_path / "meetings.key"
    key.write_bytes(KEY)
    key.chmod(0o600)
    settings = replace(
        settings,
        meeting_transcription={**settings.meeting_transcription.model_dump(), "language": "es"},
        meetings_bridge={"key_id": KEY_ID, "credential_file": str(key)},
    )
    model = FakeModel()
    app = create_app(
        settings,
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: NOW,
        processing_client=model,
    )
    return app, store, model, None


@asynccontextmanager
async def hub_with(environment, stt):
    """A hub whose own workers transcribe against `stt` and write notes with a fake."""
    settings, store, adapter = environment
    model = FakeModel()
    clock_now = {"now": NOW}
    async with tcp_server(stt.app) as stt_url:
        app = create_app(
            replace(
                settings,
                meeting_transcription={
                    "endpoint": stt_url,
                    "model": "openai/whisper-large-v3",
                    "language": "es",
                },
            ),
            store=store,
            assistant=adapter,
            speech=adapter,
            now=lambda: clock_now["now"],
            processing_client=model,
        )
        async with running_client(app) as http:
            yield app, http, model, clock_now


@asynccontextmanager
async def routes_only(app):
    """The routes without the workers: a test that claims groups itself owns them."""
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost"
    ) as http:
        yield http


async def document_of(http, paired, recording):
    answer = await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))
    assert answer.status_code == 200, answer.text
    return answer.json()


async def until_transcribed(http, paired, recording, *, budget=500):
    for _ in range(budget):
        document = await document_of(http, paired, recording)
        if document["transcript"]["state"] == "ready" and not document["transcript"].get(
            "retranscribing"
        ):
            return document
        await asyncio.sleep(0.01)
    raise AssertionError("the transcript did not finish")


def groups(store, recording):
    with store.read_snapshot() as db:
        return [
            dict(row)
            for row in db.execute(
                "SELECT state, language, attempts FROM transcription_groups WHERE recording=? "
                "ORDER BY first_sequence, track",
                (recording,),
            ).fetchall()
        ]


# Criterion 1a: a recording has the default language and Whisper receives it.


async def test_whisper_receives_the_recordings_language_else_the_configured_one():
    meetings, requests = transcriber({"text": "hello", "language": "english"}, language="es")
    chosen = await meetings.transcribe(wav_with(1), "en")
    default = await meetings.transcribe(wav_with(1))
    assert b'name="language"\r\n\r\nen' in requests[0].read()
    assert b'name="language"\r\n\r\nes' in requests[1].read()
    # Whisper names the language in words: the group is in the one it was sent.
    assert chosen.language == "en"
    assert default.language == "es"


async def test_a_new_recording_reads_the_default_language_and_whisper_receives_it(environment):
    _settings, store, _adapter = environment
    paired = device(store)
    recording = str(uuid4())
    for track in ("mic_local", "remote_app"):
        put(store, paired["device_id"], recording, track, 0, started_at_ms=STARTED)
    stt = LanguageStt("Seguimos.", open_gate=False)
    async with hub_with(environment, stt) as (_app, http, _model, _clock):
        # Nothing transcribed yet: the meeting is in the configured language.
        document = await document_of(http, paired, recording)
        stt.gate.set()
        closed = await http.post(
            f"/v1/recordings/{recording}/finalize",
            json={
                "tracks": [
                    {"track": track, "chunk_count": 1, "last_sequence": 0, "total_frames": 16_000}
                    for track in ("mic_local", "remote_app")
                ],
                "gaps": 0,
                "started_at_ms": STARTED,
                "finalized_at_ms": STARTED + 1_000,
                "title": TITLE,
            },
            headers=bearer(paired),
        )
        assert closed.status_code == 200, closed.text
        await until_transcribed(http, paired, recording)
    assert document["language"] == "es"
    assert stt.languages == ["es", "es"]


# Criterion 1b: English on a finished meeting repeats every group and the note.


async def test_english_on_a_finished_meeting_repeats_every_group_and_writes_the_note_in_english(
    environment,
):
    _settings, store, _adapter = environment
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    stt = LanguageStt(open_gate=False)
    async with hub_with(environment, stt) as (app, http, model, clock_now):
        asked = await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        assert asked.status_code == 202, asked.text
        written = await ready_note(http, paired, recording)
        edited = await http.put(
            f"/v1/recordings/{recording}/note/edits",
            json={
                "base_revision": written["revision"],
                "sections": [{"name": "Resumen", "markdown": "- lo mío"}],
            },
            headers=bearer(paired),
        )
        assert edited.status_code == 200, edited.text
        before = await all_turns(http, paired, recording)

        changed = await http.put(
            f"/v1/recordings/{recording}/language", json={"language": "en"}, headers=bearer(paired)
        )
        assert changed.status_code == 200, changed.text
        during = await document_of(http, paired, recording)
        # What was transcribed stays readable while it is repeated.
        kept = await all_turns(http, paired, recording)
        stt.gate.set()

        done = await until_transcribed(http, paired, recording)
        clock_now["now"] += 11
        app.state.notes.signal.set()
        note = await ready_note(http, paired, recording)
        after = await all_turns(http, paired, recording)

    assert changed.json() == {"language": "en", "requeued": 4}
    assert during["language"] == "en"
    assert during["transcript"]["retranscribing"] == {"language": "en", "done": 0, "total": 4}
    assert during["note"]["state"] == "waiting_transcript"
    assert kept == before
    # Every group again, each with `language=en`; the empty one too.
    assert stt.languages == ["en"] * 4
    assert [group["language"] for group in groups(store, recording)] == ["en"] * 4
    assert done["language"] == "en"
    assert done["transcript"].get("retranscribing") is None
    # The empty group said something this time. Both groups of each track
    # read together, retaining all four groups' words in two complete turns.
    assert [turn["speaker_key"] for turn in after] == ["mic_local", "remote_app"]
    assert [turn["text"] for turn in after] == [f"{ENGLISH} {ENGLISH}"] * 2
    # The note again, in English, and the user's edits set aside.
    assert len(model.calls) == 2
    assert "Write in English" in model.calls[-1]["messages"][0]["content"]
    assert ENGLISH in model.calls[-1]["messages"][1]["content"]
    assert note["superseded_available"] is True
    assert note["edited"] == {"sections": False}


async def test_the_progress_counts_groups_done_of_those_repeated(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with routes_only(app) as http:
        await http.put(
            f"/v1/recordings/{recording}/language", json={"language": "en"}, headers=bearer(paired)
        )
        job = store.groups.claim(NOW + 1)
        store.groups.finish(job, "One.", "en")
        document = await document_of(http, paired, recording)
    assert job["language"] == "en"
    assert document["transcript"]["retranscribing"] == {"language": "en", "done": 1, "total": 4}
    assert document["transcript"]["state"] == "pending"


# Criterion 1c: the same language repeats nothing; missing audio says why.


async def test_the_same_language_repeats_nothing(hub):
    app, store, model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        written = await ready_note(http, paired, recording)
        before = groups(store, recording)
        same = await http.put(
            f"/v1/recordings/{recording}/language", json={"language": "es"}, headers=bearer(paired)
        )
        document = await document_of(http, paired, recording)
    assert same.status_code == 200, same.text
    assert same.json() == {"language": "es", "requeued": 0}
    assert groups(store, recording) == before
    assert document["transcript"]["state"] == "ready"
    assert document["transcript"].get("retranscribing") is None
    assert document["note"]["revision"] == written["revision"]
    assert len(model.calls) == 1


async def test_audio_the_hub_cannot_read_refuses_with_its_cause_and_changes_nothing(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    # A group whose chunks do not form a WAV any more (`grouping.assemble_wav`).
    with store.transaction() as db:
        db.execute(
            "UPDATE transcription_groups SET state='failed', error_code='invalid_group_audio' "
            "WHERE recording=? AND track='remote_app' AND first_sequence=2",
            (recording,),
        )
    before = groups(store, recording)
    async with running_client(app) as http:
        refused = await http.put(
            f"/v1/recordings/{recording}/language", json={"language": "en"}, headers=bearer(paired)
        )
        document = await document_of(http, paired, recording)
    assert refused.status_code == 409, refused.text
    body = refused.json()
    assert body["code"] == "meeting_audio_unreadable"
    assert body["refusal"]["groups"] == 1 and body["refusal"]["total"] == 4
    assert body["refusal"]["cause"] == "invalid_group_audio"
    assert "1 of 4" in body["message"]
    assert groups(store, recording) == before
    assert document["language"] == "es"


async def test_the_language_route_refuses_what_is_not_a_language_or_not_a_recording(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        auto = await http.put(
            f"/v1/recordings/{recording}/language",
            json={"language": "auto"},
            headers=bearer(paired),
        )
        missing = await http.put(
            f"/v1/recordings/{uuid4()}/language", json={"language": "en"}, headers=bearer(paired)
        )
    assert auto.status_code == 400 and auto.json()["refusal"]["field"] == "language"
    assert missing.status_code == 404 and missing.json()["code"] == "recording_not_found"


# A meeting in progress: what is left goes in the new language, what was done again.


async def test_during_a_recording_what_is_left_and_what_was_done_go_in_the_new_language(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS, finalize=False)
    transcribed = [group for group in groups(store, recording) if group["state"] == "completed"]
    async with routes_only(app) as http:
        changed = await http.put(
            f"/v1/recordings/{recording}/language", json={"language": "en"}, headers=bearer(paired)
        )
    languages = []
    # Later, the chunks still waiting make a group by their deadline.
    while job := store.groups.claim(NOW + 1):
        languages.append(job["language"])
        store.groups.finish(job, "Go on.", "en")
    assert changed.json() == {"language": "en", "requeued": len(transcribed)}
    assert len(transcribed) == 2
    # The two repeated and the two never transcribed.
    assert languages == ["en"] * 4


# Criterion 1d: renaming changes the list, the document and Deimos, not the note.


async def deimos_list(http, paired):
    body, headers = signed(
        LIST_PATH,
        {"device_id": paired["device_id"], "since_ms": 0, "limit": 10},
        issued=int(NOW * 1000),
    )
    answer = await http.post(LIST_PATH, content=body, headers=headers)
    assert answer.status_code == 200, answer.text
    return answer.json()["meetings"]


async def test_renaming_names_the_meeting_everywhere_and_leaves_the_written_note(hub):
    app, store, model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    path = f"/v1/recordings/{recording}/title"
    async with running_client(app) as http:
        await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        written = await ready_note(http, paired, recording)
        renamed = await http.put(
            path, json={"title": "  Budget  \u00a0 review 2027 "}, headers=bearer(paired)
        )
        listed = (await http.get("/v1/recordings", headers=bearer(paired))).json()["meetings"]
        document = await document_of(http, paired, recording)
        for_deimos = await deimos_list(http, paired)
        again = await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        assert again.status_code == 202, again.text
        await ready_note(http, paired, recording)
        reset = await http.put(path, json={"title": ""}, headers=bearer(paired))
        back = await document_of(http, paired, recording)

    assert renamed.status_code == 200, renamed.text
    assert renamed.json() == {"title": "Budget review 2027"}
    assert [row["title"] for row in listed] == ["Budget review 2027"]
    assert document["title"] == "Budget review 2027"
    assert [row["title"] for row in for_deimos] == ["Budget review 2027"]
    # The note already written stays as it was written.
    assert document["note"]["revision"] == written["revision"]
    assert document["note"]["note"] == written["note"]
    # The next note is asked with the user's name.
    assert len(model.calls) == 2
    assert "Expected title: Budget review 2027" in model.calls[-1]["messages"][1]["content"]
    # An empty title gives the meeting back its own.
    assert reset.json() == {"title": TITLE}
    assert back["title"] == TITLE


async def test_a_title_is_one_line_within_the_meetings_ceiling(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    path = f"/v1/recordings/{recording}/title"
    async with running_client(app) as http:
        longest = await http.put(path, json={"title": "a" * 256}, headers=bearer(paired))
        too_long = await http.put(path, json={"title": "a" * 257}, headers=bearer(paired))
        control = await http.put(path, json={"title": "uno\ndos"}, headers=bearer(paired))
        missing = await http.put(
            f"/v1/recordings/{uuid4()}/title", json={"title": "x"}, headers=bearer(paired)
        )
    assert longest.status_code == 200
    assert too_long.status_code == 400
    assert too_long.json()["refusal"]["max"] == 256
    assert too_long.json()["refusal"]["actual"] == 257
    assert control.status_code == 400 and control.json()["refusal"]["field"] == "title"
    assert missing.status_code == 404 and missing.json()["code"] == "recording_not_found"


# Each change reaches the owning device as `recording.changed`, as the other meeting changes do.


async def test_a_rename_and_a_language_change_are_announced_to_the_owner(environment):
    settings, store, adapter = environment
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    app = create_app(
        replace(settings, allow_insecure_localhost=True, allowed_hosts=("127.0.0.1", "localhost")),
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: NOW,
        processing_client=FakeModel(),
    )
    async with tcp_server(app) as url, httpx.AsyncClient(base_url=url) as http:
        ws_url = url.replace("http://", "ws://") + "/v1/ws"
        async with connect(ws_url, additional_headers=bearer(paired)) as mine:
            assert json.loads(await mine.recv())["event"] == "ready"
            for path, body in (
                ("title", {"title": "Otro nombre"}),
                ("language", {"language": "en"}),
            ):
                answer = await http.put(
                    f"/v1/recordings/{recording}/{path}", json=body, headers=bearer(paired)
                )
                assert answer.status_code == 200, answer.text
                frame = json.loads(await wait_for(mine.recv(), f"el aviso de {path}"))
                assert frame["event"] == "recording.changed", frame
                assert frame["data"] == {"recording_id": recording}
