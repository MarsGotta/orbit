"""What the Mac sends and expects, checked against a hub run as a program.

The closing manifests are the Mac's own (`contracts/fixtures/recording-manifest-*`,
written by `finalize.rs`); the voice notes are shaped as the daemon seals them
(one microphone chunk holding the whole note). Each test has a hub of its own,
with no database: the listing holds the meetings of every device of the hub,
and the tests compare exact lists. The audio goes over `PUT /v1/chunks/{id}`
with its capture headers. A meeting's note is checked by what the invented
notes model is asked: the hub names there the expected title, the attendees in
their order and the template of the event's series, the series' only effect.
"""

import hashlib
import io
import json
import secrets
import uuid
import wave
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import NamedTuple

import httpx
import pytest
from kit_hub import (
    KIT_DEVICE_NAME,
    MEETING_MONOTONIC_NS,
    MEETING_SAMPLE_RATE,
    MEETINGS_PROTOCOL,
    _answered,
    _ModelHandler,
    _service,
    device_client,
    pair_device,
    prepare,
    run,
    serving,
    signed_headers,
    transcription_service,
    wait_for_note,
)

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
# The size of the storage key a hub reads from its key file.
STORAGE_KEY_BYTES = 32
MEETINGS_LIST_PATH = "/internal/v1/meetings/list"
# A chunk as the daemon seals a meeting's: a second of audio.
CHUNK_FRAMES = 16_000
# When the first chunk of a recording starts.
FIRST_STARTED_AT_MS = 900_000
# The series of the Mac's manifest with an event, and the invented template
# linked to it.
SERIES = "serie-comite"
TEMPLATE_ID = "kit-series-template"
TEMPLATE_NAME = "Plantilla inventada del comité"
NOTE_TEXT = "The contract kit wrote this invented note"
TRANSCRIPT_TEXT = "The contract kit transcribed this invented meeting"


class MacHub(NamedTuple):
    url: str
    device_id: str
    http: httpx.Client
    configuration: Path
    # Every request the notes model was sent, in order.
    asked: list[dict]


@contextmanager
def watched_model(text: str) -> Iterator[tuple[str, list[dict]]]:
    """The kit's invented notes model, writing down every request it is sent;
    yields its `chat/completions` URL and that list."""
    asked: list[dict] = []

    class Handler(_ModelHandler):
        def do_POST(self) -> None:
            raw = self.rfile.read(int(self.headers.get("content-length", 0)))
            try:
                asked.append(json.loads(raw))
            except ValueError:
                pass
            self.rfile = io.BytesIO(raw)
            super().do_POST()

    with _service(Handler, text) as url:
        yield f"{url}/v1/chat/completions", asked


@pytest.fixture
def mac_hub(tmp_path) -> Iterator[MacHub]:
    """A new hub, without a database, transcribing with the invented service
    and writing notes with the invented model; as a device the kit pairs."""
    with (
        transcription_service(TRANSCRIPT_TEXT) as transcription,
        watched_model(NOTE_TEXT) as (model, asked),
    ):
        configuration = prepare(
            tmp_path / "hub",
            secrets.token_bytes(STORAGE_KEY_BYTES),
            transcription=transcription,
            model=model,
        )
        code = run(configuration, "pair-code").strip()
        with serving(configuration) as url:
            paired = pair_device(url, code, KIT_DEVICE_NAME)
            with device_client(url, paired["token"]) as http:
                yield MacHub(url, paired["device_id"], http, configuration, asked)


