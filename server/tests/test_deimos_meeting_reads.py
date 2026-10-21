"""Deimos reads the new notes and the meetings still in progress.

Every test here runs the MCP tools Deimos calls against the real hub app over
loopback, with the real store, the real signed private routes and a note
written by the suite's fake model (`support_notes.FakeModel`). No Hermes, no Mac,
no provider and no fixed port.

The hub's clock is fixed at 1000 s so nothing moves on its own: chunks that the
test leaves unsealed stay pending (a group needs 20 s of audio or 30 s after
receipt), and a note that waits or retries is never picked up again. The
translator signs with the same clock.
"""

import json
import re
from contextlib import asynccontextmanager
from dataclasses import replace
from uuid import uuid4

from conftest import device
from support_client_e2e import tcp_server
from support_grouping import finalize, put
from support_notes import FakeModel, write_note
from support_orbit_mcp import DEFAULT_DEVICE, text_of, write_key, write_mcp_config

MEETINGS_KEY = bytes(range(96, 128))
MEETINGS_KEY_ID = "mcp-meetings-key"

from orbit_hub.app import create_app
from orbit_hub.meetings import repository as meetings
from orbit_hub.notes import NoteRepository
from orbit_hub.notes.models import EditRequest, Section
from orbit_hub.processing import ProcessingError
from orbit_mcp.config import Config
from orbit_mcp.hub import HubClient
from orbit_mcp.server import build_server

NOW = 1000.0
PRIVATE_NOTES = "- preguntar por el presupuesto de diciembre"


def transcribe_everything(store, text_by_track: dict[str, str], now: float = NOW + 30) -> None:
    """Seal and finish every group that is due at `now`, with a text per track."""
    while job := store.claim_transcription(now):
        store.finish_transcription(job, text_by_track[job["manifest"]["track"]])


def label_remote_groups(store, owner: str, recording: str, label: str) -> None:
    """What diarization writes into a finished group: a `Speaker N` label."""
    with store.transaction() as db:
        rows = db.execute(
            "SELECT id, manifest FROM transcription_groups "
            "WHERE device_id=? AND recording=? AND track='remote_app'",
            (owner, recording),
        ).fetchall()
        for row in rows:
            manifest = json.loads(store.decrypt(row["manifest"], f"group:{owner}:{row['id']}"))
            manifest["speaker"] = label
            sealed = store.encrypt(json.dumps(manifest).encode(), f"group:{owner}:{row['id']}")
            db.execute(
                "UPDATE transcription_groups SET manifest=? WHERE device_id=? AND id=?",
                (sealed, owner, row["id"]),
            )


def finished_meeting(store, owner, *, title="Comité de dirección", text_by_track=None):
    identity = str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(2):
            put(store, owner, identity, track, sequence)
    finalize(store, owner, identity, 2)
    with store.transaction() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner, identity),
        ).fetchone()
        manifest = json.loads(store.decrypt(row["body"], f"manifest:{owner}:{identity}"))
        manifest["title"] = title
        db.execute(
            "UPDATE recording_manifests SET body=? WHERE device_id=? AND recording=?",
            (
                store.encrypt(json.dumps(manifest).encode(), f"manifest:{owner}:{identity}"),
                owner,
                identity,
            ),
        )
    transcribe_everything(
        store,
        text_by_track
        or {"mic_local": "Abrimos el comité.", "remote_app": "Yo envío el acta el viernes."},
    )
    return identity


