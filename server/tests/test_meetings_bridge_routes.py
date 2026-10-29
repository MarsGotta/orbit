"""The private meetings routes answer a paired device's meetings.

The two routes are exercised against the real ``Authenticator`` of
``orbit_hub.private_auth``, exactly as the production routes bind it: unsigned
or tampered requests never pass. The «Reuniones» switch is retired: there is
no sharing record to consult, and Deimos sees the meetings always (the cases
at the end of this file). No Hermes, no Mac, no external network.
"""

import io
import json
import logging
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_grouping import put
from support_notes import write_note

from orbit_hub.app import create_app
from orbit_hub.meetings_bridge import (
    LIST_PATH,
    NOTE_PATH,
    PROTOCOL,
    TRANSCRIPT_PATH,
)
from orbit_hub.private_auth import sign_request
from orbit_hub.processing import ProcessingError

KEY = bytes(range(32, 64))
KEY_ID = "meetings-bridge-test-key"
NOW_MS = 1_000_000
ONE_MIB = 1024 * 1024
TWO_MIB = 2 * ONE_MIB
TRANSCRIPT_SECRET = "SECRETO_SINTETICO_DE_TRANSCRIPCION"


def signed(path, payload, *, key=KEY, key_id=KEY_ID, issued=NOW_MS, nonce=None):
    body = json.dumps(payload, separators=(",", ":")).encode()
    nonce = nonce or str(uuid4())
    headers = {
        "content-type": "application/json",
        **sign_request(PROTOCOL, path, KEY_ID, KEY, body, issued_at_ms=issued, nonce=nonce),
    }
    return body, headers


def recording_with_transcript(store, owner, text, *, title, started_at_ms=1_000, minutes=30):
    """A finalized recording whose single transcript is already finished."""
    recording = str(uuid4())
    put(store, owner, recording, "mic_local", 0, started_at_ms=started_at_ms)
    store.recordings.finalize(
        owner,
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
            "started_at_ms": started_at_ms,
            "finalized_at_ms": started_at_ms + minutes * 60_000,
            "title": title,
        },
    )
    job = store.groups.claim(1_000)
    assert job is not None and job["recording"] == recording
    store.groups.finish(job, text)
    return recording


async def test_failed_transcript_is_a_causal_refusal_for_deimos(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)
    recording = str(uuid4())
    put(store, owner["device_id"], recording, "mic_local", 0, started_at_ms=1_000)
    store.recordings.finalize(
        owner["device_id"],
        recording,
        {
            "tracks": [
                {"track": "mic_local", "chunk_count": 1, "last_sequence": 0, "total_frames": 16_000}
            ],
            "gaps": 0,
            "started_at_ms": 1_000,
            "finalized_at_ms": 2_000,
            "title": "Reunión sintética",
        },
    )
    for now in (1000, 1010, 1030, 1070, 1150):
        job = store.groups.claim(now)
        assert job is not None
        store.groups.fail(job, "transcription_http_503", now)

    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        body, headers = signed(
            TRANSCRIPT_PATH,
            {"device_id": owner["device_id"], "recording_id": recording, "format": "text"},
        )
        response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
    assert response.status_code == 503
    refusal = response.json()["refusal"]
    assert refusal["code"] == "transcription_failed"
    assert refusal["provider_status"] == 503
    assert refusal["attempts"] == 5


async def test_partial_transcript_returns_completed_track_and_failed_group_cause(
    meetings_hub_app,
):
    app, store = meetings_hub_app
    owner = device(store)
    recording = str(uuid4())
    for track in ("mic_local", "remote_app"):
        put(store, owner["device_id"], recording, track, 0, started_at_ms=1_000)
    store.recordings.finalize(
        owner["device_id"],
        recording,
        {
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
            "started_at_ms": 1_000,
            "finalized_at_ms": 2_000,
            "title": "Reunión parcial sintética",
        },
    )
    completed = store.groups.claim(1_000)
    assert completed is not None
    store.groups.finish(completed, "Texto completado de micrófono.")
    for now in (1_000, 1_010, 1_030, 1_070, 1_150):
        failed = store.groups.claim(now)
        assert failed is not None and failed["group_id"] != completed["group_id"]
        store.groups.fail(failed, "transcription_http_503", now)

    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        list_body, list_headers = signed(
            LIST_PATH, {"device_id": owner["device_id"], "since_ms": 0, "limit": 50}
        )
        listed = await http.post(LIST_PATH, content=list_body, headers=list_headers)
        body, headers = signed(
            TRANSCRIPT_PATH,
            {"device_id": owner["device_id"], "recording_id": recording, "format": "text"},
        )
        response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)

    assert listed.status_code == 200, listed.text
    assert listed.json()["meetings"][0]["transcript"] == "partial"
    assert response.status_code == 200, response.text
    payload = response.json()
    # One line per turn with its speaker; no note, so the track names it.
    assert payload["text"] == "[00:00:00] Local microphone: Texto completado de micrófono."
    assert payload["incomplete_groups"] == [
        {
            "track": "remote_app",
            "first_index": 0,
            "last_index": 0,
            "cause": "transcription_http_error",
            "provider_status": 503,
            "attempts": 5,
        }
    ]


