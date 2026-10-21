"""SIM-40: the hub serves meetings and notes to the Mac, and Deimos reads the same.

The defects these cover (issues 1 to 4) came from the
Mac rebuilding what the hub already knew: absolute times, every voice called
«Hablante sin identificar», empty entries and «Idioma: Automático». Here the
hub computes the turns once, for the device routes and for Deimos's bridge,
and the tests read them through the real routes over the real store. The
model is a fake (`support_notes.FakeModel`); the meeting STT in the WebSocket
test is a loopback service this test serves. Nothing here reaches Hermes, a
real Whisper or a Mac.
"""

import asyncio
import hashlib
import io
import json
import struct
import wave
from contextlib import asynccontextmanager
from dataclasses import replace
from typing import ClassVar
from uuid import uuid4

import httpx
import pytest
from conftest import device
from fastapi import FastAPI, Request
from support_audio import wav_bytes
from support_client_e2e import tcp_server
from support_grouping import put
from support_notes import FakeModel
from support_waits import wait_for
from websockets.asyncio.client import connect

from orbit_hub.app import create_app
from orbit_hub.meetings.turns import read, render
from orbit_hub.meetings_bridge import PROTOCOL, TRANSCRIPT_PATH
from orbit_hub.notes.generation import clock
from orbit_hub.private_auth import sign_request

KEY = bytes(range(64, 96))
KEY_ID = "sim40-meetings-key"
NOW = 1_800_000_000.0
# The recording's own start, a real wall-clock instant: what made fallo 1 show
# «497829:12:38» when the Mac used it as the turn's time.
STARTED = 1_760_652_758_000
TITLE = "Comité de dirección"
TEN_SECONDS = 160_000  # frames at 16 kHz


def signed(path, payload, *, issued):
    body = json.dumps(payload, separators=(",", ":")).encode()
    nonce = str(uuid4())
    headers = {
        "content-type": "application/json",
        **sign_request(PROTOCOL, path, KEY_ID, KEY, body, issued_at_ms=issued, nonce=nonce),
    }
    return body, headers


def meeting(
    store,
    owner,
    *,
    texts,
    language="es",
    title=TITLE,
    kind=None,
    event=None,
    finalize=True,
):
    """Three chunks of ten seconds per track, so each track makes two groups.

    `texts[track]` are the two groups' texts in order. The remote track starts
    300 ms after the local one, as two capture sources do.
    """
    identity, epoch = str(uuid4()), str(uuid4())
    for offset, track in ((0, "mic_local"), (300, "remote_app")):
        for sequence in range(3):
            put(
                store,
                owner,
                identity,
                track,
                sequence,
                frames=TEN_SECONDS,
                started_at_ms=STARTED + offset + sequence * 10_000,
                capture_epoch_id=epoch,
            )
    if finalize:
        manifest = {
            "tracks": [
                {"track": track, "chunk_count": 3, "last_sequence": 2, "total_frames": 480_000}
                for track in ("mic_local", "remote_app")
            ],
            "gaps": 0,
            "started_at_ms": STARTED,
            "finalized_at_ms": STARTED + 30_000,
            "title": title,
        }
        if kind is not None:
            manifest["kind"] = kind
        if event is not None:
            manifest["event"] = event
        store.finalize_recording(owner, identity, manifest)
    pending = {track: list(values) for track, values in texts.items()}
    while job := store.claim_transcription(1000):
        track = job["manifest"]["track"]
        store.finish_transcription(job, pending[track].pop(0), language=language)
    return identity


TEXTS = {
    "mic_local": ["Abrimos el comité.", ""],
    "remote_app": ["Gracias, empezamos.", "Yo envío el acta."],
}


@pytest.fixture
def hub(environment, tmp_path):
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
    clock_now = {"now": NOW}
    app = create_app(
        settings,
        store=store,
        adapter=adapter,
        now=lambda: clock_now["now"],
        processing_client=model,
    )
    return app, store, model, clock_now


