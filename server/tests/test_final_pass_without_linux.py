"""The final pass on a hub without Linux, and on Linux as today.

Without Linux, the audio of a track goes to a private file in the temp folder
(`tempfile.gettempdir()`), readable only by the hub's user and gone when the
pass ends, also when it fails. On Linux it stays in memory: no file in any
folder. A hub without Linux is simulated with what decides it,
`orbit_hub.host_system.linux`, and without `os.memfd_create`, which macOS does
not have. ffmpeg is the real one, behind a wrapper on PATH that records its
arguments; the separator is a fake executable at the process boundary, like
the one in test_final_pass, that records what it was given.
"""

import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

import pytest
from conftest import device
from test_final_pass import fake_whisper, final_worker
from test_meeting_views import TEXTS, meeting

FFMPEG = shutil.which("ffmpeg")
VOICES = {
    "spans": [
        {"start_ms": 0, "end_ms": 10_000, "voice": "A"},
        {"start_ms": 10_000, "end_ms": 20_000, "voice": "B"},
        {"start_ms": 20_000, "end_ms": 30_000, "voice": "A"},
    ],
}


def without_linux(monkeypatch):
    from orbit_hub import host_system

    monkeypatch.setattr(host_system, "linux", lambda: False)
    monkeypatch.delattr(os, "memfd_create", raising=False)


def temp_folder(tmp_path, monkeypatch):
    folder = tmp_path / "temp"
    folder.mkdir(mode=0o700)
    monkeypatch.setattr(tempfile, "tempdir", str(folder))
    monkeypatch.setenv("TMPDIR", str(folder))
    return folder.resolve()


def recording_ffmpeg(tmp_path, monkeypatch):
    """The real ffmpeg, first on PATH behind a wrapper that writes each argument."""
    assert FFMPEG, "ffmpeg is required, as for the rest of the final pass"
    folder = tmp_path / "bin"
    folder.mkdir()
    log = tmp_path / "ffmpeg-arguments.txt"
    wrapper = folder / "ffmpeg"
    wrapper.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{log}'\nexec '{FFMPEG}' \"$@\"\n")
    wrapper.chmod(0o700)
    monkeypatch.setenv("PATH", f"{folder}{os.pathsep}{os.environ['PATH']}")
    return log


def recording_separator(tmp_path, temp, *, exit_status=0):
    """A separator that writes what it received: the path, the file and the audio."""
    from orbit_hub.meeting_diarization import DiarizationConfiguration

    record = tmp_path / "separator.jsonl"
    executable = tmp_path / "fake-python"
    executable.write_text(
        "#!/usr/bin/python3\n"
        "import hashlib, json, os, stat, sys, wave\n"
        "path = sys.argv[2]\n"
        "info = os.stat(path)\n"
        "with wave.open(path, 'rb') as wav:\n"
        "    shape = [wav.getframerate(), wav.getnchannels(), wav.getnframes()]\n"
        "    digest = hashlib.sha256(wav.readframes(wav.getnframes())).hexdigest()\n"
        "real = os.path.realpath(path)\n"
        "seen = {'path': path, 'real': real, 'on_disk': os.path.exists(real),\n"
        "        'regular': stat.S_ISREG(info.st_mode), 'mode': stat.S_IMODE(info.st_mode),\n"
        f"        'shape': shape, 'digest': digest, 'temp': sorted(os.listdir({str(temp)!r}))}}\n"
        f"with open({str(record)!r}, 'a') as handle:\n"
        "    handle.write(json.dumps(seen) + '\\n')\n"
        f"print({json.dumps(VOICES)!r})\n"
        f"sys.exit({exit_status})\n"
    )
    executable.chmod(0o700)
    configuration = DiarizationConfiguration(
        python=executable, bundle=tmp_path / "unused-bundle", gpu_lock=tmp_path / "unused-lock"
    )
    return configuration, record