async def test_pending_transcript_reports_previous_failure_and_retry(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)
    recording = str(uuid4())
    put(store, owner["device_id"], recording, "mic_local", 0, started_at_ms=1_000)
    store.recordings.finalize(
        owner["device_id"],
        recording,
        {
            "tracks": [
                {"track": "mic_local", "chunk_count": 1, "last_sequence": 0, "total_frames": 16_000}
            ],
            "gaps": 0,
            "started_at_ms": 1_000,
            "finalized_at_ms": 2_000,
            "title": "Reunión sintética",
        },
    )
    for now in (1000, 1010):
        job = store.groups.claim(now)
        assert job is not None
        store.groups.fail(job, "transcription_timeout_300000", now)

    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        body, headers = signed(
            TRANSCRIPT_PATH,
            {"device_id": owner["device_id"], "recording_id": recording, "format": "text"},
        )
        response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
        retry_job = store.groups.claim(1030)
        assert retry_job is not None and retry_job["recording"] == recording
        body, headers = signed(
            TRANSCRIPT_PATH,
            {"device_id": owner["device_id"], "recording_id": recording, "format": "text"},
        )
        running_response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
    assert response.status_code == 503
    refusal = response.json()["refusal"]
    assert refusal["code"] == "transcription_pending"
    assert refusal["cause"] == "transcription_timeout"
    assert refusal["timeout_ms"] == 300_000
    assert refusal["attempts"] == 2
    assert refusal["retry_at_ms"] == 1_030_000
    running = running_response.json()["refusal"]
    assert running_response.status_code == 503
    assert running["phase"] == "running"
    assert running["cause"] == "transcription_timeout"
    assert running["attempts"] == 3
    assert "retry_at_ms" not in running


@pytest.fixture
def meetings_hub_app(environment, tmp_path):
    settings, store, adapter = environment
    key = tmp_path / "meetings-bridge.key"
    key.write_bytes(KEY)
    key.chmod(0o600)
    settings = replace(
        settings,
        meetings_bridge={"key_id": KEY_ID, "credential_file": str(key)},
    )
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: NOW_MS / 1000
    )
    return app, store


def bearer(owner):
    return {"Authorization": "Bearer " + owner["token"]}


async def test_meetings_routes_answer_signed_requests_for_a_paired_device(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)
    long_transcript = TRANSCRIPT_SECRET + "a" * TWO_MIB
    recording = recording_with_transcript(
        store, owner["device_id"], long_transcript, title="Daily del equipo"
    )
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        list_body, list_headers = signed(
            LIST_PATH, {"device_id": owner["device_id"], "since_ms": 0, "limit": 50}
        )
        # A request without a valid signature never passes the real
        # Authenticator; a signed one is answered with no switch to consult.
        unsigned = await http.post(
            LIST_PATH,
            content=list_body,
            headers={"content-type": "application/json"},
        )
        assert unsigned.status_code == 401

        listed = await http.post(LIST_PATH, content=list_body, headers=list_headers)
        assert listed.status_code == 200, listed.text
        meetings = listed.json()["meetings"]
        assert [row["recording_id"] for row in meetings] == [recording]
        row = meetings[0]
        # The fixed `summary: "none"` became the real state of the note,
        # and each meeting says whether the Mac has closed it.
        assert set(row) == {
            "recording_id",
            "title",
            "started_at_ms",
            "duration_ms",
            "capture",
            "transcript",
            "note",
        }
        assert row["title"] == "Daily del equipo"
        assert row["started_at_ms"] == 1_000
        assert row["duration_ms"] == 30 * 60_000
        assert row["capture"] == "closed"
        assert row["transcript"] == "ready"
        assert row["note"] == {"state": "not_requested"}

        # A 2 MiB transcript is delivered in pages of at most 48,000 characters,
        # continuable with after_index until the end.
        first_body, first_headers = signed(
            TRANSCRIPT_PATH,
            {
                "device_id": owner["device_id"],
                "recording_id": recording,
                "format": "text",
            },
        )
        first = await http.post(TRANSCRIPT_PATH, content=first_body, headers=first_headers)
        assert first.status_code == 200, first.text
        page = first.json()
        assert 0 < len(page["text"]) <= 48_000
        assert len(page["text"]) < len(long_transcript)
        assert page["after_index"] == 0
        assert page["next_after_index"] is not None
        combined = page["text"]
        cursor = page["next_after_index"]
        while cursor is not None:
            next_body, next_headers = signed(
                TRANSCRIPT_PATH,
                {
                    "device_id": owner["device_id"],
                    "recording_id": recording,
                    "format": "text",
                    "after_index": cursor,
                },
            )
            following = await http.post(TRANSCRIPT_PATH, content=next_body, headers=next_headers)
            assert following.status_code == 200, following.text
            tail = following.json()
            assert 0 < len(tail["text"]) <= 48_000
            assert tail["after_index"] == cursor
            combined += tail["text"]
            cursor = tail["next_after_index"]
        assert combined == "[00:00:00] Local microphone: " + long_transcript


