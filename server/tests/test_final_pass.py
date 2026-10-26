"""Final meeting pass over synthetic audio, fake Whisper and a fake process only."""

import json

import httpx
import pytest
from conftest import device
from support_app import client
from support_notes import note_answer
from test_finalize import recording_fixture
from test_meeting_views import TEXTS, meeting

from orbit_hub.meeting_transcription import MeetingTranscriber
from orbit_hub.notes.generation import build_note
from orbit_hub.notes.models import NoteInputs
from orbit_hub.notes.templates import builtin


def test_closing_a_meeting_queues_a_durable_final_pass(environment):
    _, store, _ = environment
    owner = device(store)
    recording, manifest = recording_fixture(store, owner["device_id"])
    with client(environment) as http:
        headers = {"Authorization": "Bearer " + owner["token"]}
        assert (
            http.post(
                f"/v1/recordings/{recording}/finalize", headers=headers, json=manifest
            ).status_code
            == 200
        )
        document = http.get(f"/v1/recordings/{recording}", headers=headers).json()
        assert document["transcript"].get("final_pass") is not None
    row = store._db.execute("SELECT state FROM recording_final_passes").fetchone()
    assert row["state"] in {"queued", "running", "failed", "completed"}


def test_renaming_one_turn_leaves_the_other_turns_of_that_voice(environment):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        headers = {"Authorization": "Bearer " + owner["token"]}
        path = f"/v1/recordings/{recording}"
        response = http.put(
            path + "/speakers",
            headers=headers,
            json={
                "names": {"remote_app": "Lucía"},
                "turn": {"at_ms": 300, "speaker_key": "remote_app"},
            },
        )
        assert response.status_code == 200, response.text
        page = http.get(path + "/turns", headers=headers).json()
        assert page["turns"][1]["speaker_key"] != "remote_app"
        assert page["turns"][1]["at_ms"] == 300
        assert page["turns"][1]["text"] == "Gracias, empezamos. Yo envío el acta."


async def test_whisper_word_times_are_read_without_losing_the_existing_text():
    async def answer(request):
        return httpx.Response(
            200,
            json={
                "text": "Hola sí",
                "language": "es",
                "segments": [{"start": 0, "end": 2, "text": "Hola sí"}],
                "words": [
                    {"start": 0, "end": 1, "word": "Hola"},
                    {"start": 1, "end": 2, "word": " sí"},
                ],
            },
        )

    transcriber = MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(answer)),
    )
    result = await transcriber.transcribe(b"synthetic")
    assert result.text == "Hola sí"
    assert getattr(result, "words", None) == (
        {"start_ms": 0, "end_ms": 1000, "text": "Hola"},
        {"start_ms": 1000, "end_ms": 2000, "text": " sí"},
    )


def test_an_unnamed_numbered_voice_is_never_a_task_owner():
    answer = note_answer()
    answer["action_items"][0]["owner"] = "Persona 2"
    note = build_note(
        answer,
        builtin("general"),
        NoteInputs(),
        model="fake",
        generated_at_ms=0,
        segments=1,
        duration_ms=2000,
        parts=1,
    )
    assert note.action_items[0].owner is None


# This executable replaces the pyannote runtime at the actual process boundary.
# It reads only the synthetic memfd it inherits; it cannot load a model or GPU.
def fake_separator(tmp_path, *, exit_status=0, malformed=False):
    from orbit_hub.meeting_diarization import DiarizationConfiguration

    executable = tmp_path / "fake-python"
    payload = {
        "spans": [
            {"start_ms": 0, "end_ms": 10_000, "voice": "A"},
            {"start_ms": 10_000, "end_ms": 20_000, "voice": "B"},
            {"start_ms": 20_000, "end_ms": 30_000, "voice": "A"},
        ],
    }
    executable.write_text(
        "#!/usr/bin/python3\nimport sys, wave\n"
        "with wave.open(sys.argv[2], 'rb') as wav:\n"
        " assert wav.getframerate() == 16000\n"
        " assert wav.getnchannels() == 1\n"
        " assert wav.getnframes() == 484800\n"
        f"print({('broken' if malformed else json.dumps(payload))!r})\n"
        f"sys.exit({exit_status})\n"
    )
    executable.chmod(0o700)
    return DiarizationConfiguration(
        python=executable, bundle=tmp_path / "unused-bundle", gpu_lock=tmp_path / "unused-lock"
    )


