"""Exercise the manual acceptance with synthetic inputs and fake provider boundaries."""

import json
from pathlib import Path
from tempfile import TemporaryDirectory

import acceptance_final_pass as acceptance
import pytest
from acceptance_final_pass import maximum_assignment, speaker_accuracy, word_error_rate
from support_audio import wav_bytes
from test_final_pass import fake_separator, fake_whisper

from orbit_hub.protocol import HubError


def acceptance_inputs(tmp_path, monkeypatch, *, references=True, failure=False):
    audio = tmp_path / "synthetic.wav"
    audio.write_bytes(wav_bytes(b"\x00\x01" * 484800))
    configuration = fake_separator(tmp_path)
    config = tmp_path / "fixture.toml"
    config.write_text(
        f'[diarization]\npython = "{configuration.python}"\n'
        f'bundle = "{configuration.bundle}"\ngpu_lock = "{configuration.gpu_lock}"\n'
    )
    transcriber, requests = fake_whisper(status=502 if failure else 200)
    monkeypatch.setattr(acceptance, "MeetingTranscriber", lambda configuration: transcriber)
    directories = []

    def temporary(**kwargs):
        directory = TemporaryDirectory(dir=tmp_path, **kwargs)
        directories.append(Path(directory.name))
        return directory

    monkeypatch.setattr(acceptance, "TemporaryDirectory", temporary)
    environment = {
        "ORBIT_FINAL_PASS_WAV": str(audio),
        "ORBIT_FINAL_PASS_CONFIG": str(config),
        "ORBIT_FINAL_PASS_WHISPER_ENDPOINT": "http://localhost:0",
    }
    if references:
        rttm, script = tmp_path / "truth.rttm", tmp_path / "script.txt"
        rttm.write_text(
            "SPEAKER fixture 1 0 10 <NA> <NA> X <NA> <NA>\n"
            "SPEAKER fixture 1 10 10 <NA> <NA> Y <NA> <NA>\n"
            "SPEAKER fixture 1 20 10 <NA> <NA> X <NA> <NA>\n"
        )
        script.write_text("Uno. Dos. Tres.")
        environment.update(ORBIT_FINAL_PASS_RTTM=str(rttm), ORBIT_FINAL_PASS_SCRIPT=str(script))
    return environment, directories, requests


@pytest.mark.parametrize("references", [True, False])
async def test_full_acceptance_runs_the_real_worker_reports_only_numbers_and_removes_its_db(
    tmp_path, monkeypatch, references
):
    environment, directories, requests = acceptance_inputs(
        tmp_path, monkeypatch, references=references
    )
    result = await acceptance.run_acceptance(environment)
    assert result["voices"] == 2
    assert result["voice_seconds"] == [20.0, 10.0]
    assert result["turns"] == 3 and result["audio_ms"] == 30300
    assert len(requests) == 1
    for field in (
        "import_seconds",
        "prepare_seconds",
        "opus_seconds",
        "whisper_seconds",
        "diarization_seconds",
        "persist_seconds",
        "pipeline_seconds",
        "total_seconds",
    ):
        assert result[field] >= 0
    if references:
        assert result["speaker_accuracy_pct"] == 100 and result["speaker_correct_words"] == 3
        assert result["words_without_reference"] == 0
        assert result["wer_pct"] == 0 and result["reference_words"] == 3
    else:
        assert "speaker_accuracy_pct" not in result and "wer_pct" not in result
    assert all(
        type(value) in (int, float)
        or (isinstance(value, list) and all(type(item) in (int, float) for item in value))
        for value in result.values()
    )
    assert "Uno" not in json.dumps(result)
    assert directories and all(not directory.exists() for directory in directories)


async def test_full_acceptance_removes_its_database_even_when_whisper_fails(tmp_path, monkeypatch):
    environment, directories, _ = acceptance_inputs(tmp_path, monkeypatch, failure=True)
    with pytest.raises(HubError) as failure:
        await acceptance.run_acceptance(environment)
    assert failure.value.code == "transcription_http_error"
    assert failure.value.message == "whisper_remote_app"
    assert failure.value.numbers == {"provider_status": 502}
    assert directories and all(not directory.exists() for directory in directories)


@pytest.mark.parametrize(
    "weights,expected",
    [
        ([[8, 7], [7, 0]], 14),
        ([[0, 4, 0], [3, 0, 0], [0, 0, 2]], 9),
        ([[3, 4, 2]], 4),
        ([[2], [3], [4]], 4),
        ([], 0),
    ],
)
def test_voice_label_assignment_is_optimal_including_rectangular_and_empty_cases(weights, expected):
    assert maximum_assignment(weights) == expected


def test_word_accuracy_penalizes_merged_and_unknown_voices_and_uncovered_words():
    reference = [
        {"start_ms": 0, "end_ms": 1000, "voice": "A"},
        {"start_ms": 1000, "end_ms": 2000, "voice": "B"},
    ]
    words = [
        {"start_ms": 0, "end_ms": 500, "voice": "voice_1"},
        {"start_ms": 1100, "end_ms": 1500, "voice": "voice_1"},
        {"start_ms": 1600, "end_ms": 1800, "voice": "remote_app"},
        {"start_ms": 2100, "end_ms": 2200, "voice": "voice_2"},
    ]
    assert speaker_accuracy(words, reference) == {
        "speaker_correct_words": 1,
        "speaker_words": 4,
        "words_without_reference": 1,
        "speaker_accuracy_pct": 25.0,
    }


def test_wer_counts_insertions_deletions_and_substitutions_after_normalization():
    assert word_error_rate("¡Uno, DOS! Tres cuatro", "uno extra dos cinco") == {
        "reference_words": 4,
        "word_errors": 3,
        "wer_pct": 75.0,
    }
    with pytest.raises(ValueError, match="no contiene palabras"):
        word_error_rate("...", "algo")


async def test_acceptance_cli_runs_with_its_own_fake_whisper_and_retains_only_numbers(
    tmp_path, monkeypatch
):
    import asyncio
    import os
    import sys

    from fastapi import FastAPI, Request
    from support_client_e2e import tcp_server

    environment, _, _ = acceptance_inputs(tmp_path, monkeypatch)
    app = FastAPI()

    @app.post("/v1/audio/transcriptions")
    async def whisper(request: Request):
        body = await request.body()
        assert b'name="vad_filter"\r\n\r\ntrue' in body and b"OggS" in body
        return {
            "text": "Uno. Dos. Tres.",
            "language": "es",
            "words": [
                {"start": 1, "end": 2, "word": "Uno."},
                {"start": 11, "end": 12, "word": "Dos."},
                {"start": 21, "end": 22, "word": "Tres."},
            ],
        }

    async with tcp_server(app) as endpoint:
        environment["ORBIT_FINAL_PASS_WHISPER_ENDPOINT"] = endpoint
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            acceptance.__file__,
            env={**os.environ, **environment, "TMPDIR": str(tmp_path)},
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout, stderr = await asyncio.wait_for(process.communicate(), 60)
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()
    assert process.returncode == 0, stderr.decode()
    result = json.loads(stdout)
    assert result["voices"] == 2 and result["turns"] == 3
    assert result["wer_pct"] == 0 and result["speaker_accuracy_pct"] == 100
    assert "Uno" not in stdout.decode()
    assert not list(tmp_path.glob("orbit-final-acceptance-*"))