async def test_meetings_routes_log_no_transcript_text(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)
    recording = recording_with_transcript(
        store, owner["device_id"], TRANSCRIPT_SECRET + " texto sintético", title="Daily"
    )
    captured = io.StringIO()
    handler = logging.StreamHandler(captured)
    package_logger = logging.getLogger("orbit_hub")
    previous_level = package_logger.level
    package_logger.addHandler(handler)
    package_logger.setLevel(logging.DEBUG)
    try:
        async with (
            app.router.lifespan_context(app),
            httpx.AsyncClient(
                transport=httpx.ASGITransport(app), base_url="https://localhost"
            ) as http,
        ):
            for path, payload in (
                (LIST_PATH, {"device_id": owner["device_id"], "since_ms": 0, "limit": 50}),
                (
                    TRANSCRIPT_PATH,
                    {
                        "device_id": owner["device_id"],
                        "recording_id": recording,
                        "format": "text",
                    },
                ),
            ):
                body, headers = signed(path, payload)
                response = await http.post(path, content=body, headers=headers)
                assert response.status_code == 200, response.text
    finally:
        package_logger.removeHandler(handler)
        package_logger.setLevel(previous_level)
    assert TRANSCRIPT_SECRET not in captured.getvalue()


# Deimos sees the meetings always; no switch stands before them. «Reuniones»
# was retired on 2026-09-25, and a Mac of the previous
# version that still reports its switch — even a «false» — blocks nothing.


# The note route behind `orbit_note` has its own `Authenticator`, bound
# to its own path like the list and transcript routes, so a signature made for
# another route never opens it; nothing of a note reaches the hub's log; and
# the listing pages with `more` and `before_ms` instead of stopping in silence.
NOTE_SECRET = "SECRETO_SINTETICO_DE_NOTA_P907"  # gitleaks:allow (synthetic test key)


async def test_the_note_route_answers_only_a_signature_made_for_it(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)["device_id"]
    recording = recording_with_transcript(store, owner, "texto sintético", title="Daily")
    await write_note(store, owner, recording, private_notes=NOTE_SECRET)
    payload = {"device_id": owner, "recording_id": recording}
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        body, headers = signed(NOTE_PATH, payload)
        unsigned = await http.post(
            NOTE_PATH, content=body, headers={"content-type": "application/json"}
        )
        # Signed for the transcript route, replayed at the note route.
        other_body, other_headers = signed(TRANSCRIPT_PATH, payload)
        replayed = await http.post(NOTE_PATH, content=other_body, headers=other_headers)
        answered = await http.post(NOTE_PATH, content=body, headers=headers)
        missing_body, missing_headers = signed(
            NOTE_PATH, {"device_id": owner, "recording_id": str(uuid4())}
        )
        missing = await http.post(NOTE_PATH, content=missing_body, headers=missing_headers)

    assert unsigned.status_code == 401
    assert replayed.status_code == 401
    assert answered.status_code == 200, answered.text
    note = answered.json()
    assert note["state"] == "ready"
    assert note["private_notes"] == NOTE_SECRET
    assert note["note"]["action_items"][0]["done"] is False
    assert missing.status_code == 404
    assert missing.json()["code"] == "recording_not_found"


async def test_a_queued_note_suggests_waiting_until_its_retry(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)["device_id"]
    recording = recording_with_transcript(store, owner, "texto sintético", title="Daily")
    # The note fails at 1000 s and the worker retries it thirty seconds later.
    await write_note(
        store,
        owner,
        recording,
        now=NOW_MS / 1000,
        answers=[ProcessingError("provider_timeout", timeout_seconds=300)],
    )
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        body, headers = signed(NOTE_PATH, {"device_id": owner, "recording_id": recording})
        queued = await http.post(NOTE_PATH, content=body, headers=headers)

    assert queued.status_code == 503, queued.text
    refusal = queued.json()["refusal"]
    assert refusal["code"] == "note_queued"
    assert refusal["retry_at_ms"] == NOW_MS + 30_000
    # The wait is the time left until that retry, not the generic note time.
    assert refusal["retry_after_ms"] == 30_000
    assert refusal["retry"] == {"kind": "after_ms", "ms": 30_000}


