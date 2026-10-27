"""Hub-wide transcription selection through authenticated public routes; local upstream only."""

import base64
import json
from dataclasses import replace

import httpx
import pytest
from conftest import device
from support_audio import wav_bytes
from test_final_pass import fake_separator
from test_meeting_language_and_title import routes_only
from test_meeting_views import TEXTS, bearer, meeting

from orbit_hub.app import create_app

KEY = "synthetic-openrouter-key-never-real"
MAI = "mai"


def configured_app(environment, tmp_path, monkeypatch, *, status=200):
    settings, store, adapter = environment
    requests = []

    async def answer(request):
        requests.append(request)
        return httpx.Response(
            status,
            json={
                "text": "Uno. Dos. Tres.",
                "language": "es",
                "words": [
                    {"start": at, "end": at + 1, "word": text}
                    for at, text in [(1, "Uno."), (11, "Dos."), (21, "Tres.")]
                ],
                "error": {"message": KEY},
            },
        )

    from orbit_hub.openrouter_transcription import OpenRouterTranscriber

    monkeypatch.setattr(
        OpenRouterTranscriber,
        "_default_client",
        lambda self: httpx.AsyncClient(transport=httpx.MockTransport(answer)),
    )
    app = create_app(
        replace(settings, diarization=fake_separator(tmp_path)),
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: 1000,
    )
    return app, requests


async def choose(http, owner, model="mai", key=KEY):
    return await http.put(
        "/v1/settings/transcription", headers=bearer(owner), json={"model": model, "api_key": key}
    )


async def test_setting_validates_encrypts_and_never_returns_key(
    environment, tmp_path, monkeypatch, caplog
):
    app, requests = configured_app(environment, tmp_path, monkeypatch)
    _, store, _ = environment
    owner, second = device(store), device(store, "second")
    async with routes_only(app) as http:
        assert (await http.get("/v1/settings/transcription")).status_code == 401
        assert (await http.get("/v1/settings/transcription", headers=bearer(owner))).json() == {
            "model": "whisper",
            "key_configured": False,
        }
        saved = await choose(http, owner)
        assert saved.status_code == 200, saved.text
        read = await http.get("/v1/settings/transcription", headers=bearer(second))
        assert read.json() == {"model": "mai", "key_configured": True}
        assert KEY not in saved.text + read.text + caplog.text
    assert len(requests) == 1  # A real STT validation, against this local fake.
    assert KEY.encode() not in store._db.serialize()


@pytest.mark.parametrize(
    "status,cause",
    [
        (401, "transcription_key_invalid"),
        (402, "transcription_quota_exhausted"),
        (429, "transcription_rate_limited"),
        (503, "transcription_http_error"),
    ],
)
async def test_key_refused_in_place_without_saving(
    environment, tmp_path, monkeypatch, status, cause, caplog
):
    app, _ = configured_app(environment, tmp_path, monkeypatch, status=status)
    owner = device(environment[1])
    async with routes_only(app) as http:
        response = await choose(http, owner)
        assert response.status_code == 502, response.text
        assert response.json()["code"] == cause
        assert response.json()["refusal"]["provider_status"] == status
        assert KEY not in response.text + caplog.text
        assert (await http.get("/v1/settings/transcription", headers=bearer(owner))).json() == {
            "model": "whisper",
            "key_configured": False,
        }


@pytest.mark.parametrize("series", [False, True])
async def test_final_pass_uses_mai_words_dictionaries_and_pyannote(
    environment, tmp_path, monkeypatch, series
):
    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import dictionaries
    from orbit_hub.meetings.dictionaries import Correct, Save, Select

    app, requests = configured_app(environment, tmp_path, monkeypatch)
    _, store, _ = environment
    owner = device(store)
    recording = meeting(
        store,
        owner["device_id"],
        texts=TEXTS,
        event={"calendar_item_id": "weekly", "title": "Biblioteca"},
    )
    dictionaries.execute(
        store,
        owner["device_id"],
        Save(action="save", dictionary_id="12345678-1234-4234-8234-123456789012", name="Equipo"),
    )
    dictionaries.execute(
        store,
        owner["device_id"],
        Correct(
            action="correct",
            recording_id=recording,
            dictionary_id="12345678-1234-4234-8234-123456789012",
            wrong="orbita",
            correct="Órbita",
        ),
    )
    if series:
        dictionaries.execute(
            store,
            owner["device_id"],
            Select(
                action="select",
                recording_id=recording,
                dictionary_id="12345678-1234-4234-8234-123456789012",
                use_for_series=True,
            ),
        )
    async with routes_only(app) as http:
        assert (await choose(http, owner)).status_code == 200
        assert (
            await http.post(
                f"/v1/recordings/{recording}/final-pass",
                headers=bearer(owner),
                json={"model": "mai"},
            )
        ).status_code == 200
        assert await app.state.final_passes.run_once()
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
    assert document["transcript"]["final_pass"]["state"] == "completed", document["transcript"][
        "final_pass"
    ]
    assert document["transcript"]["final_pass"]["model"] == "mai"
    assert len(requests) == 3  # Validation and one <=45 s window per track.
    for request in requests[1:]:
        payload = json.loads(request.content)
        assert payload["model"] == "microsoft/mai-transcribe-2"
        assert payload["response_format"] == "verbose_json"
        assert payload["timestamp_granularities"] == ["word"]
        assert base64.b64decode(payload["input_audio"]["data"]).startswith(b"OggS")
        assert payload["input_audio"]["format"] == "ogg"
        assert payload["provider"]["zdr"] is True
        assert payload["provider"]["options"]["azure"] == {
            "diarization": {"enabled": False},
            "phraseList": {"phrases": ["Órbita"]},
            "enhancedMode": {"modelOptions": {"transcribeStyle": "verbatim"}},
        }
    job = {"device_id": owner["device_id"], "recording": recording}
    result = FinalPassRepository(store).voices(job)
    assert {span["voice"] for span in result["spans"]} == {"voice_1", "voice_2"}


