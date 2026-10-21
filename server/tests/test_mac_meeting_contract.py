"""What the Mac sends and expects, checked against the real routes.

The closing manifests are the Mac's own (`contracts/fixtures/recording-manifest-*`,
written by `finalize.rs`); the voice notes are shaped as the daemon seals them
(one microphone chunk holding the whole note). No Mac, Hermes or provider runs.
"""

import json
from dataclasses import replace
from pathlib import Path
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_grouping import put
from test_meetings_bridge_routes import KEY, KEY_ID, LIST_PATH, NOW_MS, bearer, signed

from orbit_hub.app import create_app

FIXTURES = Path(__file__).resolve().parents[2] / "contracts" / "fixtures"


@pytest.fixture
def hub(environment, tmp_path):
    """The real app with Deimos's bridge, whose list shares the device's rows."""
    settings, store, adapter = environment
    key = tmp_path / "meetings-bridge.key"
    key.write_bytes(KEY)
    key.chmod(0o600)
    settings = replace(
        settings,
        meetings_bridge={"key_id": KEY_ID, "credential_file": str(key)},
    )
    return create_app(settings, store=store, adapter=adapter, now=lambda: NOW_MS / 1000), store


def app_client(app):
    return httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost")


def chunks_for(store, owner, manifest):
    recording = str(uuid4())
    for track in manifest["tracks"]:
        for sequence in range(track["chunk_count"]):
            put(store, owner, recording, track["track"], sequence)
    return recording


async def listed(http, owner, **params):
    answer = await http.get("/v1/recordings", params=params, headers=bearer(owner))
    assert answer.status_code == 200, answer.text
    return answer.json()


async def test_every_closing_manifest_the_mac_writes_is_accepted_with_its_kind_and_event(
    hub,
):
    app, store = hub
    owner = device(store)
    closed = {}
    async with app.router.lifespan_context(app), app_client(app) as http:
        for path in sorted(FIXTURES.glob("recording-manifest-*.json")):
            manifest = json.loads(path.read_text())
            recording = chunks_for(store, owner["device_id"], manifest)
            answer = await http.post(
                f"/v1/recordings/{recording}/finalize", json=manifest, headers=bearer(owner)
            )
            assert answer.status_code == 200, (path.name, answer.text)
            closed[path.name] = recording
        listing = await listed(http, owner)
        documents = {
            name: (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
            for name, recording in closed.items()
        }
    assert len(closed) == 5, sorted(closed)
    call = closed["recording-manifest-voice-call-v1.json"]
    # A voice call is not a meeting: not listed, and no note was asked for.
    assert {row["recording_id"] for row in listing["meetings"]} == set(closed.values()) - {call}
    assert documents["recording-manifest-voice-call-v1.json"]["note"] is None
    assert all(
        document["note"] is not None
        for name, document in documents.items()
        if name != "recording-manifest-voice-call-v1.json"
    )
    # The meeting's note is asked for with its event and series.
    with store.read_snapshot() as db:
        inputs = app.state.notes.repository.inputs(
            owner["device_id"], closed["recording-manifest-event-v1.json"], db=db
        )
    assert inputs.event.calendar_item_id == "serie-comite"
    assert [attendee.name for attendee in inputs.event.attendees] == ["Alina", "Ada"]
    assert inputs.title == "Comité de dirección"


async def test_a_voice_note_left_open_is_not_a_meeting_and_a_meeting_just_started_is(
    hub,
):
    """The voice notes earlier Macs uploaded and never closed.

    A meeting's chunk is sealed once it holds a second of audio and a capture
    frame holds at most one more (`capture.rs`), so it never holds two; a
    voice note is one chunk holding the whole note.
    """
    app, store = hub
    owner = device(store)
    voice, started = str(uuid4()), str(uuid4())
    put(store, owner["device_id"], voice, "mic_local", 0, frames=5 * 16_000)
    put(store, owner["device_id"], started, "mic_local", 0, frames=16_000)
    async with app.router.lifespan_context(app), app_client(app) as http:
        listing = await listed(http, owner)
        body, headers = signed(
            LIST_PATH, {"device_id": owner["device_id"], "since_ms": 0, "limit": 50}
        )
        deimos = await http.post(LIST_PATH, content=body, headers=headers)
    assert [row["recording_id"] for row in listing["meetings"]] == [started]
    assert deimos.status_code == 200, deimos.text
    assert [row["recording_id"] for row in deimos.json()["meetings"]] == [started]


async def test_meetings_that_start_together_are_all_listed_across_pages(hub):
    app, store = hub
    owner = device(store)
    recordings = sorted(str(uuid4()) for _ in range(3))
    for recording in recordings:
        put(store, owner["device_id"], recording, "mic_local", 0, started_at_ms=1_000_000)
        store.finalize_recording(
            owner["device_id"],
            recording,
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
                "started_at_ms": 1_000_000,
                "finalized_at_ms": 1_060_000,
            },
        )
    async with app.router.lifespan_context(app), app_client(app) as http:
        first = await listed(http, owner, limit=2)
        last = first["meetings"][-1]
        second = await listed(
            http,
            owner,
            limit=2,
            before_ms=last["started_at_ms"],
            before_id=last["recording_id"],
        )
    assert first["more"] is True
    assert [row["recording_id"] for row in first["meetings"]] == recordings[:2]
    assert [row["recording_id"] for row in second["meetings"]] == recordings[2:]
    assert second["more"] is False