async def test_the_note_route_logs_no_note_content(meetings_hub_app):
    app, store = meetings_hub_app
    owner = device(store)["device_id"]
    recording = recording_with_transcript(store, owner, "texto sintético", title="Daily")
    await write_note(store, owner, recording, private_notes=NOTE_SECRET)
    captured = io.StringIO()
    handler = logging.StreamHandler(captured)
    package_logger = logging.getLogger("orbit_hub")
    previous_level = package_logger.level
    package_logger.addHandler(handler)
    package_logger.setLevel(logging.DEBUG)
    try:
        async with (
            app.router.lifespan_context(app),
            httpx.AsyncClient(
                transport=httpx.ASGITransport(app), base_url="https://localhost"
            ) as http,
        ):
            body, headers = signed(NOTE_PATH, {"device_id": owner, "recording_id": recording})
            response = await http.post(NOTE_PATH, content=body, headers=headers)
            assert response.status_code == 200, response.text
    finally:
        package_logger.removeHandler(handler)
        package_logger.setLevel(previous_level)
    assert NOTE_SECRET not in captured.getvalue()


async def test_the_listing_says_when_there_are_more_and_continues_before_the_last(
    meetings_hub_app,
):
    app, store = meetings_hub_app
    owner = device(store)["device_id"]
    recordings = [
        recording_with_transcript(
            store, owner, "texto", title=f"Reunión {index}", started_at_ms=10_000 * (index + 1)
        )
        for index in range(3)
    ]
    live = str(uuid4())
    put(store, owner, live, "mic_local", 0, started_at_ms=90_000)
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        body, headers = signed(LIST_PATH, {"device_id": owner, "since_ms": 0, "limit": 2})
        first = (await http.post(LIST_PATH, content=body, headers=headers)).json()
        last = first["meetings"][-1]["started_at_ms"]
        body, headers = signed(
            LIST_PATH, {"device_id": owner, "since_ms": 0, "limit": 2, "before_ms": last}
        )
        second = (await http.post(LIST_PATH, content=body, headers=headers)).json()

    # Newest first by when they started, the open one included.
    assert [row["recording_id"] for row in first["meetings"]] == [live, recordings[2]]
    assert first["more"] is True
    assert first["meetings"][0]["capture"] == "open"
    assert first["meetings"][0]["note"] == {"state": "not_requested"}
    assert first["meetings"][1]["capture"] == "closed"
    assert [row["recording_id"] for row in second["meetings"]] == [recordings[1], recordings[0]]
    assert second["more"] is False


@pytest.mark.parametrize("format_name", ["text", "segments"])
@pytest.mark.parametrize("after_index", [0, 48_000])
async def test_every_transcript_page_is_bounded_and_continues_without_loss(
    meetings_hub_app, after_index, format_name
):
    app, store = meetings_hub_app
    owner = device(store)["device_id"]
    # Three pages, including multibyte characters. Offsets count characters,
    # exactly like the MCP tool's after_index, not UTF-8 bytes.
    text = "á🎙" * 48_000 + " final"
    recording = recording_with_transcript(store, owner, text, title="Tres páginas")
    expected = "[00:00:00] Local microphone: " + text
    if format_name == "segments":
        expected = json.dumps(
            {
                "at_ms": 0,
                "track": "mic_local",
                "speaker": "Local microphone",
                "first_index": 0,
                "last_index": 0,
                "text": text,
            },
            ensure_ascii=False,
            separators=(",", ":"),
        )
    pages = []
    cursor = after_index
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        while True:
            body, headers = signed(
                TRANSCRIPT_PATH,
                {
                    "device_id": owner,
                    "recording_id": recording,
                    "after_index": cursor,
                    "format": format_name,
                },
            )
            response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
            assert response.status_code == 200, response.text
            page = response.json()
            assert page["after_index"] == cursor
            assert page["total_chars"] == len(expected)
            assert 0 < len(page["text"]) <= 48_000
            pages.append(page["text"])
            following = page["next_after_index"]
            if following is None:
                break
            assert following == cursor + len(page["text"])
            cursor = following
    assert "".join(pages) == expected[after_index:]
    assert len(pages) == (3 if after_index == 0 else 2)