@asynccontextmanager
async def transcribing(environment, answer="Hablamos del presupuesto."):
    """A hub whose own worker transcribes against a loopback STT that answers."""
    from support_synthetic_stt import SyntheticStt

    settings, store, adapter = environment
    stt = SyntheticStt(answer)
    model = FakeModel()
    clock_now = {"now": NOW}
    async with tcp_server(stt.app) as stt_url:
        app = create_app(
            replace(
                settings,
                meeting_transcription={**stt.configuration(stt_url), "language": "es"},
            ),
            store=store,
            adapter=adapter,
            now=lambda: clock_now["now"],
            processing_client=model,
        )
        async with running_client(app) as http:
            yield app, http, model, clock_now


async def transcribed(http, paired, recording, *, budget=500):
    """Wait for the hub's own worker to finish the transcript."""
    for _ in range(budget):
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        if document["transcript"]["state"] in ("ready", "failed"):
            assert document["transcript"]["state"] == "ready", document["transcript"]
            return document
        await asyncio.sleep(0.01)
    raise AssertionError("la transcripción no terminó")


@asynccontextmanager
async def running_client(app):
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        yield http


def bearer(paired):
    return {"Authorization": "Bearer " + paired["token"]}


async def all_turns(http, paired, recording):
    turns, offset = [], 0
    while offset is not None:
        page = await http.get(
            f"/v1/recordings/{recording}/turns", params={"offset": offset}, headers=bearer(paired)
        )
        assert page.status_code == 200, page.text
        body = page.json()
        turns += body["turns"]
        offset = body["next_offset"]
    assert len(turns) == body["total"]
    return turns


def seen_by_the_device(document, turns):
    """What the Mac shows, line by line: the clock, the hub's name, the text."""
    names = {speaker["key"]: speaker["name"] for speaker in document["speakers"]}
    return "\n".join(
        f"[{clock(turn['at_ms'])}] {names[turn['speaker_key']]}: {turn['text']}" for turn in turns
    )


async def seen_by_deimos(http, paired, recording):
    body, headers = signed(
        TRANSCRIPT_PATH,
        {"device_id": paired["device_id"], "recording_id": recording, "format": "text"},
        issued=int(NOW * 1000),
    )
    answer = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
    assert answer.status_code == 200, answer.text
    return answer.json()["text"]


# Criterion 1: fallos 1 to 4, on the hub's side.


async def test_c1_turns_are_relative_named_never_empty_and_the_meeting_says_its_language(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        before = await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))
        assert before.status_code == 200, before.text
        # Before the Mac says who owns it, the local microphone is the track.
        assert {s["key"]: s["name"] for s in before.json()["speakers"]} == {
            "mic_local": "Micrófono local",
            "remote_app": "Otra persona",
        }
        profile = await http.put(
            "/v1/device/profile", json={"owner_name": "Alina"}, headers=bearer(paired)
        )
        assert profile.status_code == 200 and profile.json() == {"owner_name": "Alina"}
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        turns = await all_turns(http, paired, recording)

    assert document["title"] == TITLE
    assert (document["started_at_ms"], document["duration_ms"]) == (STARTED, 30_000)
    assert document["capture"] == "closed"
    assert document["language"] == "es"
    assert document["speakers"] == [
        {"key": "mic_local", "track": "mic_local", "name": "Alina"},
        {"key": "remote_app", "track": "remote_app", "name": "Otra persona"},
    ]
    assert document["transcript"] == {"state": "ready", "pending_chunks": 0, "failed_groups": []}
    assert document["private_notes"] == {"text": "", "revision": 0}
    assert document["note"] is None
    # Fallo 1: relative to the start, inside the recording.
    assert all(0 <= turn["at_ms"] <= document["duration_ms"] for turn in turns)
    # Fallo 3: the empty group of the local microphone is no turn.
    assert all(turn["text"].strip() for turn in turns)
    # The remote track's two groups are one turn: the local one between them
    # said nothing, and they start within a minute.
    assert turns == [
        {"at_ms": 0, "speaker_key": "mic_local", "text": "Abrimos el comité."},
        {
            "at_ms": 300,
            "speaker_key": "remote_app",
            "text": "Gracias, empezamos. Yo envío el acta.",
        },
    ]