@asynccontextmanager
async def deimos(environment, tmp_path, owner):
    """The real hub, served on an ephemeral port, and the translator pointed at it."""
    settings, store, adapter = environment
    hub_key = write_key(tmp_path, "hub-meetings.key", MEETINGS_KEY)
    settings = replace(
        settings,
        meetings_bridge={
            "key_id": MEETINGS_KEY_ID,
            "credential_file": str(hub_key),
        },
        allow_insecure_localhost=True,
        allowed_hosts=("127.0.0.1", "localhost"),
    )
    app = create_app(
        settings, store=store, adapter=adapter, now=lambda: NOW, processing_client=FakeModel()
    )
    async with tcp_server(app) as url:
        path = write_mcp_config(tmp_path, url, devices={DEFAULT_DEVICE: owner})
        meetings_key = write_key(tmp_path, "mcp-meetings.key", MEETINGS_KEY)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(
                f'\n[meetings]\nkey_id = "{MEETINGS_KEY_ID}"\nkey_file = "{meetings_key}"\n'
            )
        config = Config.load(path)
        yield build_server(
            config, HubClient(config, clock=lambda: int(NOW * 1000)), clock=lambda: NOW
        )


async def test_orbit_note_returns_the_note_with_the_user_edits_applied(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = finished_meeting(store, owner)
    worker = await write_note(
        store, owner, identity, owner_name="Alina", private_notes=PRIVATE_NOTES
    )
    document = worker.repository.document(owner, identity)
    task = document["note"]["action_items"][0]["id"]
    edited_sections = (
        Section(name="Resumen", markdown="- Alina reescribió este punto"),
        Section(name="Siguientes pasos", markdown="- cerrar el presupuesto"),
    )
    worker.repository.edit(
        owner,
        identity,
        EditRequest(
            base_revision=document["revision"],
            title="Comité, versión de Alina",
            sections=edited_sections,
            done={task: True},
        ),
        NOW,
    )

    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_note", {"device": DEFAULT_DEVICE, "recording_id": identity}
        )

    text = text_of(result)
    assert not result.is_error, text
    # The note as the user sees it: their title and their sections, not the model's.
    assert "Comité, versión de Alina" in text
    assert "Alina reescribió este punto" in text and "cerrar el presupuesto" in text
    assert "- punto 1" not in text
    assert "Quien usa Orbit editó el título y las secciones" in text
    # The model's own parts that the user did not touch.
    assert "Resumen corto." in text
    assert "[hecha] Lucía: Enviar el acta" in text
    assert "Aprobado" in text
    assert "Temas abiertos" in text and "Ninguno." in text
    # Their private notes, the template, the model and the date it was written.
    assert f"Notas privadas de Alina\n{PRIVATE_NOTES}" in text
    assert "plantilla General" in text
    assert "qwen3.8-flash-next" in text
    assert "1970-01-01 00:16 UTC" in text


async def test_orbit_note_of_a_failed_note_names_the_cause_with_its_numbers(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = finished_meeting(store, owner)
    await write_note(
        store,
        owner,
        identity,
        answers=[
            ProcessingError(
                "incomplete_completion", completion_tokens=8192, max_completion_tokens=8192
            )
        ],
    )

    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_note", {"device": DEFAULT_DEVICE, "recording_id": identity}
        )
        never = await server.call_tool(
            "orbit_note", {"device": DEFAULT_DEVICE, "recording_id": finished_meeting(store, owner)}
        )

    text = text_of(result)
    assert result.is_error
    assert text.startswith("note_failed: ")
    assert "8192 tokens" in text
    assert "1 intento" in text
    # Absent is not «no»: a note nobody asked for is not an empty note.
    assert never.is_error
    assert text_of(never).startswith("note_not_requested: ")
    assert "orbit_transcript" in text_of(never)


async def test_a_note_waiting_for_its_retry_says_when(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = finished_meeting(store, owner)
    await write_note(
        store,
        owner,
        identity,
        answers=[ProcessingError("provider_timeout", timeout_seconds=300)],
    )

    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_note", {"device": DEFAULT_DEVICE, "recording_id": identity}
        )

    text = text_of(result)
    assert result.is_error
    assert text.startswith("note_queued: ")
    assert "300 s" in text
    # The retry is thirty seconds after the failure, at 1030 s.
    assert "1970-01-01 00:17:10 UTC" in text