def wav(frames: int) -> bytes:
    """`frames` of 16-bit mono audio, all of one sample that is not silence."""
    output = io.BytesIO()
    with wave.open(output, "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(MEETING_SAMPLE_RATE)
        audio.writeframes(b"\x00\x01" * frames)
    return output.getvalue()


def put(
    http: httpx.Client,
    recording: str,
    track: str,
    sequence: int,
    *,
    frames: int = CHUNK_FRAMES,
    started_at_ms: int | None = None,
) -> None:
    """One chunk over `PUT /v1/chunks/{id}` with its capture headers: each
    chunk a second after the one before unless `started_at_ms` is given."""
    body = wav(frames)
    timing = {
        "started-at-ms": FIRST_STARTED_AT_MS + sequence * 1000
        if started_at_ms is None
        else started_at_ms,
        "monotonic-ns": MEETING_MONOTONIC_NS
        + sequence * frames * 1_000_000_000 // MEETING_SAMPLE_RATE,
        "first-sample": sequence * frames,
        "frames": frames,
        "sample-rate": MEETING_SAMPLE_RATE,
        "channels": 1,
    }
    headers = {
        "content-type": "audio/wav",
        "x-orbit-recording": recording,
        "x-orbit-track": track,
        "x-orbit-sequence": str(sequence),
        "x-orbit-sha256": hashlib.sha256(body).hexdigest(),
        **{f"x-orbit-{name}": str(value) for name, value in timing.items()},
    }
    _answered(http.put(f"/v1/chunks/{recording}.{track}.{sequence}", content=body, headers=headers))


def chunks_for(http: httpx.Client, manifest: dict) -> str:
    recording = str(uuid.uuid4())
    for track in manifest["tracks"]:
        for sequence in range(track["chunk_count"]):
            put(http, recording, track["track"], sequence)
    return recording


def listed(http: httpx.Client, **params) -> dict:
    answer = http.get("/v1/recordings", params=params)
    assert answer.status_code == 200, answer.text
    return answer.json()


def link_template(http: httpx.Client) -> None:
    """An invented template, linked to the series of the manifest with an event."""
    template = {
        "name": TEMPLATE_NAME,
        "context": "",
        "sections": [{"name": "Acuerdos", "instructions": "Lo que se acordó."}],
    }
    answer = http.put(f"/v1/note-templates/{TEMPLATE_ID}", json=template)
    assert answer.status_code in (200, 201), answer.text
    link = {"calendar_item_id": SERIES, "template_id": TEMPLATE_ID}
    _answered(http.put("/v1/note-template-links", json=link))


def asked_lines(request: dict) -> list[str]:
    """The lines of a request's messages, each without its Markdown heading
    mark: the template's name heads its section (`## Template: …`)."""
    return [
        line.removeprefix("## ")
        for message in request.get("messages", [])
        if isinstance(message.get("content"), str)
        for line in message["content"].splitlines()
    ]


def field(lines: list[str], name: str) -> str | None:
    found = [line.removeprefix(f"{name}: ") for line in lines if line.startswith(f"{name}: ")]
    assert len(found) <= 1, lines
    return found[0] if found else None


def test_every_closing_manifest_the_mac_writes_is_accepted_with_its_kind_and_event(mac_hub):
    http = mac_hub.http
    link_template(http)
    closed = {}
    for path in sorted(FIXTURES.glob("recording-manifest-*.json")):
        manifest = json.loads(path.read_text())
        recording = chunks_for(http, manifest)
        answer = http.post(f"/v1/recordings/{recording}/finalize", json=manifest)
        assert answer.status_code == 200, (path.name, answer.text)
        closed[path.name] = recording
    listing = listed(http)
    documents = {
        name: _answered(http.get(f"/v1/recordings/{recording}"))
        for name, recording in closed.items()
    }
    assert len(closed) == 6, sorted(closed)
    # «Silenciar»: the muted closure's transcript says where.
    assert documents["recording-manifest-muted-v1.json"]["transcript"]["muted_microphone"] == [
        {"start_ms": 2000, "end_ms": 4000},
        {"start_ms": 7000, "end_ms": 10000},
    ]
    call = closed["recording-manifest-voice-call-v1.json"]
    # A voice call is not a meeting: not listed, and no note was asked for.
    assert {row["recording_id"] for row in listing["meetings"]} == set(closed.values()) - {call}
    assert documents["recording-manifest-voice-call-v1.json"]["note"] is None
    assert all(
        document["note"] is not None
        for name, document in documents.items()
        if name != "recording-manifest-voice-call-v1.json"
    )
    # The meeting's note is asked for with its event and series: the series'
    # template, its attendees in their order and its title.
    wait_for_note(http, closed["recording-manifest-event-v1.json"])
    with_template = [
        lines
        for lines in map(asked_lines, mac_hub.asked)
        if field(lines, "Template") == TEMPLATE_NAME
    ]
    assert len(with_template) == 1, [asked_lines(request) for request in mac_hub.asked]
    (lines,) = with_template
    assert (field(lines, "Attendees") or "").split(", ") == ["Alina", "Ada"]
    assert field(lines, "Expected title") == "Comité de dirección"


def test_a_voice_note_left_open_is_not_a_meeting_and_a_meeting_just_started_is(mac_hub):
    """The voice notes earlier Macs uploaded and never closed.

    A meeting's chunk is sealed once it holds a second of audio and a capture
    frame holds at most one more (`capture.rs`), so it never holds two; a
    voice note is one chunk holding the whole note.
    """
    http = mac_hub.http
    voice, started = str(uuid.uuid4()), str(uuid.uuid4())
    put(http, voice, "mic_local", 0, frames=5 * 16_000)
    put(http, started, "mic_local", 0, frames=16_000)
    listing = listed(http)
    body = json.dumps({"device_id": mac_hub.device_id, "since_ms": 0, "limit": 50}).encode()
    headers = {
        "content-type": "application/json",
        **signed_headers(mac_hub.configuration, MEETINGS_LIST_PATH, body, MEETINGS_PROTOCOL),
    }
    deimos = httpx.post(f"{mac_hub.url}{MEETINGS_LIST_PATH}", content=body, headers=headers)
    assert [row["recording_id"] for row in listing["meetings"]] == [started]
    assert deimos.status_code == 200, deimos.text
    assert [row["recording_id"] for row in deimos.json()["meetings"]] == [started]


def test_meetings_that_start_together_are_all_listed_across_pages(mac_hub):
    http = mac_hub.http
    recordings = sorted(str(uuid.uuid4()) for _ in range(3))
    for recording in recordings:
        put(http, recording, "mic_local", 0, started_at_ms=1_000_000)
        manifest = {
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
        }
        _answered(http.post(f"/v1/recordings/{recording}/finalize", json=manifest))
    first = listed(http, limit=2)
    last = first["meetings"][-1]
    second = listed(
        http,
        limit=2,
        before_ms=last["started_at_ms"],
        before_id=last["recording_id"],
    )
    assert first["more"] is True
    assert [row["recording_id"] for row in first["meetings"]] == recordings[:2]
    assert [row["recording_id"] for row in second["meetings"]] == recordings[2:]
    assert second["more"] is False