async def test_voice_note_uses_selected_model_and_whisper_keeps_existing_path(
    environment, tmp_path, monkeypatch
):
    app, requests = configured_app(environment, tmp_path, monkeypatch)
    owner = device(environment[1])
    async with app.router.lifespan_context(app), routes_only(app) as http:
        audio = wav_bytes(b"\x10\x00" * 16000)
        before = await http.post("/v1/audio/transcribe", headers=bearer(owner), content=audio)
        assert before.status_code == 200
        assert not requests
        assert (await choose(http, owner)).status_code == 200
        after = await http.post("/v1/audio/transcribe", headers=bearer(owner), content=audio)
        assert after.status_code == 200, after.text
        assert after.json()["text"] == "Uno. Dos. Tres."
        assert len(requests) == 2


async def test_failed_draft_retries_mai_without_fallback_even_if_preference_changes(
    environment, tmp_path, monkeypatch
):
    import asyncio
    from uuid import uuid4

    from support_grouping import put
    from support_waits import wait_for

    from orbit_hub.transcription_preferences import Preferences

    app, _requests = configured_app(environment, tmp_path, monkeypatch)
    _, store, adapter = environment
    owner = device(store)
    async with routes_only(app) as http:
        assert (await choose(http, owner)).status_code == 200
    attempts, failed, finished = [], asyncio.Event(), asyncio.Event()

    async def fail_then_succeed(request):
        attempts.append(json.loads(request.content))
        if len(attempts) == 1:
            return httpx.Response(402, json={"error": {"message": KEY}})
        return httpx.Response(200, json={"text": "Acta lista.", "words": []})

    from orbit_hub.openrouter_transcription import OpenRouterTranscriber

    monkeypatch.setattr(
        OpenRouterTranscriber,
        "_default_client",
        lambda self: httpx.AsyncClient(transport=httpx.MockTransport(fail_then_succeed)),
    )
    recording = str(uuid4())
    put(store, owner["device_id"], recording, "mic_local", 0, frames=320_000)
    # Use the public worker and its normal durable scheduling, with explicit time.
    from orbit_hub.transcription import TranscriptionWorker
    from orbit_hub.transcription_preferences import Transcriber

    now = [1000]

    def changed(*_):
        with store.read_snapshot() as db:
            row = db.execute(
                "SELECT state,error_code FROM transcription_groups WHERE recording=?", (recording,)
            ).fetchone()
        if row["error_code"]:
            assert row["error_code"] == "transcription_quota_exhausted_status_402"
            failed.set()
        elif row["state"] == "completed":
            finished.set()

    worker = TranscriptionWorker(
        store,
        adapter,
        lambda: now[0],
        meetings=Transcriber(store, environment[0].meeting_transcription),
        recording_changed=changed,
    )
    worker.start()
    try:
        await wait_for(failed.wait(), "the local MAI failure")
        assert not adapter.audio
        # A setting change cannot reroute already sent audio on its retry.
        Preferences(store).save("whisper", KEY)
        now[0] = 1020
        worker.prepare_signal.set()
        await wait_for(finished.wait(), "the durable MAI retry")
    finally:
        await worker.close()
    assert len(attempts) == 2
    assert all(p["model"] == "microsoft/mai-transcribe-2" for p in attempts)


