"""Calendar bounds and replacement identities at the real worker/HTTP boundaries."""

import json
from contextlib import closing

import pytest
from conftest import device
from test_final_pass import fake_separator, fake_whisper, final_worker
from test_meeting_language_and_title import routes_only
from test_meeting_views import TEXTS, bearer, meeting
from test_retranscribe import app_for

from orbit_hub.meeting_final_pass import FinalPassRepository
from orbit_hub.meetings import repository, turns


@pytest.mark.parametrize(
    "guests,expected",
    [
        (None, None),
        ([], None),
        ([True], 1),
        ([True, False, False, False, False], 4),
        ([False, False], 2),
    ],
)
async def test_calendar_bound_reaches_the_separator_process(
    environment, tmp_path, guests, expected
):
    _, store, _ = environment
    identity = device(store)["device_id"]
    event = (
        None
        if guests is None
        else {
            "calendar_item_id": "synthetic-series",
            "attendees": [{"name": f"Guest {i}", "is_self": me} for i, me in enumerate(guests)],
        }
    )
    recording = meeting(store, identity, texts=TEXTS, event=event)
    configuration = fake_separator(tmp_path)
    source = configuration.python.read_text()
    configuration.python.write_text(
        source.replace(
            "import sys, wave",
            "import sys, wave\n"
            f"assert sys.argv[5:] == {([] if expected is None else [str(expected)])!r}",
        )
    )
    FinalPassRepository(store).request(identity, recording)
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, configuration).run_once()
    assert (
        store._db.execute("SELECT state FROM recording_final_passes").fetchone()[0] == "completed"
    )