async def test_c1_the_meeting_language_is_the_most_frequent_and_old_groups_read_as_configured(
    hub,
):
    app, store, _model, _clock = hub
    paired = device(store)
    english = meeting(
        store,
        paired["device_id"],
        texts={"mic_local": ["Hello.", "Again."], "remote_app": ["Hola.", "Sí."]},
        language=None,
    )
    with store.transaction() as db:
        db.execute(
            "UPDATE transcription_groups SET language='en' WHERE recording=? AND track='mic_local'",
            (english,),
        )
    async with running_client(app) as http:
        document = (await http.get(f"/v1/recordings/{english}", headers=bearer(paired))).json()
    # Two groups said «en»; the two written before the hub kept a language read
    # as the configured «es». A tie keeps the first heard.
    assert document["language"] == "en"


# Criterion 2: one rule, for the device and for Deimos.


async def test_c2_deimos_and_the_device_read_the_same_transcript(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        await http.put("/v1/device/profile", json={"owner_name": "Alina"}, headers=bearer(paired))
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        device_view = seen_by_the_device(document, await all_turns(http, paired, recording))
        deimos_view = await seen_by_deimos(http, paired, recording)
    assert deimos_view == device_view
    # And a question about the meeting (SIM-30's `meeting_context`) reads the same.
    with store.read_snapshot() as db:
        assert render(read(store, db, paired["device_id"], recording).lines()) == device_view
    assert deimos_view.splitlines() == [
        "[00:00:00] Alina: Abrimos el comité.",
        "[00:00:00] Otra persona: Gracias, empezamos. Yo envío el acta.",
    ]


# Criterion 3: renaming changes the turns and Deimos, never the written note.


async def test_c3_renaming_names_the_turns_and_deimos_and_leaves_the_note_as_written(hub):
    app, store, model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        await http.put("/v1/device/profile", json={"owner_name": "Alina"}, headers=bearer(paired))
        asked = await http.post(
            f"/v1/recordings/{recording}/note", json={"template_id": None}, headers=bearer(paired)
        )
        assert asked.status_code == 202, asked.text
        written = await ready_note(http, paired, recording)
        renamed = await http.put(
            f"/v1/recordings/{recording}/speakers",
            json={"names": {"remote_app": "Lucía"}},
            headers=bearer(paired),
        )
        assert renamed.status_code == 200, renamed.text
        assert renamed.json() == {
            "speakers": [
                {"key": "mic_local", "track": "mic_local", "name": "Alina"},
                {"key": "remote_app", "track": "remote_app", "name": "Lucía"},
            ]
        }
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        device_view = seen_by_the_device(document, await all_turns(http, paired, recording))
        deimos_view = await seen_by_deimos(http, paired, recording)
        unknown = await http.put(
            f"/v1/recordings/{recording}/speakers",
            json={"names": {"Speaker 9": "Pedro"}},
            headers=bearer(paired),
        )
        reset = await http.put(
            f"/v1/recordings/{recording}/speakers",
            json={"names": {"remote_app": None}},
            headers=bearer(paired),
        )

    assert "[00:00:00] Lucía: Gracias, empezamos. Yo envío el acta." in device_view.splitlines()
    assert deimos_view == device_view
    # The note already written is not rewritten: same revision, same text.
    assert document["note"]["revision"] == written["revision"]
    assert document["note"]["note"] == written["note"]
    assert len(model.calls) == 1
    assert unknown.status_code == 400
    assert unknown.json()["code"] == "unknown_speaker"
    assert unknown.json()["op"]
    assert reset.json()["speakers"][1]["name"] == "Otra persona"


async def ready_note(http, paired, recording, *, budget=200):
    for _ in range(budget):
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        if document["note"] is not None and document["note"]["state"] in ("ready", "failed"):
            assert document["note"]["state"] == "ready", document["note"]
            return document["note"]
        await asyncio.sleep(0.01)
    raise AssertionError("la nota no llegó a escribirse")


# Criterion 4: finalizing asks for the note with the meeting's title and the owner.


def manifest_body(**changes):
    return {
        "tracks": [
            {"track": track, "chunk_count": 3, "last_sequence": 2, "total_frames": 480_000}
            for track in ("mic_local", "remote_app")
        ],
        "gaps": 0,
        "started_at_ms": STARTED,
        "finalized_at_ms": STARTED + 30_000,
        "title": TITLE,
        **changes,
    }


async def test_c4_finalizing_a_meeting_asks_for_its_note_and_the_worker_writes_it(environment):
    _settings, store, _adapter = environment
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS, finalize=False)
    voice = meeting(store, paired["device_id"], texts=TEXTS, finalize=False)
    async with transcribing(environment) as (app, http, model, clock_now):
        await http.put("/v1/device/profile", json={"owner_name": "Alina"}, headers=bearer(paired))
        closed = await http.post(
            f"/v1/recordings/{recording}/finalize", json=manifest_body(), headers=bearer(paired)
        )
        assert closed.status_code == 200, closed.text
        voice_call = await http.post(
            f"/v1/recordings/{voice}/finalize",
            json=manifest_body(kind="voice_call", title=None),
            headers=bearer(paired),
        )
        assert voice_call.status_code == 200, voice_call.text
        await transcribed(http, paired, recording)
        await transcribed(http, paired, voice)
        clock_now["now"] += 11
        app.state.notes.signal.set()
        note = await ready_note(http, paired, recording)
        listing = (await http.get("/v1/recordings", headers=bearer(paired))).json()
        call_document = (await http.get(f"/v1/recordings/{voice}", headers=bearer(paired))).json()

    assert [call["purpose"] for call in model.calls] == ["note"]
    system, user = (message["content"] for message in model.calls[0]["messages"])
    assert f"Título previsto: {TITLE}" in user
    assert "notas privadas de Alina" in system
    assert "Alina: Hablamos del presupuesto." in user
    assert note["template_id"] is None and note["note"]["template_source"] == "default"
    # A voice call is not a meeting: no note, and not in the meetings list.
    assert call_document["note"] is None
    assert [row["recording_id"] for row in listing["meetings"]] == [recording]
    row = listing["meetings"][0]
    assert row == {
        "recording_id": recording,
        "title": TITLE,
        "started_at_ms": STARTED,
        "duration_ms": 30_000,
        "capture": "closed",
        "transcript": "ready",
        "note": {"state": "ready", "short_summary": "Resumen corto."},
    }
    assert listing["more"] is False