def fake_whisper(*, missing_words=False, status=200):
    requests = []

    async def answer(request):
        requests.append(request)
        assert b"OggS" in request.content
        assert b'name="timestamp_granularities[]"' in request.content
        assert b'name="vad_filter"' in request.content
        # The service takes no VAD parameters: none are sent.
        assert b'name="vad_parameters"' not in request.content
        assert b'name="prompt"' not in request.content
        words = [
            {"start": 1, "end": 2, "word": "Uno."},
            {"start": 11, "end": 12, "word": "Dos."},
            {"start": 21, "end": 22, "word": "Tres."},
        ]
        return httpx.Response(
            status,
            json={
                "text": "Uno. Dos. Tres.",
                "language": "es",
                "words": [] if missing_words else words,
            },
        )

    return MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(answer)),
    ), requests


def final_worker(store, transcriber, configuration):
    import asyncio

    from orbit_hub.meeting_final_pass import FinalPassWorker

    return FinalPassWorker(
        store, transcriber, configuration, asyncio.Event(), lambda device, recording: None, None
    )


async def test_final_words_split_voices_and_all_readers_use_the_same_names(environment, tmp_path):
    from support_notes import write_note

    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import repository, turns
    from orbit_hub.meetings_bridge.service import MeetingsBridgeService

    _, store, _ = environment
    owner = device(store)
    identity = meeting(store, owner["device_id"], texts=TEXTS)
    transcriber, requests = fake_whisper()
    repo = FinalPassRepository(store)
    repo.request(owner["device_id"], identity)
    worker = final_worker(store, transcriber, fake_separator(tmp_path))
    assert await worker.run_once()
    with store.read_snapshot() as db:
        transcript = turns.read(store, db, owner["device_id"], identity)
        state = db.execute("SELECT state,error FROM recording_final_passes").fetchone()
        assert dict(state) == {"state": "completed", "error": None}
        blob = db.execute("SELECT body FROM recording_voices").fetchone()[0]
        assert b"Uno." not in blob and b"embeddings" not in blob
        assert json.loads(
            store.decrypt(blob, f"recording-voices:{owner['device_id']}:{identity}")
        ) == {
            "spans": [
                {"start_ms": 0, "end_ms": 10000, "voice": "voice_1"},
                {"start_ms": 10000, "end_ms": 20000, "voice": "voice_2"},
                {"start_ms": 20000, "end_ms": 30000, "voice": "voice_1"},
            ]
        }
    remote = [s for s in transcript.segments if s.track == "remote_app"]
    assert [(s.at_ms, s.end_ms, s.speaker_key, s.text) for s in remote] == [
        (1000, 2000, "voice_1", "Uno."),
        (11000, 12000, "voice_2", "Dos."),
        (21000, 22000, "voice_1", "Tres."),
    ]
    assert [(s.key, s.name) for s in transcript.speakers if s.key.startswith("voice_")] == [
        ("voice_1", "Persona 1"),
        ("voice_2", "Persona 2"),
    ]
    assert len(requests) == 2
    with client(environment) as http:
        headers = {"Authorization": "Bearer " + owner["token"]}
        path = f"/v1/recordings/{identity}"
        assert (
            http.put(
                path + "/speakers", headers=headers, json={"names": {"voice_1": "Lucía"}}
            ).status_code
            == 200
        )
        assert (
            http.put(
                path + "/speakers", headers=headers, json={"names": {"voice_2": "Pedro"}}
            ).status_code
            == 200
        )
        # One later turn changes without altering either voice's earlier turns.
        assert (
            http.put(
                path + "/speakers",
                headers=headers,
                json={
                    "names": {"voice_1": "Pedro"},
                    "turn": {"at_ms": 21000, "speaker_key": "voice_1"},
                },
            ).status_code
            == 200
        )
        page = http.get(path + "/turns", headers=headers).json()
        remote_turns = [t for t in page["turns"] if t["speaker_key"] != "mic_local"]
        # Naming the last word as the preceding voice joins their otherwise
        # unoccupied pause. All three words retain their chosen voice.
        assert [(t["at_ms"], t["speaker_key"], t["text"]) for t in remote_turns] == [
            (1000, "voice_1", "Uno."),
            (11000, "voice_2", "Dos. Tres."),
        ]
        stale = http.put(
            path + "/speakers",
            headers=headers,
            json={
                "names": {"voice_1": "Lucía"},
                "turn": {"at_ms": 21000, "speaker_key": "voice_1"},
            },
        )
        assert stale.status_code == 409
        assert stale.json()["code"] == "turn_no_longer_exists"
        assert stale.json()["refusal"]["at_ms"] == 21000
    note_worker = await write_note(store, owner["device_id"], identity, owner_name="Alina")
    prompt = note_worker.client.calls[0]["messages"][-1]["content"]
    with store.read_snapshot() as db:
        after = turns.read(store, db, owner["device_id"], identity)
        changes = repository.speaker_changes(store, db, owner["device_id"], identity)
    assert changes["turns"] == [
        {"track": "remote_app", "start_ms": 21000, "end_ms": 22000, "speaker_key": "voice_2"}
    ]
    assert [(s.text, s.speaker_key) for s in after.segments if s.track == "remote_app"] == [
        ("Uno.", "voice_1"),
        ("Dos.", "voice_2"),
        ("Tres.", "voice_2"),
    ]
    text = MeetingsBridgeService._render(after, "text")
    assert text in prompt
    segments = [
        json.loads(line) for line in MeetingsBridgeService._render(after, "segments").splitlines()
    ]
    assert [
        (s["at_ms"], s["speaker_key"], s["speaker"], s["text"])
        for s in segments
        if s["track"] == "remote_app"
    ] == [(1000, "voice_1", "Lucía", "Uno."), (11000, "voice_2", "Pedro", "Dos. Tres.")]
    # A manual repeat transcribes again, reusing the successful voice intervals.
    repo.request(owner["device_id"], identity)
    assert await worker.run_once()
    assert len(requests) == 4
    assert store._db.execute("SELECT body FROM recording_voices").fetchone()[0] == blob


