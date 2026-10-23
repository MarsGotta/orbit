import asyncio
import json
import logging
import time
from uuid import uuid4

from conftest import device
from support_app import client, crash_the_revocation_pass

from orbit_hub.protocol import HubError
from orbit_hub.runtime import DeviceRuntime
from orbit_hub.transcription import TranscriptionWorker


class FlakyClaims:
    """The store's transcription groups, whose claim crashes `crashes` times."""

    def __init__(self, inner, crashes):
        self._inner, self._crashes = inner, crashes

    def claim(self, now):
        if self._crashes[0] > 0:
            self._crashes[0] -= 1
            raise RuntimeError("fixture claim crash")
        return self._inner.claim(now)

    def __getattr__(self, name):
        return getattr(self._inner, name)


class FlakyClaimStore:
    def __init__(self, inner, crashes):
        self._inner = inner
        self.groups = FlakyClaims(inner.groups, crashes)

    def __getattr__(self, name):
        return getattr(self._inner, name)


class MeetingTranscriberFake:
    async def transcribe(self, _audio, _language=None):
        return "Transcripción de reunión sintética"


async def test_worker_claim_crash_is_logged_relaunched_and_transcription_recovers(
    environment, caplog
):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    crashes = [1]
    worker = TranscriptionWorker(
        FlakyClaimStore(store, crashes),
        adapter,
        lambda: 1000,
        meetings=MeetingTranscriberFake(),
    )
    with caplog.at_level(logging.DEBUG):
        worker.start()
        deadline = time.monotonic() + 5
        while crashes[0] and time.monotonic() < deadline:
            await asyncio.sleep(0.01)
        assert crashes[0] == 0, "worker never reached the crashing claim"
        text = None
        while time.monotonic() < deadline:
            try:
                text = await worker.transcribe(owner, b"interactive-fixture")
                break
            except HubError:
                await asyncio.sleep(0.05)
        assert text == "Transcripción sintética", "transcribe() never accepted work again"
        assert any(getattr(record, "event", None) for record in caplog.records), (
            "the crash was not logged as a structured event"
        )
    await worker.close()


def test_enforce_revocations_crash_relaunches_and_later_revocation_applies(
    environment, monkeypatch, caplog
):
    _, store, adapter = environment
    crashes = crash_the_revocation_pass(
        store, monkeypatch, lambda: RuntimeError("fixture enforce crash")
    )
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with caplog.at_level(logging.DEBUG), client(environment) as http:
        with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
            assert ws.receive_json()["event"] == "ready"
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "chat.send",
                    "params": {"text": "fixture revocation turn"},
                }
            )
            for _ in range(10):
                if ws.receive_json()["event"] == "state.snapshot":
                    break
        crash_deadline = time.monotonic() + 2
        while crashes[0] and time.monotonic() < crash_deadline:
            time.sleep(0.02)
        assert crashes[0] == 0, "enforce_revocations never reached the crashing pass"
        store.devices.revoke(paired["device_id"])
        connection = adapter.connections[0]
        deadline = time.monotonic() + 5
        while connection.connected and time.monotonic() < deadline:
            time.sleep(0.02)
        assert connection.connected is False, "revocation was never applied after the crash"
        assert "session.interrupt" in [method for method, _ in connection.calls]
        assert any(getattr(record, "event", None) for record in caplog.records), (
            "the crash was not logged as a structured event"
        )


def test_internal_failure_is_logged_with_event_and_code_and_never_fixture_text(
    environment, monkeypatch, caplog
):
    secret = "TEXTO-PRIVADO-DE-FIXTURE-4f7c"

    async def exploding_handle(self, method, params, request_id):
        raise RuntimeError("fixture internal failure")

    monkeypatch.setattr(DeviceRuntime, "handle", exploding_handle)
    _, store, _ = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with caplog.at_level(logging.DEBUG), client(environment) as http:
        with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
            assert ws.receive_json()["event"] == "ready"
            operation = str(uuid4())
            ws.send_json(
                {
                    "v": 1,
                    "id": operation,
                    "method": "chat.send",
                    "params": {"text": secret},
                }
            )
            frame = ws.receive_json()
            assert frame["event"] == "error" and frame["id"] == operation
            assert frame["data"]["code"] == "internal_error"
            assert secret not in json.dumps(frame)
        structured = [
            record
            for record in caplog.records
            if getattr(record, "event", None) and getattr(record, "code", None)
        ]
        assert structured, "internal failure was not logged with event and code"
        for record in caplog.records:
            observed = f"{record.getMessage()!s} {json.dumps(vars(record), default=str)}"
            assert secret not in observed, f"fixture message text leaked to record {record}"
