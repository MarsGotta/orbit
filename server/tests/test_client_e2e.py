"""Opt-in process E2E. Real Rust/HTTP/WS/storage; synthetic Hermes, PCM and native host.

The device is driven over its own socket, as the app drives it.

Run with ORBIT_TEST_BINARY=/absolute/path/to/orbit uv run pytest -q tests/test_client_e2e.py.
No TCC, Keychain, microphone, browser or real provider is accessed.
"""

import asyncio
import os
import tempfile
import time
from pathlib import Path

import pytest
from support_client_e2e import (
    BINARY,
    DAEMON_E2E_TIMEOUT_SECONDS,
    RustDevice,
    SilentHub,
    SyntheticHermes,
    grouped_transcript,
    tcp_server,
    until,
)
from support_synthetic_stt import SyntheticStt

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.meetings import repository
from orbit_hub.protocol import HubError
from orbit_hub.storage import Store, initialize

pytestmark = pytest.mark.skipif(
    not BINARY, reason="Requires explicit ORBIT_TEST_BINARY process fixture"
)


def turn_of(state):
    """The Mac's state carries only the turn in progress, or the last."""
    turn = state.get("turn") or {}
    return [message for message in (turn.get("user"), turn.get("answer")) if message]


async def test_rust_daemon_pair_chat_privacy_recording_and_recovery():
    binary = Path(BINARY).resolve()
    assert binary.is_file(), "Build Orbit first; ORBIT_TEST_BINARY must name its executable"
    # A short dedicated directory respects the Unix socket's 100-byte path limit.
    with tempfile.TemporaryDirectory(prefix="orbit-e2e-") as directory:
        root = Path(directory)
        data, key = root / "hub", root / "storage.key"
        initialize(data, key)
        credential = root / "hermes-fixture.key"
        fd = os.open(credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(fd, b"e2e-synthetic-hermes-token")
        finally:
            os.close(fd)
        store = Store(data, key)
        upload_receipts = []
        original_put = store.put_chunk

        def observe_upload(*arguments, **keywords):
            receipt = original_put(*arguments, **keywords)
            upload_receipts.append((arguments[3], arguments[4], time.monotonic()))
            return receipt

        store.put_chunk = observe_upload
        hermes = SyntheticHermes()
        # The hub transcribes meetings against its own STT, not Hermes.
        stt = SyntheticStt("Audio sintético de prueba.")
        device = RustDevice(binary, root / "device")
        try:
            async with tcp_server(hermes.app) as upstream, tcp_server(stt.app) as transcriber:
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
                    # The running daemon pairs, with the hub it is given,
                    # so it starts before the code is redeemed.
                    await device.start()
                    await device.request(
                        {
                            "command": "pair",
                            "hub_url": hub,
                            "code": store.create_code(time.time()),
                            "device_name": "Synthetic E2E device",
                        }
                    )
                    assert len(store.devices()) == 1
                    await until(device.state, lambda state: state["connection"] == "online")

                    await device.request({"command": "chat", "text": "E2E_BASE"})
                    state = await until(
                        device.state,
                        lambda state: state["active_request"] is None and len(turn_of(state)) == 2,
                    )
                    assert turn_of(state)[-1]["text"] == "Respuesta final sintética E2E_BASE"
                    assert hermes.prompts == ["E2E_BASE"]
                    # The turn reached Hermes saying where it came from.
                    assert hermes.contexts == ["[Orbit · Synthetic E2E device · texto]"]

                    # Provider failures must not promote partial output to a completed answer.
                    await device.request({"command": "chat", "text": "E2E_ERROR"})
                    state = await until(device.state, lambda state: state["active_request"] is None)
                    assert turn_of(state)[-1]["complete"] is False
                    assert turn_of(state)[-1]["text"] == "Texto parcial."
                    # The daemon says a code, never the hub's sentence.
                    assert state["notice"]["code"] == "assistant_failed"
                    # The conversation is read from the hub, through
                    # the daemon; the failed answer was never stored.
                    read = await device.request(
                        {
                            "command": "read",
                            "read": {
                                "kind": "session_messages",
                                "session_id": state["session_id"],
                                "cursor": None,
                            },
                        }
                    )
                    stored = read["answer"]["value"]["items"]
                    assert [item["text"] for item in stored] == [
                        "E2E_BASE",
                        "Respuesta final sintética E2E_BASE",
                        "E2E_ERROR",
                    ]
                    assert stored[2]["id"] == turn_of(state)[0]["id"]

                    await device.request({"command": "chat", "text": "E2E_SLOW"})
                    await until(device.state, lambda state: state["active_request"] is not None)
                    await device.request({"command": "interrupt"})
                    await asyncio.wait_for(hermes.interrupted.wait(), DAEMON_E2E_TIMEOUT_SECONDS)
                    await device.request({"command": "new_session"})
                    await device.request({"command": "chat", "text": "E2E_NEW"})
                    hermes.release.set()
                    state = await until(
                        device.state,
                        lambda state: (
                            state["active_request"] is None
                            and any(
                                "E2E_NEW" in item["text"] and item["role"] == "assistant"
                                for item in turn_of(state)
                            )
                        ),
                    )
                    assert all(
                        "OBSOLETE_E2E_RESPONSE" not in item["text"] for item in turn_of(state)
                    )

                    await device.request({"command": "chat", "text": "E2E_PRIVACY"})
                    await until(device.state, lambda state: state["active_request"] is not None)
                    generation = (await device.state())["generation"]
                    await device.request({"command": "privacy_stop"})
                    state = await until(
                        device.state,
                        lambda state: state["privacy_stopped"] and state["active_request"] is None,
                    )
                    assert state["generation"] > generation
                    hermes.release_privacy.set()
                    refused = await device.request(
                        {"command": "chat", "text": "MUST_NOT_SUBMIT"}, ok=False
                    )
                    assert refused["ok"] is False, refused
                    await asyncio.sleep(0.1)
                    assert "MUST_NOT_SUBMIT" not in hermes.prompts
                    assert all(
                        "OBSOLETE_E2E_RESPONSE" not in item["text"]
                        for item in turn_of(await device.state())
                    )
                    await device.emit({"v": 1, "event": "local_rearm", "privacy_stopped": False})
                    await until(
                        device.state,
                        lambda state: (
                            not state["privacy_stopped"] and state["connection"] == "online"
                        ),
                    )

                    await device.request(
                        {
                            "command": "meeting_start",
                            "title": "Synthetic meeting",
                            "audio": {"application": {"application_pid": 424242}},
                            "duration_minutes": 1,
                        }
                    )
                    state = await until(
                        device.state,
                        lambda state: state.get("recording", {}).get("phase") == "recording",
                    )
                    recording = state["recording"]["id"]
                    for sequence in range(20):
                        await device.pcm("mic_local", sequence)
                        await device.pcm("remote_app", sequence)
                    await until(
                        device.state, lambda state: state["recording"]["sealed_chunks"] == 40
                    )
                    owner = store.devices()[0]["id"]

                    async def transcript():
                        try:
                            value = grouped_transcript(store, owner, recording)
                        except HubError:
                            value = {}  # nothing of this recording reached the hub yet
                        value["e2e_uploads"] = [
                            (track, sequence, round(received - upload_receipts[0][2], 3))
                            for track, sequence, received in upload_receipts
                        ]
                        return value

                    live = await until(
                        transcript,
                        lambda value: (
                            len(value.get("segments", [])) == 2 and value.get("pending_chunks") == 0
                        ),
                        timeout=DAEMON_E2E_TIMEOUT_SECONDS,
                    )
                    assert live["capture_finalized"] is False
                    assert all(len(segment["sources"]) == 20 for segment in live["segments"])
                    epoch = live["segments"][0]["capture_epoch_id"]
                    assert all(
                        source["capture_epoch_id"] == epoch
                        for segment in live["segments"]
                        for source in segment["sources"]
                    )
                    await device.request({"command": "meeting_finish"})
                    receipt = await until(
                        transcript,
                        lambda value: (
                            value.get("capture_finalized") and value.get("transcript_ready")
                        ),
                        timeout=DAEMON_E2E_TIMEOUT_SECONDS,
                    )
                    assert receipt["capture_complete"] is True
                    assert len(receipt["segments"]) == 2
                    assert {
                        source["sha256"]
                        for segment in receipt["segments"]
                        for source in segment["sources"]
                    }
                    # Both groups went to the hub's STT, none to Hermes.
                    assert len(stt.audio) == 2 and hermes.audio_calls == 0
                    assert any(
                        "Audio sintético de prueba." in segment.get("text", "")
                        for segment in receipt["segments"]
                    )

                    # The app reads the meeting through the daemon,
                    # which reads it from the hub's routes and passes it on.
                    meeting = await until(
                        lambda: device.read({"kind": "note_detail", "id": recording}),
                        lambda page: (
                            page["state"] == "present"
                            and page["value"]["items"][0]["capture"] == "closed"
                            and page["value"]["items"][0]["transcript"]["state"] == "ready"
                        ),
                        timeout=DAEMON_E2E_TIMEOUT_SECONDS,
                    )
                    document = meeting["value"]["items"][0]

                    async def owner_name():
                        with store.read_snapshot() as db:
                            return repository.owner_name(store, db, owner)

                    # On connecting, the daemon told the hub who owns this Mac.
                    named = await until(owner_name, lambda name: name is not None)
                    names = {speaker["key"]: speaker["name"] for speaker in document["speakers"]}
                    assert set(names) == {"mic_local", "remote_app"}
                    assert names["mic_local"] == named
                    assert document["title"] == "Synthetic meeting"
                    # Finalizing the meeting asked for its note.
                    assert document["note"] is not None
                    listing = await device.read({"kind": "notes_page", "cursor": None})
                    assert recording in {
                        item.get("recording_id") for item in listing["value"]["items"]
                    }
                    turns = await device.read(
                        {"kind": "note_transcript", "id": recording, "cursor": None}
                    )
                    items = turns["value"]["items"]
                    assert items and all(item["text"].strip() for item in items)
                    assert all(0 <= item["at_ms"] <= document["duration_ms"] for item in items)
                    assert {item["speaker_key"] for item in items} <= set(names)
                    await device.request(
                        {
                            "command": "note_private_save",
                            "recording_id": recording,
                            "markdown": "- nota privada sintética",
                            "base_revision": 0,
                        }
                    )

                    async def private_notes():
                        with store.read_snapshot() as db:
                            return repository.private_notes(store, db, owner, recording)

                    saved = await until(private_notes, lambda value: value[1] == 1)
                    assert saved == ("- nota privada sintética", 1)

                    before_crash = await device.state()
                    before_prompts = list(hermes.prompts)
                    await device.close(crash=True)
                    await device.start()
                    recovered = await device.state()
                    # A crash nobody asked for comes back working.
                    assert recovered["privacy_stopped"] is False
                    assert recovered["generation"] > before_crash["generation"]
                    assert recovered["recording"]["sealed_chunks"] == 40
                    assert hermes.prompts == before_prompts  # no prompt replay on crash recovery
                    await device.close()
        finally:
            await device.close()
            store.close()


async def test_a_recording_keeps_going_while_the_hub_does_not_answer():
    """The hub stops answering mid-meeting and comes back: the recording never
    stops, its audio waits on the Mac and reaches the hub once each, in full."""
    binary = Path(BINARY).resolve()
    with tempfile.TemporaryDirectory(prefix="orbit-e2e-") as directory:
        root = Path(directory)
        data, key = root / "hub", root / "storage.key"
        initialize(data, key)
        credential = root / "hermes-fixture.key"
        fd = os.open(credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(fd, b"e2e-synthetic-hermes-token")
        finally:
            os.close(fd)
        store = Store(data, key)
        deliveries = []
        original_put = store.put_chunk

        def observe_upload(*arguments, **keywords):
            receipt = original_put(*arguments, **keywords)
            deliveries.append((arguments[3], arguments[4], hub_answering()))
            return receipt

        store.put_chunk = observe_upload
        hermes = SyntheticHermes()
        stt = SyntheticStt("Audio sintético de prueba.")
        device = RustDevice(binary, root / "device")
        hub = None

        def hub_answering():
            return hub is None or hub.answering.is_set()

        try:
            async with tcp_server(hermes.app) as upstream, tcp_server(stt.app) as transcriber:
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
                async with tcp_server(create_app(settings, store=store)) as served:
                    hub = SilentHub(served)
                    async with hub as relay:
                        await device.start()
                        await device.request(
                            {
                                "command": "pair",
                                "hub_url": relay,
                                "code": store.create_code(time.time()),
                                "device_name": "Synthetic E2E device",
                            }
                        )
                        owner = store.devices()[0]["id"]
                        await until(device.state, lambda state: state["connection"] == "online")
                        await device.request(
                            {
                                "command": "meeting_start",
                                "title": "Reunión sintética sin hub",
                                "audio": {"application": {"application_pid": 424242}},
                            }
                        )
                        state = await until(
                            device.state,
                            lambda state: state.get("recording", {}).get("phase") == "recording",
                        )
                        recording = state["recording"]["id"]

                        async def hub_chunks():
                            with store.read_snapshot() as db:
                                rows = db.execute(
                                    "SELECT track, sequence FROM chunks"
                                    " WHERE device_id=? AND recording=?",
                                    (owner, recording),
                                ).fetchall()
                            return sorted((row["track"], row["sequence"]) for row in rows)

                        # A few seconds per phase: each is two uploads to a hub
                        # served by this same process, and what waited on the
                        # Mac must catch up within one wait under the gate's load.
                        sequence = 0

                        async def speak(seconds):
                            nonlocal sequence
                            for _ in range(seconds):
                                await device.pcm("mic_local", sequence)
                                await device.pcm("remote_app", sequence)
                                sequence += 1

                        await speak(3)
                        await until(hub_chunks, lambda rows: len(rows) == 6)

                        # The hub stops answering, from a chunk it stores. The
                        # meeting goes on: what is said is sealed on the Mac.
                        hub.silence_from(b"PUT /v1/chunks/")
                        await speak(4)
                        await until(
                            device.state, lambda state: state["recording"]["sealed_chunks"] == 14
                        )
                        # Long enough for the Mac to give up on the hub.
                        offline = await until(
                            device.state, lambda state: state["connection"] != "online"
                        )
                        assert offline["recording"]["phase"] == "recording", offline["recording"]
                        await speak(4)
                        state = await until(
                            device.state, lambda state: state["recording"]["sealed_chunks"] == 22
                        )
                        assert state["recording"]["phase"] == "recording", state["recording"]
                        assert state["recording"]["gaps"] == 0, state["recording"]

                        # The hub answers again: what waited on the Mac arrives.
                        hub.answer_again()
                        await until(device.state, lambda state: state["connection"] == "online")
                        await speak(2)
                        # Every second of both tracks, once each.
                        rows = await until(hub_chunks, lambda rows: len(rows) == 26)
                        for track in ("mic_local", "remote_app"):
                            assert [sequence for name, sequence in rows if name == track] == list(
                                range(13)
                            ), track
                        # Among them, the chunk the hub stored while its answer
                        # was lost.
                        assert any(not answered for *_, answered in deliveries), deliveries

                        await device.request({"command": "meeting_finish"})

                        async def capture():
                            return store.groups.capture(owner, recording)

                        closed = await until(capture, lambda value: value.get("capture_finalized"))
                        assert closed["capture_complete"] is True, closed
                        await device.close()
        finally:
            await device.close()
            store.close()