@pytest.mark.parametrize(
    "failure,code,phase",
    [
        ("exit", "diarization_process_failed", "diarization"),
        ("json", "diarization_invalid_response", "diarization"),
        ("words", "transcription_words_missing", "whisper_mic_local"),
        ("http", "transcription_http_error", "whisper_mic_local"),
    ],
)
async def test_a_failed_pass_is_atomic_and_the_note_uses_the_draft_with_its_cause(
    environment, tmp_path, failure, code, phase
):
    from support_notes import write_note

    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.notes import NoteRepository

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    notes = NoteRepository(store)
    notes.initialize()
    FinalPassRepository(store).request(owner, identity)
    notes.request(owner, identity, 1000, True)
    assert notes.claim(1000) is None  # The final job, not the clock, controls readiness.
    transcriber, _ = fake_whisper(
        missing_words=failure == "words", status=502 if failure == "http" else 200
    )
    worker = final_worker(
        store,
        transcriber,
        fake_separator(
            tmp_path, exit_status=17 if failure == "exit" else 0, malformed=failure == "json"
        ),
    )
    assert await worker.run_once()
    assert store._db.execute("SELECT count(*) FROM recording_voices").fetchone()[0] == 0
    stored = json.loads(store._db.execute("SELECT error FROM recording_final_passes").fetchone()[0])
    assert stored["code"] == code and stored["phase"] == phase
    if failure == "exit":
        assert stored["exit_status"] == 17
    if failure == "words":
        assert stored["words"] == 0 and stored["characters"] == 15
    worker = await write_note(store, owner, identity)
    document = worker.repository.document(owner, identity)
    assert document["state"] == "ready"
    assert document["note"]["final_pass_error"] == stored
    assert "Otra persona: Gracias, empezamos." in worker.client.calls[0]["messages"][-1]["content"]