async def test_mai_final_pass_spans_multiple_45_second_windows(environment, tmp_path, monkeypatch):
    from uuid import uuid4

    from support_grouping import put

    app, requests = configured_app(environment, tmp_path, monkeypatch)
    separator = tmp_path / "fake-python"
    separator.write_text(separator.read_text().replace("484800", "1920000"))
    _, store, _ = environment
    owner = device(store)
    recording = str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(6):
            put(
                store,
                owner["device_id"],
                recording,
                track,
                sequence,
                frames=320_000,
                started_at_ms=900_000 + sequence * 20_000,
            )
    store.recordings.finalize(
        owner["device_id"],
        recording,
        {
            "tracks": [
                {"track": track, "chunk_count": 6, "last_sequence": 5, "total_frames": 1_920_000}
                for track in ("mic_local", "remote_app")
            ],
            "gaps": 0,
            "started_at_ms": 900_000,
            "finalized_at_ms": 1_020_000,
        },
    )
    async with routes_only(app) as http:
        assert (await choose(http, owner)).status_code == 200
        assert (
            await http.post(
                f"/v1/recordings/{recording}/final-pass",
                headers=bearer(owner),
                json={"model": "mai"},
            )
        ).status_code == 200
        # Pin MAI, then change the default before its first window.
        assert (await choose(http, owner, model="whisper", key=None)).status_code == 200
        assert await app.state.final_passes.run_once()
        document = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
    assert document["transcript"]["final_pass"]["state"] == "completed", document["transcript"][
        "final_pass"
    ]
    assert document["transcript"]["final_pass"]["model"] == "mai"
    # 120 seconds per track, three windows of <=45s per track plus validation.
    assert len(requests) == 7
    row = store._db.execute("SELECT body FROM recording_final_transcripts").fetchone()
    transcript = json.loads(
        store.decrypt(row[0], f"recording-final-transcript:{owner['device_id']}:{recording}")
    )
    local = [w for w in transcript["words"] if w["track"] == "mic_local"]
    assert len(local) == 9
    assert local[3]["start_ms"] == 46_000
    assert local[6]["start_ms"] == 91_000
    assert all(w["voice"] == "mic_local" for w in local)


async def test_final_mai_failure_is_visible_and_a_requested_retry_uses_mai(
    environment, tmp_path, monkeypatch, caplog
):
    app, _ = configured_app(environment, tmp_path, monkeypatch)
    owner = device(environment[1])
    recording = meeting(environment[1], owner["device_id"], texts=TEXTS)
    from orbit_hub.openrouter_transcription import OpenRouterTranscriber

    calls = []

    async def endpoint(request):
        calls.append(request)
        if len(calls) == 1:
            return httpx.Response(402, json={"error": {"message": KEY}})
        return httpx.Response(
            200,
            json={
                "text": "Uno. Dos. Tres.",
                "words": [
                    {"word": text, "start": at, "end": at + 1}
                    for text, at in [("Uno.", 1), ("Dos.", 11), ("Tres.", 21)]
                ],
            },
        )

    async with routes_only(app) as http:
        assert (await choose(http, owner)).status_code == 200
        monkeypatch.setattr(
            OpenRouterTranscriber,
            "_default_client",
            lambda self: httpx.AsyncClient(transport=httpx.MockTransport(endpoint)),
        )
        path = f"/v1/recordings/{recording}/final-pass"
        assert (
            await http.post(path, headers=bearer(owner), json={"model": "mai"})
        ).status_code == 200
        assert await app.state.final_passes.run_once()
        failed = await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))
        outcome = failed.json()["transcript"]["final_pass"]
        assert outcome["state"] == "failed"
        assert outcome["error"]["code"] == "transcription_quota_exhausted"
        assert outcome["error"]["provider_status"] == 402
        assert KEY not in failed.text + caplog.text
        assert (
            await http.post(path, headers=bearer(owner), json={"model": "mai"})
        ).status_code == 200
        assert await app.state.final_passes.run_once()
        after = (await http.get(f"/v1/recordings/{recording}", headers=bearer(owner))).json()
        assert after["transcript"]["final_pass"]["state"] == "completed"
    assert len(calls) == 3


@pytest.mark.parametrize(
    "error,cause",
    [
        (
            httpx.ConnectError("synthetic-openrouter-key-never-real"),
            "transcription_connection_error",
        ),
        (httpx.ReadTimeout("synthetic-openrouter-key-never-real"), "transcription_timeout"),
    ],
)
async def test_validation_transport_failures_are_sanitized(
    environment, tmp_path, monkeypatch, error, cause, caplog
):
    app, _ = configured_app(environment, tmp_path, monkeypatch)
    owner = device(environment[1])

    async def endpoint(request):
        raise error

    from orbit_hub.openrouter_transcription import OpenRouterTranscriber

    monkeypatch.setattr(
        OpenRouterTranscriber,
        "_default_client",
        lambda self: httpx.AsyncClient(transport=httpx.MockTransport(endpoint)),
    )
    async with routes_only(app) as http:
        response = await choose(http, owner)
        assert response.json()["code"] == cause
        assert KEY not in response.text + caplog.text
        if cause == "transcription_timeout":
            assert response.json()["refusal"]["timeout_ms"] == 30000


async def test_mai_refuses_audio_over_its_memory_bound_with_numbers(environment):
    from orbit_hub.meeting_transcription import MAX_AUDIO_BYTES, MeetingTranscriptionConfiguration
    from orbit_hub.openrouter_transcription import OpenRouterTranscriber
    from orbit_hub.protocol import HubError

    transcriber = OpenRouterTranscriber(
        KEY, MeetingTranscriptionConfiguration.model_validate(environment[0].meeting_transcription)
    )
    with pytest.raises(HubError) as refused:
        await transcriber.transcribe(bytes(MAX_AUDIO_BYTES + 1))
    assert refused.value.code == "too_large"
    assert refused.value.numbers == {
        "resource": "audio",
        "bytes": MAX_AUDIO_BYTES + 1,
        "max_bytes": MAX_AUDIO_BYTES,
    }
