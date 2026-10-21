"""Hub-owned STT integration with a loopback synthetic provider, never real ASR."""

import asyncio
import json
import sys
import threading

import httpx
import pytest
import uvicorn
from conftest import device
from support_client_e2e import tcp_server
from support_synthetic_stt import SyntheticStt
from support_transcription import insert_job, wav_fixture
from support_waits import wait_for, wait_thread

from orbit_hub import meeting_transcription
from orbit_hub.app import create_app
from orbit_hub.cli import main
from orbit_hub.config import Settings
from orbit_hub.meetings.turns import read as read_transcript


class HermesMustStayDisconnected:
    """Records forbidden Hermes use without emulating either Hermes or Whisper."""

    def __init__(self):
        self.connection_attempts = 0
        self.transcription_attempts = 0

    def connection(self, _callback):
        self.connection_attempts += 1
        raise AssertionError("meeting transcription must not open Hermes")

    async def transcribe(self, _body):
        self.transcription_attempts += 1
        raise AssertionError("meeting transcription must not use Hermes transcription")

    async def speech(self, _text):
        if False:
            yield b""


def toml_string(value) -> str:
    return json.dumps(str(value))


def load_stt_settings(tmp_path, base, endpoint: str, *, language: str | None = "es") -> Settings:
    path = tmp_path / "orbit.toml"
    meeting = [
        "[meeting_transcription]",
        f"endpoint = {toml_string(endpoint)}",
        'model = "openai/whisper-large-v3"',
    ]
    if language is not None:
        meeting.append(f"language = {toml_string(language)}")
    path.write_text(
        "\n".join(
            (
                f"data_dir = {toml_string(base.data_dir)}",
                f"key_file = {toml_string(base.key_file)}",
                f"hermes_url = {toml_string(base.hermes_url)}",
                f"hermes_credential_file = {toml_string(base.hermes_credential_file)}",
                *meeting,
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return Settings.load(path)


def seal_one_group(store, owner):
    _, recording = insert_job(store, owner)
    store.finalize_recording(
        owner,
        recording,
        {
            "tracks": [
                {
                    "track": "mic_local",
                    "chunk_count": 1,
                    "last_sequence": 0,
                    "total_frames": 160,
                }
            ],
            "gaps": 0,
            "started_at_ms": 2000,
            "finalized_at_ms": 2010,
        },
    )
    return recording


def group_row(store, recording):
    with store._lock:
        row = store._db.execute(
            "SELECT state,attempts,error_code FROM transcription_groups WHERE recording=?",
            (recording,),
        ).fetchone()
    return dict(row) if row is not None else None


def exhaust_one_group(store, owner):
    recording = seal_one_group(store, owner)
    for now in (1000, 1010, 1030, 1070, 1150):
        job = store.claim_transcription(now)
        assert job is not None and job["recording"] == recording
        store.fail_transcription(job, "transcription_failed", now)
    assert group_row(store, recording) == {
        "state": "failed",
        "attempts": 5,
        "error_code": "transcription_failed",
    }
    return recording


async def transcript(store, owner, recording):
    with store.read_snapshot() as db:
        return read_transcript(store, db, owner, recording)


def observe_transcription_changes(store, monkeypatch):
    """Queue durable worker transitions without putting a clock on them."""
    loop = asyncio.get_running_loop()
    changes = asyncio.Queue()
    original_finish = store.finish_transcription
    original_fail = store.fail_transcription

    def finish(job, text, language=None):
        original_finish(job, text, language)
        loop.call_soon_threadsafe(changes.put_nowait, (job["recording"], "completed"))

    def fail(job, code, now):
        original_fail(job, code, now)
        loop.call_soon_threadsafe(changes.put_nowait, (job["recording"], "failed"))

    monkeypatch.setattr(store, "finish_transcription", finish)
    monkeypatch.setattr(store, "fail_transcription", fail)
    return changes


async def run_until_completed(
    settings, store, hermes, owner, recording, changes, *, now=lambda: 1000
):
    app = create_app(settings, store=store, adapter=hermes, now=now)
    async with app.router.lifespan_context(app):
        assert await wait_for(changes.get(), f"durable completion of group {recording}") == (
            recording,
            "completed",
        )
        result = await transcript(store, owner, recording)
        # The durable row, not the observed call, is what the daemon serves.
        assert result.groups == 1
        assert group_row(store, recording)["state"] == "completed"
        return result


def multipart_value(body: bytes, field: str) -> str:
    remainder = body.split(f'name="{field}"'.encode(), 1)[1]
    value = remainder.split(b"\r\n\r\n", 1)[1].split(b"\r\n", 1)[0]
    return value.decode()


async def test_group_uses_configured_stt_without_opening_hermes(environment, tmp_path, monkeypatch):
    base, store, _ = environment
    owner = device(store)["device_id"]
    recording = seal_one_group(store, owner)
    changes = observe_transcription_changes(store, monkeypatch)
    provider = SyntheticStt("texto sintético del STT")
    hermes = HermesMustStayDisconnected()

    async with tcp_server(provider.app) as endpoint:
        settings = load_stt_settings(tmp_path, base, endpoint)
        result = await run_until_completed(settings, store, hermes, owner, recording, changes)

    assert result.segments[0].text == "texto sintético del STT"
    assert len(provider.requests) == 1
    request = provider.requests[0]
    assert request["content_type"].startswith("multipart/form-data;")
    assert wav_fixture() in request["body"]
    assert multipart_value(request["body"], "model") == "openai/whisper-large-v3"
    assert multipart_value(request["body"], "language") == "es"
    assert hermes.connection_attempts == 0
    assert hermes.transcription_attempts == 0


async def test_empty_stt_text_is_stored_as_completed_silence(environment, tmp_path, monkeypatch):
    base, store, _ = environment
    owner = device(store)["device_id"]
    recording = seal_one_group(store, owner)
    changes = observe_transcription_changes(store, monkeypatch)
    provider = SyntheticStt("")
    hermes = HermesMustStayDisconnected()

    async with tcp_server(provider.app) as endpoint:
        settings = load_stt_settings(tmp_path, base, endpoint)
        result = await run_until_completed(settings, store, hermes, owner, recording, changes)

    assert len(provider.requests) == 1
    assert result.groups == 1 and result.segments == ()
    assert group_row(store, recording) == {"state": "completed", "attempts": 1, "error_code": None}


async def test_http_error_and_timeout_remain_pending_and_increment_attempts(
    environment, tmp_path, monkeypatch
):
    monkeypatch.setattr(meeting_transcription, "REQUEST_TIMEOUT_SECONDS", 0.05)
    monkeypatch.setattr(meeting_transcription, "CONNECT_TIMEOUT_SECONDS", 0.05)
    base, store, _ = environment
    owner = device(store)["device_id"]
    recording = seal_one_group(store, owner)
    changes = observe_transcription_changes(store, monkeypatch)
    provider = SyntheticStt(500, None)
    hermes = HermesMustStayDisconnected()

    async with tcp_server(provider.app) as endpoint:
        settings = load_stt_settings(tmp_path, base, endpoint)
        first_app = create_app(settings, store=store, adapter=hermes, now=lambda: 1000)
        async with first_app.router.lifespan_context(first_app):
            assert await wait_for(changes.get(), f"first failed attempt of group {recording}") == (
                recording,
                "failed",
            )
            first = group_row(store, recording)

        second_app = create_app(settings, store=store, adapter=hermes, now=lambda: 1010)
        async with second_app.router.lifespan_context(second_app):
            assert await wait_for(changes.get(), f"second failed attempt of group {recording}") == (
                recording,
                "failed",
            )
            second = group_row(store, recording)

    assert first == {"state": "pending", "attempts": 1, "error_code": "transcription_http_500"}
    assert second == {"state": "pending", "attempts": 2, "error_code": "transcription_timeout_50"}
    assert len(provider.requests) == 2
    assert hermes.transcription_attempts == 0


def test_cli_refuses_to_start_without_meeting_transcription(tmp_path, monkeypatch, capsys):
    path = tmp_path / "orbit.toml"
    path.write_text(
        "\n".join(
            (
                f"data_dir = {toml_string(tmp_path / 'data')}",
                f"key_file = {toml_string(tmp_path / 'key')}",
                'hermes_url = "http://127.0.0.1:0"',
                f"hermes_credential_file = {toml_string(tmp_path / 'credential')}",
                "allow_insecure_localhost = true",
            )
        )
        + "\n",
        encoding="utf-8",
    )
    started = []
    monkeypatch.setattr(sys, "argv", ["orbit-hub", "--config", str(path), "serve"])
    monkeypatch.setattr("orbit_hub.app.create_app", lambda _settings: object())
    monkeypatch.setattr(uvicorn, "run", lambda *_args, **_kwargs: started.append(True))

    with pytest.raises(SystemExit) as stopped:
        main()

    assert stopped.value.code == 1
    assert "meeting_transcription" in capsys.readouterr().err
    assert started == []


async def test_authenticated_retry_route_requeues_and_worker_completes(
    environment, tmp_path, monkeypatch
):
    base, store, _ = environment
    paired = device(store)
    owner = paired["device_id"]
    recording = exhaust_one_group(store, owner)
    provider = SyntheticStt("texto recuperado por HTTP")
    hermes = HermesMustStayDisconnected()
    changes = observe_transcription_changes(store, monkeypatch)
    original_claim = store.claim_transcription
    claim_released = threading.Event()
    pending_claim_started = asyncio.Event()
    checked_empty = asyncio.Event()
    loop = asyncio.get_running_loop()

    def hold_claim_after_retry(now):
        row = group_row(store, recording)
        if row is not None and row["state"] == "pending":
            loop.call_soon_threadsafe(pending_claim_started.set)
            wait_thread(claim_released, "test permission to claim the retried group")
        result = original_claim(now)
        if row is not None and row["state"] == "failed" and result is None:
            loop.call_soon_threadsafe(checked_empty.set)
        return result

    monkeypatch.setattr(store, "claim_transcription", hold_claim_after_retry)

    async with tcp_server(provider.app) as endpoint:
        settings = load_stt_settings(tmp_path, base, endpoint)
        app = create_app(settings, store=store, adapter=hermes, now=lambda: 2000)
        try:
            async with app.router.lifespan_context(app):
                await wait_for(checked_empty.wait(), f"the idle worker declining group {recording}")
                assert group_row(store, recording)["state"] == "failed"
                assert provider.requests == []

                async with httpx.AsyncClient(
                    transport=httpx.ASGITransport(app=app),
                    base_url="https://localhost",
                    headers={"Authorization": "Bearer " + paired["token"]},
                ) as http:
                    response = await http.post(f"/v1/recordings/{recording}/transcript/retry")

                assert response.status_code == 200
                assert response.json() == {"requeued": 1}
                await wait_for(
                    pending_claim_started.wait(),
                    "the store worker noticing the retried group",
                )
                assert group_row(store, recording) == {
                    "state": "pending",
                    "attempts": 0,
                    "error_code": None,
                }

                claim_released.set()
                assert await wait_for(
                    changes.get(), f"durable completion of group {recording}"
                ) == (recording, "completed")
                result = await transcript(store, owner, recording)
                assert group_row(store, recording)["state"] == "completed"
        finally:
            claim_released.set()

    assert result.segments[0].text == "texto recuperado por HTTP"
    assert len(provider.requests) == 1


async def test_missing_language_is_omitted_from_multipart(environment, tmp_path, monkeypatch):
    base, store, _ = environment
    owner = device(store)["device_id"]
    recording = seal_one_group(store, owner)
    changes = observe_transcription_changes(store, monkeypatch)
    provider = SyntheticStt("language autodetectado")
    hermes = HermesMustStayDisconnected()

    async with tcp_server(provider.app) as endpoint:
        settings = load_stt_settings(tmp_path, base, endpoint, language=None)
        result = await run_until_completed(settings, store, hermes, owner, recording, changes)

    assert result.segments[0].text == "language autodetectado"
    assert len(provider.requests) == 1
    body = provider.requests[0]["body"]
    assert multipart_value(body, "model") == "openai/whisper-large-v3"
    assert b'name="language"' not in body