def test_queue_recovery_and_old_recordings_are_not_backfilled(environment):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    _, store, _ = environment
    owner = device(store)["device_id"]
    old = meeting(store, owner, texts=TEXTS)
    assert store._db.execute("SELECT count(*) FROM recording_final_passes").fetchone()[0] == 0
    repo = FinalPassRepository(store)
    repo.request(owner, old)
    job = repo.claim()
    assert job["recording"] == old
    assert repo.claim() is None
    store.recover_interrupted()
    assert repo.claim() == job


def test_every_separator_voice_survives_even_a_one_millisecond_guest():
    from orbit_hub.meeting_diarization import normalize_voices

    result = normalize_voices(
        {
            "spans": [
                {"start_ms": 0, "end_ms": 6000, "voice": "stray"},
                {"start_ms": 6000, "end_ms": 16000, "voice": "main"},
                {"start_ms": 16000, "end_ms": 16001, "voice": "guest"},
            ],
        },
        17000,
    )
    assert [s["voice"] for s in result["spans"]] == ["voice_1", "voice_2", "voice_3"]
    assert set(result) == {"spans"}


@pytest.mark.parametrize("start,end", [(0, 0), (100, 200), (749, 749), (2251, 2251), (3000, 4000)])
def test_remote_words_far_from_every_voice_are_discarded(start, end):
    from orbit_hub.meeting_final_pass import attribute

    word = {"start_ms": start, "end_ms": end, "text": "Inventado."}
    spans = [{"start_ms": 1000, "end_ms": 2000, "voice": "voice_1"}]
    assert attribute([word], spans, "remote_app") == []


@pytest.mark.parametrize(
    "start,end", [(750, 750), (700, 750), (1500, 1500), (1900, 2100), (2250, 2250)]
)
def test_remote_words_within_250_ms_of_a_voice_are_kept_and_attributed(start, end):
    from orbit_hub.meeting_final_pass import attribute

    word = {"start_ms": start, "end_ms": end, "text": "Cerca."}
    spans = [{"start_ms": 1000, "end_ms": 2000, "voice": "voice_1"}]
    assert attribute([word], spans, "remote_app") == [
        {**word, "track": "remote_app", "voice": "voice_1"}
    ]


def test_local_words_do_not_need_remote_speech_and_empty_remote_speech_drops_words():
    from orbit_hub.meeting_final_pass import attribute

    word = {"start_ms": 0, "end_ms": 0, "text": "Local."}
    assert attribute([word], [], "mic_local") == [
        {**word, "track": "mic_local", "voice": "mic_local"}
    ]
    assert attribute([word], [], "remote_app") == []


@pytest.mark.parametrize("at,voice", [(2100, "voice_1"), (2900, "voice_2")])
def test_a_word_in_a_gap_uses_the_nearest_voice(at, voice):
    from orbit_hub.meeting_final_pass import attribute

    word = {"start_ms": at, "end_ms": at, "text": "Borde."}
    spans = [
        {"start_ms": 1000, "end_ms": 2000, "voice": "voice_1"},
        {"start_ms": 3000, "end_ms": 4000, "voice": "voice_2"},
    ]
    assert attribute([word], spans, "remote_app") == [
        {**word, "track": "remote_app", "voice": voice}
    ]


def test_overlapping_word_uses_the_voice_with_most_overlap():
    from orbit_hub.meeting_final_pass import attribute

    word = {"start_ms": 1900, "end_ms": 2300, "text": "Cambio."}
    spans = [
        {"start_ms": 1000, "end_ms": 2000, "voice": "voice_1"},
        {"start_ms": 2000, "end_ms": 3000, "voice": "voice_2"},
    ]
    assert attribute([word], spans, "remote_app") == [
        {**word, "track": "remote_app", "voice": "voice_2"}
    ]


