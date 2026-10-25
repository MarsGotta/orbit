"""Voice notices over a real daemon subscription; isolated hub and synthetic I/O.

No microphone, Hermes profile or host action is used. The STT gate keeps the
working phase observable without depending on provider speed or sleeps.
"""

import asyncio
import base64
import json
import os
import tempfile
import time
from pathlib import Path

import pytest
from fastapi import Request
from fastapi.responses import JSONResponse
from support_audio import wav_bytes
from support_client_e2e import (
    BINARY,
    DAEMON_E2E_TIMEOUT_SECONDS,
    MAX_RESPONSE_BYTES,
    RustDevice,
    SyntheticHermes,
    tcp_server,
    until,
)

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.storage import Store, initialize

pytestmark = pytest.mark.skipif(not BINARY, reason="Requires ORBIT_TEST_BINARY")


@pytest.mark.parametrize("outcome", ["answered", "transcription_failed"])
async def test_voice_working_notices_end_over_the_daemon_subscription(outcome, unit_stt):
    peer = SyntheticHermes()
    entered, release = asyncio.Event(), asyncio.Event()
    route = next(route for route in peer.app.routes if route.path == "/api/audio/transcribe")
    transcribe = route.endpoint
    peer.app.router.routes.remove(route)

    @peer.app.post("/api/audio/transcribe")
    async def gated_transcription(request: Request):
        entered.set()
        await release.wait()
        if outcome == "transcription_failed":
            return JSONResponse({"code": "synthetic_stt_failure"}, status_code=503)
        return await transcribe(request)

    @peer.app.post("/api/audio/speak")
    async def speak():
        audio = base64.b64encode(wav_bytes(b"\x00\x01" * 1600)).decode()
        return {
            "ok": True,
            "data_url": f"data:audio/wav;base64,{audio}",
            "mime_type": "audio/wav",
            "provider": "fixture",
        }

    with tempfile.TemporaryDirectory(prefix="notice-e2e-") as directory:
        root = Path(directory)
        initialize(root / "hub", root / "key")
        credential = root / "credential"
        descriptor = os.open(credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            output.write("e2e-synthetic-hermes-token")
        store = Store(root / "hub", root / "key")
        device = RustDevice(Path(BINARY).resolve(), root / "device")
        reader_task, writer = None, None
        received = []

        async def collect(reader):
            while line := await reader.readline():
                event = json.loads(line)
                if "frame" not in event:
                    continue
                knowledge = event["frame"]["data"]["knowledge"]
                assert knowledge["state"] == "present", knowledge
                received.extend(knowledge["value"]["notices"])

        async def commands():
            return device.native_commands

        async def observations():
            if reader_task is not None and reader_task.done():
                reader_task.result()
            return received

        try:
            async with tcp_server(peer.app) as upstream:
                settings = Settings(
                    profile="deimos",
                    data_dir=root / "hub",
                    key_file=root / "key",
                    hermes_url=upstream,
                    hermes_credential_file=credential,
                    meeting_transcription=unit_stt.configuration(),
                    allow_insecure_localhost=True,
                    allowed_hosts=("127.0.0.1", "localhost"),
                )
                async with tcp_server(create_app(settings, store=store)) as hub:
                    await device.start()
                    await device.request(
                        {
                            "command": "pair",
                            "hub_url": hub,
                            "code": store.devices.create_code(time.time()),
                            "device_name": "Notice fixture",
                        }
                    )
                    await until(device.state, lambda state: state["connection"] == "online")
                    reader, writer = await asyncio.open_unix_connection(
                        str(device.data / "orbit.sock"), limit=MAX_RESPONSE_BYTES
                    )
                    writer.write(
                        json.dumps(
                            {
                                "v": 1,
                                "id": "notice-fixture",
                                "body": {"command": "subscribe_topics", "topics": ["notices"]},
                            }
                        ).encode()
                        + b"\n"
                    )
                    await writer.drain()
                    reader_task = asyncio.create_task(collect(reader))
                    completed_ops, elapsed = set(), []
                    for note_index in range(3):
                        started = time.monotonic()
                        entered.clear()
                        release.clear()
                        await device.emit({"event": "hotkey", "action": "ptt", "phase": "pressed"})

                        async def streams():
                            return device.streams

                        await until(streams, bool)
                        await device.pcm(next(iter(device.streams)))
                        await device.emit({"event": "hotkey", "action": "ptt", "phase": "released"})
                        await asyncio.wait_for(entered.wait(), DAEMON_E2E_TIMEOUT_SECONDS)
                        notices = await until(
                            observations,
                            lambda items: any(
                                item["code"] == "voice_transcribing"
                                and item["op"] not in completed_ops
                                for item in items
                            ),
                        )
                        op = next(
                            item["op"]
                            for item in reversed(notices)
                            if item["code"] == "voice_transcribing"
                            and item["op"] not in completed_ops
                        )
                        release.set()
                        terminal = (
                            "assistant_answered"
                            if outcome == "answered"
                            else "voice_transcription_failed"
                        )
                        await until(
                            observations,
                            lambda items, op=op, terminal=terminal: any(
                                item["op"] == op and item["code"] == terminal for item in items
                            ),
                        )
                        if outcome == "answered":
                            # Starting the next PTT cancels an unfinished speech
                            # fetch. Hear this answer before superseding it; the
                            # three-clip assertion must not rely on stale work.
                            await until(
                                commands,
                                lambda items, expected=note_index + 1: (
                                    sum(item["command"] == "playback_clip" for item in items)
                                    == expected
                                ),
                            )
                        completed_ops.add(op)
                        elapsed.append(round(time.monotonic() - started, 3))
                    if outcome == "answered":
                        assert peer.audio_calls == 3

                        await until(
                            commands,
                            lambda items: (
                                sum(item["command"] == "playback_clip" for item in items) == 3
                            ),
                        )
                    reports = [
                        json.loads(line)
                        for line in (device.data / "logs/report-daemon.log")
                        .read_text()
                        .splitlines()
                    ]
                    classes = {(row["op"], row["code"]): row["class"] for row in reports}
                    working, seen = set(), set()
                    for notice in received:
                        key = (notice["op"], notice["code"])
                        if key in seen:
                            continue
                        seen.add(key)
                        if classes[key] == "working":
                            working.add(notice["op"])
                        else:
                            working.discard(notice["op"])
                    assert len(completed_ops) == 3
                    assert working == set(), f"Unfinished operations after {outcome}: {working}"
                    print(f"{outcome}: notes=3 working=0 elapsed_s={elapsed}")
        finally:
            release.set()
            if reader_task:
                reader_task.cancel()
                await asyncio.gather(reader_task, return_exceptions=True)
            if writer:
                writer.close()
                await writer.wait_closed()
            await device.close()
            store.close()