async def test_orbit_meetings_lists_the_meeting_in_progress_and_each_notes_state(
    environment, tmp_path
):
    _, store, _ = environment
    owner = device(store)["device_id"]
    written = finished_meeting(store, owner, title="Comité con nota")
    await write_note(store, owner, written, owner_name="Alina")
    unrequested = finished_meeting(store, owner, title="Reunión sin nota")
    failed = finished_meeting(store, owner, title="Reunión con nota fallida")
    await write_note(
        store, owner, failed, answers=[ProcessingError("provider_rejected_credential", status=401)]
    )
    live = str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(3):
            put(store, owner, live, track, sequence, started_at_ms=2_000_000 + sequence * 1000)

    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_meetings", {"device": DEFAULT_DEVICE})

    text = text_of(result)
    assert not result.is_error, text
    lines = {line.split(" · ")[0]: line for line in text.splitlines() if line.startswith("- ")}
    # The one in progress is listed, first, and says it is not closed.
    in_progress = next(line for line in text.splitlines() if live in line)
    assert in_progress.startswith("- EN CURSO")
    assert "el Mac aún no la ha cerrado" in in_progress
    assert "3 s de audio recibido" in in_progress
    assert text.splitlines()[1] == in_progress
    # Each finished one with the real state of its note.
    ready = next(line for line in lines.values() if written in line)
    assert "nota lista: «Comité» — Resumen corto." in ready
    absent = next(line for line in lines.values() if unrequested in line)
    assert "nota sin pedir" in absent
    broken = next(line for line in lines.values() if failed in line)
    assert "nota fallida" in broken and "HTTP 401" in broken


async def test_orbit_transcript_of_a_meeting_in_progress_is_partial_and_uses_the_notes_names(
    environment, tmp_path
):
    _, store, _ = environment
    owner = device(store)["device_id"]
    live = str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(3):
            put(store, owner, live, track, sequence)
    transcribe_everything(store, {"mic_local": "Hola, empezamos.", "remote_app": "Vale, adelante."})
    # Two more seconds per track arrived and are not transcribed yet.
    for track in ("mic_local", "remote_app"):
        for sequence in range(3, 5):
            put(store, owner, live, track, sequence)
    repository = NoteRepository(store)
    repository.initialize()
    # The local microphone is named by the device's owner, from its profile.
    meetings.set_owner_name(store, owner, "Alina", NOW)
    repository.request(owner, live, NOW, True)

    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": live}
        )

    text = text_of(result)
    assert not result.is_error, text
    first = text.splitlines()[0]
    assert first.startswith("Transcripción parcial: la reunión sigue en curso")
    assert "4 fragmentos" in first and "2 s de audio" in first
    assert "Alina: Hola, empezamos." in text
    assert "Otra persona: Vale, adelante." in text
    assert "Speaker" not in text and "mic_local" not in text


async def test_a_finished_transcript_names_diarized_voices_as_the_device_does(
    environment, tmp_path
):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = finished_meeting(store, owner)
    label_remote_groups(store, owner, identity, "Speaker 1")
    unnamed = finished_meeting(store, owner)
    label_remote_groups(store, owner, unnamed, "Speaker 1")

    async with deimos(environment, tmp_path, owner) as server:
        before = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": unnamed}
        )
    # The names are the hub's, the ones the Mac shows: the device's
    # owner for the local microphone and the user's names for the voices.
    await write_note(store, owner, identity, owner_name="Alina", speakers={"Speaker 1": "Lucía"})
    async with deimos(environment, tmp_path, owner) as server:
        named = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": identity}
        )
        labelled = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": unnamed}
        )

    assert not named.is_error, text_of(named)
    assert "Alina: Abrimos el comité." in text_of(named)
    assert "Lucía: Yo envío el acta el viernes." in text_of(named)
    assert "Speaker 1" not in text_of(named)
    # A voice the user has not named is «Otra persona», never the recognizer's label.
    assert "Otra persona: Yo envío el acta el viernes." in text_of(labelled)
    assert "Alina: Abrimos el comité." in text_of(labelled)
    assert "Speaker 1" not in text_of(labelled)
    # Before the Mac says who owns it, the local microphone is the track, and
    # the answer says why.
    assert "Micrófono local: Abrimos el comité." in text_of(before)
    assert text_of(before).splitlines()[0].startswith("Hablantes sin nombre")