async def test_word_window_offsets_and_actual_opus_encoding(environment, monkeypatch):
    from orbit_hub import meeting_final_pass as final

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    final.FinalPassRepository(store).request(owner, identity)
    calls = []

    async def answer(request):
        calls.append(request)
        assert b"OggS" in request.content
        if len(calls) == 7:  # The remote track has a final 300 ms of audio.
            return httpx.Response(200, json={"text": "", "words": []})
        return httpx.Response(
            200, json={"text": "Hola.", "words": [{"start": 1, "end": 2, "word": "Hola."}]}
        )

    transcriber = MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(answer)),
    )

    async def remote_voice(*args):
        return {"spans": [{"start_ms": 0, "end_ms": 30300, "voice": "voice_1"}]}

    monkeypatch.setattr(final, "WINDOW_SECONDS", 10)
    monkeypatch.setattr(final, "separate", remote_voice)
    worker = final_worker(store, transcriber, object())
    assert await worker.run_once()
    with store.read_snapshot() as db:
        row = db.execute("SELECT body FROM recording_final_transcripts").fetchone()
        assert row is not None
        payload = json.loads(
            store.decrypt(row["body"], f"recording-final-transcript:{owner}:{identity}")
        )
    assert [(w["start_ms"], w["end_ms"]) for w in payload["words"]] == [
        (1000, 2000),
        (11000, 12000),
        (21000, 22000),
    ] * 2
    assert [w["voice"] for w in payload["words"]] == ["mic_local"] * 3 + ["voice_1"] * 3


async def test_a_language_change_retranscribes_text_without_changing_voice_identity(
    environment, tmp_path
):
    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import repository, turns

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    transcriber, requests = fake_whisper()
    FinalPassRepository(store).request(owner, identity)
    configuration = fake_separator(tmp_path)
    worker = final_worker(store, transcriber, configuration)
    assert await worker.run_once()
    blob = store._db.execute("SELECT body FROM recording_voices").fetchone()[0]
    # If the model were invoked again, this missing runtime would fail the job.
    configuration.python.unlink()
    repository.change_language(store, owner, identity, "en", "es", 2000)
    assert await worker.run_once()
    assert b'name="language"\r\n\r\nen' in requests[-1].content
    assert store._db.execute("SELECT body FROM recording_voices").fetchone()[0] == blob
    assert (
        store._db.execute("SELECT state FROM recording_final_passes").fetchone()[0] == "completed"
    )
    with store.read_snapshot() as db:
        assert turns.read(store, db, owner, identity).language == "en"


async def test_a_stale_language_result_cannot_complete_the_replacement_job(environment):
    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import repository

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    repo = FinalPassRepository(store)
    repo.request(owner, identity)
    job = repo.claim()
    job["language"] = "es"
    repository.change_language(store, owner, identity, "en", "es", 2000)
    repo.complete(job, {"spans": [], "words": [], "audio_ms": 1000, "language": "es"})
    assert store._db.execute("SELECT state FROM recording_final_passes").fetchone()[0] == "queued"
    assert store._db.execute("SELECT count(*) FROM recording_voices").fetchone()[0] == 0


async def test_diarization_deadline_records_phase_and_budget(environment, monkeypatch):
    from orbit_hub import meeting_final_pass as final

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    final.FinalPassRepository(store).request(owner, identity)
    transcriber, _ = fake_whisper()

    async def expired(*args):
        raise TimeoutError

    monkeypatch.setattr(final, "separate", expired)
    assert await final_worker(store, transcriber, object()).run_once()
    error = json.loads(store._db.execute("SELECT error FROM recording_final_passes").fetchone()[0])
    assert error == {"code": "final_pass_timeout", "phase": "diarization", "timeout_ms": 926069}


