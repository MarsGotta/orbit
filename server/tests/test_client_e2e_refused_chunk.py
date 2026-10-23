"""Opt-in process E2E: a chunk the hub refuses for good, in the middle of a
track, does not keep the meeting open. Real Rust daemon, HTTP and storage;
synthetic STT and native host.

Run with ORBIT_TEST_BINARY=/absolute/path/to/orbit uv run pytest -q
tests/test_client_e2e_refused_chunk.py. No TCC, Keychain, microphone or real
provider is accessed.
"""

import json
import tempfile
import time
from pathlib import Path

import pytest
from support_client_e2e import (
    BINARY,
    DAEMON_E2E_TIMEOUT_SECONDS,
    RustDevice,
    SyntheticHermes,
    grouped_transcript,
    tcp_server,
    until,
)
from support_synthetic_stt import SyntheticStt

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.protocol import HubError
from orbit_hub.storage import Store, initialize

pytestmark = pytest.mark.skipif(
    not BINARY, reason="Requires explicit ORBIT_TEST_BINARY process fixture"
)

SECONDS = 6
REFUSED = ("mic_local", 3)


async def test_a_chunk_refused_in_the_middle_is_declared_and_the_meeting_closes():
    binary = Path(BINARY).resolve()
    assert binary.is_file(), "Build Orbit first; ORBIT_TEST_BINARY must name its executable"
    with tempfile.TemporaryDirectory(prefix="orbit-e2e-") as directory:
        root = Path(directory)
        data, key = root / "hub", root / "storage.key"
        initialize(data, key)
        credential = root / "hermes-fixture.key"
        credential.write_bytes(b"e2e-synthetic-hermes-token")
        credential.chmod(0o600)
        store = Store(data, key)
        original_put = store.recordings.put_chunk
        refusals = []

        def refuse_one(*arguments, **keywords):
            # The hub refuses this chunk for good, as it refuses a chunk whose
            # id it already holds with other bytes.
            if (arguments[3], arguments[4]) == REFUSED:
                refusals.append(REFUSED)
                raise HubError("chunk_conflict", "Fragmento con otro contenido")
            return original_put(*arguments, **keywords)

        store.recordings.put_chunk = refuse_one
        stt = SyntheticStt("Audio sintético de prueba.")
        device = RustDevice(binary, root / "device")
        try:
            async with (
                tcp_server(SyntheticHermes().app) as upstream,
                tcp_server(stt.app) as transcriber,
            ):
                settings = Settings(
                    profile="deimos",
                    data_dir=data,
                    key_file=key,
                    hermes_url=upstream,
                    hermes_credential_file=credential,
                    meeting_transcription=stt.configuration(transcriber),
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
                            "device_name": "Synthetic E2E device",
                        }
                    )
                    await until(device.state, lambda state: state["connection"] == "online")
                    await device.request(
                        {
                            "command": "meeting_start",
                            "title": "Synthetic meeting with a hole",
                            "audio": {"application": {"application_pid": 424242}},
                            "duration_minutes": 1,
                        }
                    )
                    state = await until(
                        device.state,
                        lambda state: state.get("recording", {}).get("phase") == "recording",
                    )
                    recording = state["recording"]["id"]
                    for sequence in range(SECONDS):
                        await device.pcm("mic_local", sequence)
                        await device.pcm("remote_app", sequence)
                    await until(
                        device.state,
                        lambda state: state["recording"]["sealed_chunks"] == 2 * SECONDS,
                    )
                    await until(lambda: _value(refusals), lambda value: value == [REFUSED])
                    await device.request({"command": "meeting_finish"})
                    owner = store.devices.all()[0]["id"]

                    async def transcript():
                        try:
                            return grouped_transcript(store, owner, recording)
                        except HubError:
                            return {}

                    closed = await until(
                        transcript,
                        lambda value: (
                            value.get("capture_finalized") and value.get("transcript_ready")
                        ),
                        timeout=DAEMON_E2E_TIMEOUT_SECONDS,
                    )
                    # Closed, honestly incomplete, with every other chunk.
                    assert closed["capture_complete"] is False
                    held = {
                        (segment["track"], source["sequence"])
                        for segment in closed["segments"]
                        for source in segment["sources"]
                    }
                    expected = {
                        (track, sequence)
                        for track in ("mic_local", "remote_app")
                        for sequence in range(SECONDS)
                    } - {REFUSED}
                    assert held == expected
                    with store.read_snapshot() as db:
                        row = db.execute(
                            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
                            (owner, recording),
                        ).fetchone()
                    manifest = json.loads(
                        store.decrypt(row["body"], f"manifest:{owner}:{recording}")
                    )
                    mic = next(t for t in manifest["tracks"] if t["track"] == "mic_local")
                    assert mic["missing_sequences"] == [3]
                    assert mic["chunk_count"] == SECONDS - 1
                    assert mic["last_sequence"] == SECONDS - 1
                    assert manifest["gaps"] >= 1
                    # The Mac reads the meeting closed, saying what is missing.
                    meeting = await until(
                        lambda: device.read({"kind": "note_detail", "id": recording}),
                        lambda page: (
                            page["state"] == "present"
                            and page["value"]["items"][0]["capture"] == "closed"
                        ),
                        timeout=DAEMON_E2E_TIMEOUT_SECONDS,
                    )
                    missing = meeting["value"]["items"][0]["transcript"]["missing_audio"]
                    assert [(hole["track"], hole["chunks"]) for hole in missing] == [
                        ("mic_local", 1)
                    ]
                    assert missing[0]["at_ms"] is not None
                    await device.close()
        finally:
            await device.close()
            store.close()


async def _value(value):
    return list(value)