def records(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def final_state(store):
    row = store._db.execute("SELECT state, error FROM recording_final_passes").fetchone()
    return row["state"], json.loads(row["error"]) if row["error"] else None


def remote_turns(store, owner, identity):
    from orbit_hub.meetings import turns

    with store.read_snapshot() as db:
        transcript = turns.read(store, db, owner, identity)
    return [
        (s.at_ms, s.end_ms, s.speaker_key, s.text)
        for s in transcript.segments
        if s.track == "remote_app"
    ]


async def test_without_linux_the_audio_is_a_private_file_that_is_gone_afterwards(
    environment, tmp_path, monkeypatch
):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    temp = temp_folder(tmp_path, monkeypatch)
    arguments = recording_ffmpeg(tmp_path, monkeypatch)
    without_linux(monkeypatch)
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    transcriber, requests = fake_whisper()
    FinalPassRepository(store).request(owner, identity)
    configuration, record = recording_separator(tmp_path, temp)

    assert await final_worker(store, transcriber, configuration).run_once()

    assert final_state(store) == ("completed", None)
    [seen] = records(record)
    # A real file, only for the hub's user, in the temp folder while it is used.
    assert seen["regular"] and seen["on_disk"]
    assert seen["mode"] == 0o600
    assert Path(seen["real"]).is_relative_to(temp)
    assert seen["shape"] == [16000, 1, 484800]
    given = arguments.read_text().splitlines()
    assert given, "ffmpeg converted the tracks"
    assert [a for a in given if a.startswith(("/proc/", "/dev/fd/"))] == []
    # Nothing is left behind once the pass ends.
    assert list(temp.iterdir()) == []
    assert remote_turns(store, owner, identity) == [
        (1000, 2000, "voice_1", "Uno."),
        (11000, 12000, "voice_2", "Dos."),
        (21000, 22000, "voice_1", "Tres."),
    ]
    assert len(requests) == 2


async def test_the_separator_hears_the_same_audio_with_and_without_linux(
    environment, tmp_path, monkeypatch
):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    temp = temp_folder(tmp_path, monkeypatch)
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    transcriber, _ = fake_whisper()
    configuration, record = recording_separator(tmp_path, temp)
    worker = final_worker(store, transcriber, configuration)
    repository = FinalPassRepository(store)
    repository.request(owner, identity)
    assert await worker.run_once()
    on_linux = remote_turns(store, owner, identity)

    without_linux(monkeypatch)
    # A manual repeat separates the voices again, now without Linux.
    repository.request(owner, identity)
    assert await worker.run_once()

    assert final_state(store) == ("completed", None)
    first, second = records(record)
    assert first["shape"] == second["shape"] == [16000, 1, 484800]
    assert first["digest"] == second["digest"]
    assert remote_turns(store, owner, identity) == on_linux
    assert list(temp.iterdir()) == []


async def test_without_linux_a_failed_pass_also_removes_its_file(
    environment, tmp_path, monkeypatch
):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    temp = temp_folder(tmp_path, monkeypatch)
    without_linux(monkeypatch)
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    transcriber, _ = fake_whisper()
    FinalPassRepository(store).request(owner, identity)
    configuration, record = recording_separator(tmp_path, temp, exit_status=17)

    assert await final_worker(store, transcriber, configuration).run_once()

    state, error = final_state(store)
    assert state == "failed"
    assert error["code"] == "diarization_process_failed" and error["exit_status"] == 17
    [seen] = records(record)
    assert seen["temp"] != [], "the separator was given a file in the temp folder"
    assert list(temp.iterdir()) == []


@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="the Linux hub's own path")
async def test_on_linux_the_audio_stays_in_memory_as_today(environment, tmp_path, monkeypatch):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    temp = temp_folder(tmp_path, monkeypatch)
    arguments = recording_ffmpeg(tmp_path, monkeypatch)
    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    transcriber, _ = fake_whisper()
    FinalPassRepository(store).request(owner, identity)
    configuration, record = recording_separator(tmp_path, temp)

    assert await final_worker(store, transcriber, configuration).run_once()

    assert final_state(store) == ("completed", None)
    [seen] = records(record)
    # No directory entry anywhere, and nothing in the temp folder, ever.
    assert seen["regular"] and not seen["on_disk"]
    assert seen["temp"] == []
    assert seen["shape"] == [16000, 1, 484800]
    assert [a for a in arguments.read_text().splitlines() if a.startswith(str(temp))] == []
    assert list(temp.iterdir()) == []
