import asyncio
import hashlib
import threading
import time
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_audio import wav_bytes

from orbit_hub.app import create_app


def eight_mib_wav() -> bytes:
    return wav_bytes(b"\0\0" * (4 * 1024 * 1024 - 22))


def test_ping_is_answered_while_an_8mib_chunk_persist_waits_for_release(environment, monkeypatch):
    settings, store, _ = environment
    paired = device(store)
    recording, chunk = str(uuid4()), str(uuid4())
    body = eight_mib_wav()
    assert len(body) == settings.max_chunk_bytes
    headers = {
        "Authorization": "Bearer " + paired["token"],
        "X-Orbit-Recording": recording,
        "X-Orbit-Track": "mic_local",
        "X-Orbit-Sequence": "0",
        "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
    }
    persist = store.put_chunk
    persist_started = threading.Event()
    release_persist = threading.Event()
    pong_received = threading.Event()

    def delayed_persist(*arguments, **keywords):
        persist_started.set()
        release_persist.wait()
        return persist(*arguments, **keywords)

    monkeypatch.setattr(store, "put_chunk", delayed_persist)
    with (
        client(environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        outcome = {}

        def upload():
            outcome["status"] = http.put(
                f"/v1/chunks/{chunk}", content=body, headers=headers
            ).status_code

        uploader = threading.Thread(target=upload)
        uploader.start()
        try:
            assert persist_started.wait(10)
            operation = str(uuid4())

            def ping():
                ws.send_json({"v": 1, "id": operation, "method": "ping", "params": {}})
                outcome["pong"] = ws.receive_json()
                pong_received.set()

            pinger = threading.Thread(target=ping)
            pinger.start()
            assert pong_received.wait(10), "pong blocked behind the unfinished store write"
            assert not release_persist.is_set()
            assert "status" not in outcome
            assert outcome["pong"]["event"] == "pong" and outcome["pong"]["id"] == operation
        finally:
            release_persist.set()
            uploader.join(15)
            if "pinger" in locals():
                pinger.join(15)
        assert outcome["status"] == 200


class RawWebsocket:
    """One device websocket driven straight through the ASGI interface.

    ``stall_after`` turns the transport into a client that stops reading: once
    that many data frames have been delivered, every further server write
    awaits forever, exactly like a socket whose peer stopped draining it.
    """

    def __init__(self, app, token: str, *, stall_after: int | None):
        self.inbox: asyncio.Queue = asyncio.Queue()
        self.inbox.put_nowait({"type": "websocket.connect"})
        self.stall_after = stall_after
        self.delivered = 0
        self.first_frame = asyncio.Event()
        self.stalled = asyncio.Event()
        self.finished = asyncio.Event()
        self.close_code = None
        self.stalled_at = 0.0
        scope = {
            "type": "websocket",
            "asgi": {"version": "3.0", "spec_version": "2.3"},
            "http_version": "1.1",
            "scheme": "wss",
            "path": "/v1/ws",
            "raw_path": b"/v1/ws",
            "query_string": b"",
            "root_path": "",
            "headers": [
                (b"host", b"localhost"),
                (b"authorization", b"Bearer " + token.encode()),
            ],
            "client": ("127.0.0.1", 0),
            "server": ("localhost", 0),
            "subprotocols": [],
        }

        async def session():
            try:
                await app(scope, self.receive, self.send)
            finally:
                self.finished.set()

        self.task = asyncio.create_task(session())

    async def receive(self):
        return await self.inbox.get()

    async def send(self, message):
        if message["type"] == "websocket.close":
            self.close_code = message["code"]
            return
        if message["type"] != "websocket.send":
            return
        if self.stall_after is not None and self.delivered >= self.stall_after:
            self.stalled_at = time.monotonic()
            self.stalled.set()
            await asyncio.Event().wait()
        self.delivered += 1
        self.first_frame.set()


async def test_slow_client_is_evicted_within_ten_seconds_and_its_slot_freed(environment):
    settings, store, adapter = environment
    token = device(store)["token"]
    app = create_app(settings, store=store, adapter=adapter, now=lambda: 1001)
    sockets = []
    try:
        slow = RawWebsocket(app, token, stall_after=0)
        sockets.append(slow)
        await asyncio.wait_for(slow.stalled.wait(), 5)
        for _ in range(3):
            filler = RawWebsocket(app, token, stall_after=None)
            sockets.append(filler)
            await asyncio.wait_for(filler.first_frame.wait(), 5)
        rejected = RawWebsocket(app, token, stall_after=None)
        sockets.append(rejected)
        await asyncio.wait_for(rejected.finished.wait(), 5)
        assert rejected.close_code == 4429
        try:
            await asyncio.wait_for(slow.finished.wait(), 12)
        except TimeoutError:
            pytest.fail("the client that never reads kept its slot beyond the 10 s write deadline")
        assert time.monotonic() - slow.stalled_at <= 12
        replacement = RawWebsocket(app, token, stall_after=None)
        sockets.append(replacement)
        await asyncio.wait_for(replacement.first_frame.wait(), 5)
        assert replacement.close_code is None
    finally:
        for socket in sockets:
            socket.task.cancel()
        await asyncio.gather(*(socket.task for socket in sockets), return_exceptions=True)