async def test_c4_the_event_in_the_manifest_reaches_the_note_and_picks_the_series_template(
    environment,
):
    _settings, store, _adapter = environment
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS, finalize=False)
    event = {
        "title": "Comité semanal",
        "starts_at_ms": STARTED,
        "ends_at_ms": STARTED + 3_600_000,
        "attendees": [{"name": "Lucía", "email": "lucia@example.test", "is_self": False}],
        "calendar_item_id": "serie-comite",
    }
    async with transcribing(environment) as (app, http, model, clock_now):
        linked = await http.put(
            "/v1/note-template-links",
            json={"calendar_item_id": "serie-comite", "template_id": "one_on_one"},
            headers=bearer(paired),
        )
        assert linked.status_code == 200, linked.text
        closed = await http.post(
            f"/v1/recordings/{recording}/finalize",
            json=manifest_body(event=event),
            headers=bearer(paired),
        )
        assert closed.status_code == 200, closed.text
        again = await http.post(
            f"/v1/recordings/{recording}/finalize",
            json=manifest_body(event=event),
            headers=bearer(paired),
        )
        assert again.status_code == 200, again.text
        await transcribed(http, paired, recording)
        clock_now["now"] += 11
        app.state.notes.signal.set()
        note = await ready_note(http, paired, recording)
    assert note["note"]["template_id"] == "one_on_one"
    assert note["note"]["template_source"] == "series"
    assert "Invitados: Lucía" in model.calls[0]["messages"][1]["content"]


async def test_c4_post_note_writes_it_again_with_another_template(hub):
    app, store, model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        first = await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        assert first.status_code == 202, first.text
        await ready_note(http, paired, recording)
        model.script(
            "note",
            {
                "title": "x",
                "short_summary": "Otra.",
                "sections": {f"s{i + 1}": "- y" for i in range(5)},
                "action_items": [],
                "decisions": [],
                "open_topics": [],
            },
        )
        again = await http.post(
            f"/v1/recordings/{recording}/note",
            json={"template_id": "project"},
            headers=bearer(paired),
        )
        assert again.status_code == 202, again.text
        note = await ready_note(http, paired, recording)
        legacy = await http.post(
            f"/v1/recordings/{recording}/note",
            json={"inputs": {"owner_name": "Alina"}},
            headers=bearer(paired),
        )
    assert note["template_id"] == "project" and note["note"]["template_id"] == "project"
    assert legacy.status_code == 400 and legacy.json()["code"] == "invalid_note_request"