async def test_bounds_a_long_transcript_is_paged_for_deimos_with_its_numbers(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    sentence = "Seguimos hablando del presupuesto con muchísimo detalle. "
    identity = finished_meeting(
        store,
        owner,
        text_by_track={"mic_local": sentence * 1000, "remote_app": sentence * 1000},
    )

    async with deimos(environment, tmp_path, owner) as server:
        first = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": identity}
        )
        header = text_of(first).splitlines()[0]
        assert header.startswith("Página: caracteres 0–")
        following = int(header.rsplit("after_index=", 1)[1].rstrip("."))
        second = await server.call_tool(
            "orbit_transcript",
            {"device": DEFAULT_DEVICE, "recording_id": identity, "after_index": following},
        )
        beyond = await server.call_tool(
            "orbit_transcript",
            {"device": DEFAULT_DEVICE, "recording_id": identity, "after_index": 10**9},
        )

    # What protects Deimos: Hermes keeps a tool result whole up to 50 000 characters.
    assert len(text_of(first)) <= 50_000
    assert not second.is_error
    assert f"Página: caracteres {following}–" in text_of(second).splitlines()[0]
    assert beyond.is_error
    assert text_of(beyond).startswith("after_index_out_of_range: ")
    assert "1000000000" in text_of(beyond)