@pytest.mark.parametrize("reverse", [False, True])
@pytest.mark.parametrize("zero_duration", [False, True])
async def test_manual_retranscription_merges_voices_and_keeps_the_larger_named_identity(
    environment, tmp_path, reverse, zero_duration
):
    _, store, _ = environment
    owner = device(store)
    identity = owner["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    app = app_for(environment, tmp_path)
    configuration = fake_separator(tmp_path)
    if reverse:
        payload = {
            "spans": [
                {"start_ms": 0, "end_ms": 10000, "voice": "A"},
                {"start_ms": 10000, "end_ms": 30300, "voice": "B"},
            ]
        }
        configuration.python.write_text(
            "#!/usr/bin/python3\n" + f"print({json.dumps(payload)!r})\n"
        )
    transcriber, _ = fake_whisper(zero_duration=zero_duration)
    worker = final_worker(store, transcriber, configuration)
    repo = FinalPassRepository(store)
    repo.request(identity, recording)
    assert await worker.run_once()
    # Alternate which voice has two words to catch "always keep voice_1".
    winner, loser = ("voice_2", "voice_1") if reverse else ("voice_1", "voice_2")
    repository.rename(
        store, identity, recording, {winner: "Ada", loser: "Bruno"}, {"voice_1", "voice_2"}, 1000
    )
    merged = {"spans": [{"start_ms": 0, "end_ms": 30300, "voice": "A"}]}
    configuration.python.write_text("#!/usr/bin/python3\n" + f"print({json.dumps(merged)!r})\n")
    # Nothing requeues itself just because the separator changed.
    assert not await worker.run_once()
    async with routes_only(app) as http:
        response = await http.post(f"/v1/recordings/{recording}/final-pass", headers=bearer(owner))
    assert response.status_code == 200
    assert await worker.run_once()
    with store.read_snapshot() as db:
        transcript = turns.read(store, db, identity, recording)
        changes = repository.speaker_changes(store, db, identity, recording)
    assert [(s.key, s.name) for s in transcript.speakers if s.track == "remote_app"] == [
        (winner, "Ada")
    ]
    assert changes["names"][loser] == "Bruno"
    assert repository.canonical_speaker_key(changes, loser) == winner


async def test_failed_reclustering_keeps_the_published_voices_text_and_names(environment, tmp_path):
    _, store, _ = environment
    identity = device(store)["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    transcriber, _ = fake_whisper()
    worker = final_worker(store, transcriber, fake_separator(tmp_path))
    repo = FinalPassRepository(store)
    repo.request(identity, recording)
    assert await worker.run_once()
    repository.rename(store, identity, recording, {"voice_1": "Ada"}, {"voice_1", "voice_2"}, 1000)
    with store.read_snapshot() as db:
        before = turns.read(store, db, identity, recording)
        voices = repo.voices({"device_id": identity, "recording": recording})
        names = repository.speaker_changes(store, db, identity, recording)
    worker.configuration = fake_separator(tmp_path, exit_status=42)
    repo.request(identity, recording)
    assert await worker.run_once()
    with store.read_snapshot() as db:
        assert turns.read(store, db, identity, recording) == before
        assert repo.voices({"device_id": identity, "recording": recording}) == voices
        assert repository.speaker_changes(store, db, identity, recording) == names
        assert db.execute("SELECT state FROM recording_final_passes").fetchone()[0] == "failed"


async def test_explicit_user_merge_survives_two_retranscriptions(environment, tmp_path):
    _, store, _ = environment
    identity = device(store)["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    transcriber, _ = fake_whisper()
    worker = final_worker(store, transcriber, fake_separator(tmp_path))
    repo = FinalPassRepository(store)
    repo.request(identity, recording)
    assert await worker.run_once()
    for key in ("voice_1", "voice_2"):
        repository.rename(store, identity, recording, {key: "Ada"}, {"voice_1", "voice_2"}, 1000)
    for _ in range(2):
        repo.request(identity, recording)
        assert await worker.run_once()
        with store.read_snapshot() as db:
            transcript = turns.read(store, db, identity, recording)
        assert [(s.key, s.name) for s in transcript.speakers if s.track == "remote_app"] == [
            ("voice_1", "Ada")
        ]


def test_older_database_does_not_requeue_completed_meetings_and_refresh_survives_restart(
    environment,
):
    from orbit_hub.storage import Store

    settings, store, _ = environment
    identity = device(store)["device_id"]
    recording = meeting(store, identity, texts=TEXTS)
    # A fixture from before the column existed. No production data is opened.
    with store.transaction() as db:
        db.execute("UPDATE recording_final_passes SET state='completed'")
        db.execute("ALTER TABLE recording_final_passes DROP COLUMN refresh_voices")
    with closing(Store(settings.data_dir, settings.key_file)) as reopened:
        repo = FinalPassRepository(reopened)
        reopened.recover_interrupted()
        assert repo.claim() is None
        assert repo.request(identity, recording)
        assert repo.claim()["refresh_voices"] == 1
        reopened.recover_interrupted()
        assert repo.claim()["refresh_voices"] == 1


async def test_count_only_acceptance_passes_the_bound_without_printing_private_content(
    tmp_path, monkeypatch, capsys
):
    from acceptance_diarization import test_real_local_diarization_boundary as count
    from support_audio import wav_bytes

    configuration = fake_separator(tmp_path)
    source = configuration.python.read_text()
    configuration.python.write_text(
        source.replace("import sys, wave", "import sys, wave\nassert sys.argv[5:] == ['4']")
    )
    audio = tmp_path / "fixture.wav"
    audio.write_bytes(wav_bytes(b"\x00\x01" * 484800))
    for key, value in {
        "ORBIT_DIARIZATION_PYTHON": configuration.python,
        "ORBIT_DIARIZATION_BUNDLE": configuration.bundle,
        "ORBIT_DIARIZATION_GPU_LOCK": configuration.gpu_lock,
        "ORBIT_DIARIZATION_WAV": audio,
        "ORBIT_DIARIZATION_MAX_SPEAKERS": 4,
    }.items():
        monkeypatch.setenv(key, str(value))
    await count()
    assert capsys.readouterr().out == "{'voices': 2}\n"