# Criterion 6: private notes are the hub's, with a revision, and the note reads them.


async def test_c6_private_notes_refuse_an_old_revision_and_the_note_reads_them(hub):
    app, store, model, _clock = hub
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    path = f"/v1/recordings/{recording}/private-notes"
    async with running_client(app) as http:
        saved = await http.put(
            path,
            json={"text": "- Ojo con el presupuesto", "base_revision": 0},
            headers=bearer(paired),
        )
        stale = await http.put(
            path, json={"text": "- otra cosa", "base_revision": 0}, headers=bearer(paired)
        )
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))).json()
        await http.post(f"/v1/recordings/{recording}/note", json={}, headers=bearer(paired))
        await ready_note(http, paired, recording)
        too_long = await http.put(
            path,
            json={"text": "a" * (64 * 1024 + 1), "base_revision": 1},
            headers=bearer(paired),
        )
    assert saved.status_code == 200 and saved.json() == {"revision": 1}
    assert stale.status_code == 409, stale.text
    assert stale.json()["code"] == "stale_revision"
    assert stale.json()["refusal"]["revision"] == 1
    assert stale.json()["refusal"]["base_revision"] == 0
    assert document["private_notes"] == {"text": "- Ojo con el presupuesto", "revision": 1}
    assert "- Ojo con el presupuesto" in model.calls[0]["messages"][1]["content"]
    assert too_long.status_code == 400 and too_long.json()["refusal"]["max"] == 64 * 1024


# The routes' own refusals and bounds.


async def test_the_routes_refuse_other_devices_unknown_recordings_and_bad_input(hub):
    app, store, _model, _clock = hub
    paired, other = device(store), device(store, name="otro")
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running_client(app) as http:
        foreign = await http.get(f"/v1/recordings/{recording}", headers=bearer(other))
        missing = await http.get(f"/v1/recordings/{uuid4()}/turns", headers=bearer(paired))
        empty = await http.put(
            "/v1/device/profile", json={"owner_name": " "}, headers=bearer(paired)
        )
        long = await http.put(
            "/v1/device/profile", json={"owner_name": "M" * 129}, headers=bearer(paired)
        )
        limit = await http.get("/v1/recordings", params={"limit": 51}, headers=bearer(paired))
        offset = await http.get(
            f"/v1/recordings/{recording}/turns", params={"offset": -1}, headers=bearer(paired)
        )
    assert foreign.status_code == 404 and foreign.json()["code"] == "recording_not_found"
    assert missing.status_code == 404 and missing.json()["op"]
    assert empty.status_code == 400 and empty.json()["refusal"]["field"] == "owner_name"
    assert long.status_code == 400 and long.json()["refusal"]["max"] == 128
    assert limit.status_code == 400 and limit.json()["refusal"]["max"] == 50
    assert offset.status_code == 400 and offset.json()["refusal"]["field"] == "offset"


async def test_the_listing_pages_newest_first_and_continues_before_the_last(hub):
    app, store, _model, _clock = hub
    paired = device(store)
    recordings = []
    for minutes in range(3):
        identity = str(uuid4())
        put(store, paired["device_id"], identity, "mic_local", 0, started_at_ms=STARTED)
        store.finalize_recording(
            paired["device_id"],
            identity,
            {
                "tracks": [
                    {
                        "track": "mic_local",
                        "chunk_count": 1,
                        "last_sequence": 0,
                        "total_frames": 16_000,
                    }
                ],
                "gaps": 0,
                "started_at_ms": STARTED + minutes * 60_000,
                "finalized_at_ms": STARTED + minutes * 60_000 + 1_000,
                "title": f"Reunión {minutes}",
            },
        )
        recordings.append(identity)
    async with running_client(app) as http:
        first = (
            await http.get("/v1/recordings", params={"limit": 2}, headers=bearer(paired))
        ).json()
        rest = (
            await http.get(
                "/v1/recordings",
                params={"limit": 2, "before_ms": first["meetings"][-1]["started_at_ms"]},
                headers=bearer(paired),
            )
        ).json()
    assert [row["title"] for row in first["meetings"]] == ["Reunión 2", "Reunión 1"]
    assert first["more"] is True
    assert [row["title"] for row in rest["meetings"]] == ["Reunión 0"]
    assert rest["more"] is False
    assert rest["meetings"][0]["transcript"] == "pending" and rest["meetings"][0]["note"] is None