async def test_cancelling_the_separator_kills_and_reaps_its_process(tmp_path, monkeypatch):
    import asyncio

    from orbit_hub import meeting_diarization as separation
    from orbit_hub.meeting_final_pass import memory_file

    configuration = fake_separator(tmp_path)
    configuration.python.write_text("#!/usr/bin/python3\nimport signal\nsignal.pause()\n")
    real_spawn = asyncio.create_subprocess_exec
    spawned, child = asyncio.Event(), []

    async def spawn(*args, **kwargs):
        process = await real_spawn(*args, **kwargs)
        child.append(process)
        spawned.set()
        return process

    monkeypatch.setattr(separation.asyncio, "create_subprocess_exec", spawn)
    with memory_file() as audio:
        task = asyncio.create_task(separation.separate(configuration, audio.fileno(), 30000))
        await spawned.wait()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
    assert child[0].returncode is not None


def test_the_runtime_script_uses_offline_local_model_batch_four_and_cpu_fallback(
    tmp_path, monkeypatch, capsys
):
    import sys
    from types import SimpleNamespace

    from orbit_hub import diarization_process

    annotation = SimpleNamespace(
        itertracks=lambda **kwargs: [(SimpleNamespace(start=0, end=1), 0, "A")]
    )
    output = SimpleNamespace(
        exclusive_speaker_diarization=annotation,
    )

    class Pipeline:
        @staticmethod
        def from_pretrained(path):
            assert path == str(tmp_path / "bundle" / "config.yaml")
            return Pipeline()

        def __call__(self, path):
            assert self.segmentation_batch_size == self.embedding_batch_size == 4
            assert path == "synthetic.wav" and self.device == "cpu"
            return output

        def to(self, device):
            self.device = device

    class Specifications:
        pass

    class Problem:
        pass

    class Resolution:
        pass

    allowed = []
    monkeypatch.setitem(
        sys.modules,
        "pyannote.audio.core.task",
        SimpleNamespace(Specifications=Specifications, Problem=Problem, Resolution=Resolution),
    )
    monkeypatch.setitem(
        sys.modules,
        "torch",
        SimpleNamespace(
            cuda=SimpleNamespace(is_available=lambda: False),
            device=lambda value: value,
            serialization=SimpleNamespace(add_safe_globals=allowed.extend),
        ),
    )
    monkeypatch.setitem(sys.modules, "pyannote.audio", SimpleNamespace(Pipeline=Pipeline))
    monkeypatch.setattr(
        sys, "argv", ["script", "synthetic.wav", str(tmp_path / "bundle"), str(tmp_path / "lock")]
    )
    diarization_process.main()
    assert allowed == [Specifications, Problem, Resolution]
    assert json.loads(capsys.readouterr().out) == {
        "spans": [{"start_ms": 0, "end_ms": 1000, "voice": "A"}],
    }


async def test_the_final_result_replaces_even_a_failed_or_pending_draft(environment, tmp_path):
    from support_notes import write_note

    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import views
    from orbit_hub.notes import NoteRepository

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    with store.transaction() as db:
        db.execute(
            "UPDATE transcription_groups SET state='failed',error_code='transcription_http_502'"
        )
    repo = FinalPassRepository(store)
    repo.request(owner, identity)
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, fake_separator(tmp_path)).run_once()
    notes = NoteRepository(store)
    notes.initialize()
    with store.read_snapshot() as db:
        document = views.document(store, notes, db, owner, identity, default_language="es")
    assert document["transcript"] == {
        "state": "ready",
        "pending_chunks": 0,
        "failed_groups": [],
        "final_pass": {"state": "completed", "error": None},
    }
    note_worker = await write_note(store, owner, identity)
    assert "Gracias, empezamos." not in note_worker.client.calls[0]["messages"][-1]["content"]
    assert "Persona 1: Uno." in note_worker.client.calls[0]["messages"][-1]["content"]