async def test_deimos_reconstructs_a_transcript_of_at_least_three_pages(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    local = " ".join(f"Local {i:04d}: café, revisión y presupuesto." for i in range(2000))
    remote = " ".join(f"Remoto {i:04d}: mañana envío el acta 🎙." for i in range(2000))
    identity = finished_meeting(
        store, owner, text_by_track={"mic_local": local, "remote_app": remote}
    )
    meetings.set_owner_name(store, owner, "Alina", NOW)
    # Fixture audio starts at 900000 ms; the recording starts at 1000 ms.
    expected = f"[00:14:59] Alina: {local}\n[00:14:59] Otra persona: {remote}"
    pages, cursor = [], 0
    async with deimos(environment, tmp_path, owner) as server:
        for _ in range(10):
            result = await server.call_tool(
                "orbit_transcript",
                {"device": DEFAULT_DEVICE, "recording_id": identity, "after_index": cursor},
            )
            rendered = text_of(result)
            assert not result.is_error, rendered
            assert len(rendered) <= 50_000
            header, separator, text = rendered.partition("\n")
            assert separator
            match = re.fullmatch(r"Página: caracteres (\d+)–(\d+) de (\d+)(.*)", header)
            assert match, header
            start, end, total = map(int, match.group(1, 2, 3))
            assert start == cursor and cursor < end <= total
            assert total == len(expected)
            assert len(text) == end - start
            pages.append(text)
            if match[4] == "; es la última.":
                assert end == total
                break
            continuation = re.fullmatch(r"\. Sigue con after_index=(\d+)\.", match[4])
            assert continuation, header
            cursor = int(continuation[1])
            assert cursor == end
        else:
            assert False, "The fixture's transcript did not end within ten pages"
    assert len(pages) >= 3
    reconstructed = "".join(pages)
    assert len(reconstructed) == len(expected)
    assert all(actual == wanted for actual, wanted in zip(reconstructed, expected, strict=True))


def failed_meeting(store, owner, *, partial=False, pending=False):
    identity = str(uuid4())
    tracks = ("mic_local", "remote_app") if partial else ("mic_local",)
    for track in tracks:
        put(store, owner, identity, track, 0)
    finalize(store, owner, identity, 1, tracks=tracks)
    if partial:
        job = store.claim_transcription(1000)
        assert job["manifest"]["track"] == "mic_local"
        store.finish_transcription(job, "Texto completado de micrófono.")
    for now in (1000, 1010) if pending else (1000, 1010, 1030, 1070, 1150):
        job = store.claim_transcription(now)
        assert job is not None
        store.fail_transcription(
            job, "transcription_timeout_300000" if pending else "transcription_http_503", now
        )
    return identity


async def test_deimos_receives_failed_transcription_cause(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = failed_meeting(store, owner)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_transcript", {"recording_id": identity})
    assert result.is_error
    message = text_of(result)
    assert "HTTP 503" in message
    assert "5 intentos" in message
    assert "no se reintentará automáticamente" in message.lower()


async def test_deimos_receives_completed_text_and_missing_track_cause(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = failed_meeting(store, owner, partial=True)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_transcript", {"recording_id": identity})
    assert not result.is_error
    message = text_of(result)
    assert "Texto completado de micrófono." in message
    assert "remote_app, fragmentos 0–0" in message
    assert "HTTP 503" in message
    assert "5 intentos" in message
    assert "no se reintentará automáticamente" in message.lower()


async def test_deimos_receives_pending_retry_details(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = failed_meeting(store, owner, pending=True)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_transcript", {"recording_id": identity})
    assert result.is_error
    message = text_of(result)
    assert "300000 ms" in message
    assert "2 intentos" in message
    assert "1970-01-01 00:17:10 UTC" in message
    assert "ms Unix" not in message


async def test_a_long_note_is_paged_without_losing_sections_or_private_notes(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = finished_meeting(store, owner)
    worker = await write_note(store, owner, identity, private_notes="p" * 65_536)
    document = worker.repository.document(owner, identity)
    sections = tuple(
        Section(name=f"Bloque {i}", markdown=f"- detalle {i} " + "x" * 16_000) for i in range(12)
    )
    worker.repository.edit(
        owner,
        identity,
        EditRequest(
            base_revision=document["revision"],
            title="Planificación anual",
            sections=sections,
        ),
        NOW,
    )
    pages, after = [], 0
    async with deimos(environment, tmp_path, owner) as server:
        for _ in range(10):
            result = await server.call_tool(
                "orbit_note", {"recording_id": identity, "after_index": after}
            )
            assert not result.is_error, text_of(result)
            pages.append(text_of(result))
            marker = next(line for line in pages[-1].splitlines() if line.startswith("Página:"))
            if "after_index=" not in marker:
                break
            after = int(marker.rsplit("after_index=", 1)[1].rstrip("."))
    assert len(pages) > 1
    assert all(len(page) <= 50_000 for page in pages)
    joined = "".join(pages)
    assert all(f"- detalle {i} " in joined for i in range(12))
    assert joined.count("p") >= 65_536
    assert "Planificación anual" in pages[0].splitlines()[1]


async def test_the_largest_listing_fits_whole_and_says_how_to_continue(environment, tmp_path):
    from support_notes import note_answer

    _, store, _ = environment
    owner = device(store)["device_id"]
    for index in range(21):
        identity = finished_meeting(store, owner, title="T" * 256)
        with store.transaction() as db:
            row = db.execute(
                "SELECT body FROM recording_manifests WHERE recording=?", (identity,)
            ).fetchone()
            manifest = json.loads(store.decrypt(row["body"], f"manifest:{owner}:{identity}"))
            manifest.update(
                started_at_ms=1000 + index * 10_000, finalized_at_ms=14_401_000 + index * 10_000
            )
            db.execute(
                "UPDATE recording_manifests SET body=? WHERE recording=?",
                (
                    store.encrypt(json.dumps(manifest).encode(), f"manifest:{owner}:{identity}"),
                    identity,
                ),
            )
        answer = {**note_answer(), "title": "N" * 200, "short_summary": "S" * 1000}
        await write_note(store, owner, identity, answers=[answer])
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_meetings", {})
    assert not result.is_error, text_of(result)
    assert len(text_of(result)) <= 50_000
    assert len(result.content) == 2
    assert all(block.type == "text" for block in result.content)
    assert "portatil" in result.content[1].text and "fichero" in result.content[1].text
    text = result.content[0].text
    assert text.count("S" * 1000) == 20
    assert "before_ms=11000" in text.splitlines()[-1]