async def test_turn_pages_stay_under_their_byte_ceiling_and_lose_nothing(hub, monkeypatch):
    from orbit_hub.meetings import views

    app, store, _model, _clock = hub
    paired = device(store)
    recording = meeting(
        store,
        paired["device_id"],
        texts={"mic_local": ["uno " * 40, "dos " * 40], "remote_app": ["tres " * 40, "cuatro"]},
    )
    monkeypatch.setattr(views, "MAX_TURN_PAGE_BYTES", 400)
    async with running_client(app) as http:
        pages = []
        offset = 0
        while offset is not None:
            body = (
                await http.get(
                    f"/v1/recordings/{recording}/turns",
                    params={"offset": offset},
                    headers=bearer(paired),
                )
            ).json()
            pages.append(body)
            offset = body["next_offset"]
    assert len(pages) > 1
    assert all(len(page["turns"]) >= 1 for page in pages)
    assert sum(len(page["turns"]) for page in pages) == pages[0]["total"] == 4


# The language the transcriber says, or the configured one; never «auto».


def transcriber(answer, **configuration):
    from orbit_hub.meeting_transcription import MeetingTranscriber

    requests = []

    async def handle(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        return httpx.Response(200, json=answer)

    fields = {"endpoint": "http://127.0.0.1:0", "model": "openai/whisper-large-v3"}
    return (
        MeetingTranscriber(
            {**fields, **configuration},
            client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(handle)),
        ),
        requests,
    )


@pytest.mark.parametrize(
    ("answer", "configured", "expected"),
    [
        ({"text": "hola", "language": "es"}, None, "es"),
        ({"text": "hola", "language": "es-ES"}, "en", "es"),
        ({"text": "hola"}, "es", "es"),
        ({"text": "hola", "language": "auto"}, "es", "es"),
        ({"text": "hola", "language": "spanish"}, None, None),
        ({"text": "hola"}, None, None),
    ],
)
async def test_the_group_keeps_the_language_whisper_gives_else_the_configured_one(
    answer, configured, expected
):
    extra = {"language": configured} if configured else {}
    meetings, requests = transcriber(answer, **extra)
    result = await meetings.transcribe(wav_with(1))
    assert (result.text, result.language) == ("hola", expected)
    assert b'name="response_format"\r\n\r\nverbose_json' in requests[0].read()


# Criterion 5: one event per change, to the owning device's connections only.


def wav_with(marker: int, frames: int = 16_000) -> bytes:
    return wav_bytes(struct.pack("<h", marker * 1000) * frames)


class GatedStt:
    """A loopback meeting STT that answers each request only when the test says."""

    TEXTS: ClassVar[dict[int, str]] = {1: "Abrimos el comité.", 2: "Gracias, empezamos."}

    def __init__(self):
        self.app = FastAPI()
        self.gates = [asyncio.Event(), asyncio.Event()]
        self.served = 0

        @self.app.post("/v1/audio/transcriptions")
        async def transcriptions(request: Request):
            form = await request.form()
            audio = await form["file"].read()
            with wave.open(io.BytesIO(audio), "rb") as wav:
                (sample,) = struct.unpack("<h", wav.readframes(1))
                marker = sample // 1000
            gate = self.gates[self.served]
            self.served += 1
            await gate.wait()
            return {"text": self.TEXTS[marker], "language": "es"}