async def test_merging_whole_voices_preserves_immutable_intervals_and_overrides(
    environment, tmp_path
):
    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meetings import repository, turns

    _, store, _ = environment
    owner = device(store)["device_id"]
    identity = meeting(store, owner, texts=TEXTS)
    FinalPassRepository(store).request(owner, identity)
    transcriber, _ = fake_whisper()
    assert await final_worker(store, transcriber, fake_separator(tmp_path)).run_once()
    voices = store._db.execute("SELECT body FROM recording_voices").fetchone()[0]
    repository.rename(store, owner, identity, {"voice_1": "Lucía"}, {"voice_1", "voice_2"}, 2000)
    repository.rename(store, owner, identity, {"voice_2": "Lucía"}, {"voice_1", "voice_2"}, 2000)
    with store.read_snapshot() as db:
        transcript = turns.read(store, db, owner, identity)
        changes = repository.speaker_changes(store, db, owner, identity)
    assert changes["same_as"] == {"voice_2": "voice_1"}
    assert {s.speaker_key for s in transcript.segments if s.track == "remote_app"} == {"voice_1"}
    assert [s.name for s in transcript.speakers if s.track == "remote_app"] == ["Lucía"]
    assert store._db.execute("SELECT body FROM recording_voices").fetchone()[0] == voices


async def test_http_close_runs_the_durable_pass_before_the_note(environment, tmp_path, monkeypatch):
    import asyncio
    from dataclasses import replace

    from support_notes import FakeModel
    from support_waits import wait_for
    from test_meeting_views import manifest_body

    from orbit_hub.app import create_app

    settings, store, adapter = environment
    owner = device(store)
    identity = meeting(store, owner["device_id"], texts=TEXTS, finalize=False)
    transcriber, requests = fake_whisper()
    monkeypatch.setattr(
        MeetingTranscriber, "_default_client", lambda self: transcriber._client_factory()
    )
    model = FakeModel()
    app = create_app(
        replace(settings, diarization=fake_separator(tmp_path)),
        store=store,
        assistant=adapter,
        speech=adapter,
        processing_client=model,
    )
    ready = asyncio.Event()
    original = app.state.notes.recording_changed

    def changed(device, recording):
        original(device, recording)
        with store.read_snapshot() as db:
            row = db.execute(
                "SELECT state FROM recording_notes WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if row is not None and row["state"] == "ready":
                ready.set()

    app.state.notes.recording_changed = changed
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        headers = {"Authorization": "Bearer " + owner["token"]}
        response = await http.post(
            f"/v1/recordings/{identity}/finalize", headers=headers, json=manifest_body()
        )
        assert response.status_code == 200, response.text
        await wait_for(ready.wait(), "la nota escrita después de la pasada final")
        document = (await http.get(f"/v1/recordings/{identity}", headers=headers)).json()
        assert document["transcript"]["final_pass"] == {"state": "completed", "error": None}
        assert document["note"]["state"] == "ready"
        assert len(model.calls) == 1
        assert "Persona 1: Uno." in model.calls[0]["messages"][-1]["content"]
        assert sum(b'name="timestamp_granularities[]"' in r.content for r in requests) == 2


def test_explicit_final_pass_is_device_scoped_and_requires_closed_audio(environment):
    _, store, _ = environment
    owner, other = device(store), device(store)
    identity = meeting(store, owner["device_id"], texts=TEXTS, finalize=False)
    with client(environment) as http:
        path = f"/v1/recordings/{identity}/final-pass"
        assert (
            http.post(path, headers={"Authorization": "Bearer " + other["token"]}).status_code
            == 404
        )
        response = http.post(path, headers={"Authorization": "Bearer " + owner["token"]})
        assert response.status_code == 400 and response.json()["code"] == "recording_still_open"
    assert store._db.execute("SELECT count(*) FROM recording_final_passes").fetchone()[0] == 0


async def test_the_final_pass_enables_vad_after_the_real_draft_comparison():
    async def answer(request):
        assert b'name="vad_filter"\r\n\r\ntrue' in request.content
        return httpx.Response(
            200, json={"text": "Hola.", "words": [{"start": 0, "end": 1, "word": "Hola."}]}
        )

    transcriber = MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(answer)),
    )
    await transcriber.transcribe(b"synthetic", final=True)
