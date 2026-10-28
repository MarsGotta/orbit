"""Work the hub was doing when it stopped is never left stuck.

A hub restart in the middle of an answer cuts that turn: the conversation
takes the next message, and that turn is told the previous answer was cut. A
transcription group whose outcome could not be written (a full disk, twice)
is transcribed again once the worker comes back, never left `running`. The
agent, the transcriber and every name are synthetic.
"""

import asyncio
import itertools
import sqlite3
from uuid import uuid4

import pytest
from conftest import FakeConnection, device, only
from support_transcription import insert_job

from orbit_hub.assistant.backend import ResumedSession
from orbit_hub.meeting_transcription import Transcription
from orbit_hub.protocol import HubError, SendParams
from orbit_hub.runtime import DeviceRuntime
from orbit_hub.transcription import TranscriptionWorker

DISK_FULL = "database or disk is full"


async def unfinished(connection, method, params):
    """A Hermes that takes the turn and has not answered when the hub stops."""
    connection.calls.append((method, params))
    return {"status": "streaming" if method == "prompt.submit" else "ok"}


async def reopened(connection, stored_id):
    """Hermes reopens the conversation, its history already loaded."""
    live = str(uuid4())
    connection.live_sessions.add(live)
    return ResumedSession(live, stored_id, None, False)


def frames_of(queue) -> list[dict]:
    frames = []
    while not queue.empty():
        frames.append(queue.get_nowait())
    return frames


async def test_a_conversation_a_hub_restart_cut_takes_the_next_message(environment, monkeypatch):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    with monkeypatch.context() as patched:
        patched.setattr(FakeConnection, "rpc", unfinished)
        before = DeviceRuntime(owner, store, only(adapter, "Lyra"), lambda: 1000)
        _, sent = await before.handle(
            "chat.send", SendParams(text="Primera pregunta de prueba"), str(uuid4())
        )
        await before.close()
    session = sent["session_id"]
    store.recover_interrupted()  # what the next hub start does
    monkeypatch.setattr(FakeConnection, "resume_session", reopened)
    after = DeviceRuntime(owner, store, only(adapter, "Lyra"), lambda: 2000)
    frames = after.subscribe()
    turn = str(uuid4())
    try:
        name, data = await after.handle(
            "chat.send", SendParams(text="Segunda pregunta de prueba", session_id=session), turn
        )
    except HubError as refusal:
        pytest.fail(f"{refusal.code}: {refusal.message}")
    finally:
        await after.close()
    assert name == "chat.session" and data["session_id"] == session
    # The new turn is told, as itself, that the answer before it was cut.
    [said] = [
        frame
        for frame in frames_of(frames)
        if frame["event"] == "chat.status" and frame["data"]["kind"] == "warning"
    ]
    assert said["id"] == turn and said["data"]["session_id"] == session
    assert said["data"]["text"] == (
        "The previous response from Lyra did not arrive: it was cut and the hub has restarted since then"
    )


async def test_a_conversation_cut_by_a_restart_says_it_only_once(environment, monkeypatch):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    with monkeypatch.context() as patched:
        patched.setattr(FakeConnection, "rpc", unfinished)
        before = DeviceRuntime(owner, store, only(adapter), lambda: 1000)
        _, sent = await before.handle("chat.send", SendParams(text="Primera"), str(uuid4()))
        await before.close()
    store.recover_interrupted()
    monkeypatch.setattr(FakeConnection, "resume_session", reopened)
    after = DeviceRuntime(owner, store, only(adapter), lambda: 2000)
    frames = after.subscribe()
    try:
        for text in ("Segunda", "Tercera"):
            await after.handle(
                "chat.send", SendParams(text=text, session_id=sent["session_id"]), str(uuid4())
            )
    finally:
        await after.close()
    warnings = [
        frame
        for frame in frames_of(frames)
        if frame["event"] == "chat.status" and frame["data"]["kind"] == "warning"
    ]
    assert len(warnings) == 1


class Transcriber:
    async def transcribe(self, audio, language=None, prompt=""):
        return Transcription("Transcripción sintética de una reunión de prueba", "es")


async def test_a_group_whose_outcome_could_not_be_written_is_transcribed_again(
    environment, monkeypatch
):
    monkeypatch.setattr("orbit_hub.log.RELAUNCH_BACKOFF_SECONDS", (0,))
    monkeypatch.setattr("orbit_hub.transcription.PREPARE_TICK_SECONDS", 0.05)
    _, store, adapter = environment
    owner = device(store)["device_id"]
    _, recording = insert_job(store, owner)
    full = {"finish": 1, "fail": 1}

    def failing(name, original):
        def write(*args, **kwargs):
            if full[name]:
                full[name] -= 1
                raise sqlite3.OperationalError(DISK_FULL)
            return original(*args, **kwargs)

        return write

    monkeypatch.setattr(store.groups, "finish", failing("finish", store.groups.finish))
    monkeypatch.setattr(store.groups, "fail", failing("fail", store.groups.fail))
    clock = itertools.count(1000, 600)  # every backoff is due on the next look
    worker = TranscriptionWorker(store, adapter, lambda: next(clock), meetings=Transcriber())
    worker.start()
    loop = asyncio.get_running_loop()
    deadline = loop.time() + 3
    try:
        while loop.time() < deadline:
            if store.groups.capture(owner, recording)["pending_chunks"] == 0:
                break
            await asyncio.sleep(0.05)
    finally:
        await worker.close()
    assert full == {"finish": 0, "fail": 0}
    states = [
        row[0]
        for row in store._db.execute(
            "SELECT state FROM transcription_groups WHERE device_id=? AND recording=?",
            (owner, recording),
        )
    ]
    assert states == ["completed"]
    assert store.groups.capture(owner, recording)["pending_chunks"] == 0