def upload_headers(paired, recording, track, sequence, body, marker_start):
    return {
        **bearer(paired),
        "Content-Type": "audio/wav",
        "X-Orbit-Recording": recording,
        "X-Orbit-Track": track,
        "X-Orbit-Sequence": str(sequence),
        "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
        "X-Orbit-Started-At-Ms": str(marker_start),
        "X-Orbit-Monotonic-Ns": str(1_000_000_000),
        "X-Orbit-First-Sample": "0",
        "X-Orbit-Frames": "16000",
        "X-Orbit-Sample-Rate": "16000",
        "X-Orbit-Channels": "1",
    }


async def test_c5_each_change_reaches_the_owning_device_and_no_other(environment, tmp_path):
    settings, store, adapter = environment
    stt = GatedStt()
    model = FakeModel()
    clock_now = {"now": NOW}
    paired, other = device(store), device(store, name="otro")
    recording = str(uuid4())
    async with tcp_server(stt.app) as stt_url:
        app = create_app(
            replace(
                settings,
                meeting_transcription={
                    "endpoint": stt_url,
                    "model": "openai/whisper-large-v3",
                    "language": "es",
                },
                allow_insecure_localhost=True,
                allowed_hosts=("127.0.0.1", "localhost"),
            ),
            store=store,
            adapter=adapter,
            now=lambda: clock_now["now"],
            processing_client=model,
        )
        async with tcp_server(app) as url, httpx.AsyncClient(base_url=url) as http:
            ws_url = url.replace("http://", "ws://") + "/v1/ws"
            async with (
                connect(ws_url, additional_headers=bearer(paired)) as mine,
                connect(ws_url, additional_headers=bearer(other)) as theirs,
            ):
                for socket in (mine, theirs):
                    assert json.loads(await socket.recv())["event"] == "ready"

                async def changed(what):
                    frame = json.loads(await wait_for(mine.recv(), what))
                    assert frame["event"] == "recording.changed", frame
                    assert frame["data"] == {"recording_id": recording}
                    return (
                        await http.get(f"/v1/recordings/{recording}", headers=bearer(paired))
                    ).json()

                for marker, track in ((1, "mic_local"), (2, "remote_app")):
                    body = wav_with(marker)
                    stored = await http.put(
                        f"/v1/chunks/{recording}-{track}",
                        content=body,
                        headers=upload_headers(paired, recording, track, 0, body, STARTED),
                    )
                    assert stored.status_code == 200, stored.text
                closed = await http.post(
                    f"/v1/recordings/{recording}/finalize",
                    json={
                        "tracks": [
                            {
                                "track": track,
                                "chunk_count": 1,
                                "last_sequence": 0,
                                "total_frames": 16_000,
                            }
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
                document = await changed("el aviso del cierre")
                assert document["capture"] == "closed"
                assert document["note"]["state"] == "waiting_transcript"

                stt.gates[0].set()
                document = await changed("el aviso del primer grupo transcrito")
                assert document["transcript"]["pending_chunks"] == 1
                assert len(await all_turns(http, paired, recording)) == 1

                stt.gates[1].set()
                document = await changed("el aviso del segundo grupo transcrito")
                assert document["transcript"]["state"] == "ready"

                clock_now["now"] += 11
                app.state.notes.signal.set()
                # Two: the note is being written, then it is written.
                await changed("el aviso de la nota escribiéndose")
                document = await changed("el aviso de la nota escrita")
                assert document["note"]["state"] == "ready"

                await http.put(
                    f"/v1/recordings/{recording}/speakers",
                    json={"names": {"remote_app": "Lucía"}},
                    headers=bearer(paired),
                )
                document = await changed("el aviso del renombre")
                assert document["speakers"][1]["name"] == "Lucía"

                await http.put(
                    f"/v1/recordings/{recording}/private-notes",
                    json={"text": "- acta", "base_revision": 0},
                    headers=bearer(paired),
                )
                document = await changed("el aviso de las notas privadas")
                assert document["private_notes"]["revision"] == 1

                # The other device heard nothing: its first frame is its own pong.
                await theirs.send(json.dumps({"v": 1, "id": str(uuid4()), "method": "ping"}))
                frame = json.loads(await wait_for(theirs.recv(), "el pong del otro dispositivo"))
                assert frame["event"] == "pong", frame
