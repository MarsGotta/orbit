"""Explicit full-pipeline acceptance; never discovered by the ordinary test suite.

Run from the repository with server/.venv/bin/python and these environment vars:
  ORBIT_FINAL_PASS_WAV: remote WAV (16-bit, mono/stereo, 8–192 kHz).
  ORBIT_FINAL_PASS_WHISPER_ENDPOINT: base URL of the real Whisper service.
  ORBIT_FINAL_PASS_CONFIG: TOML with [diarization] python, bundle, gpu_lock;
      optional [meeting_transcription] model (default openai/whisper-large-v3).
  ORBIT_FINAL_PASS_RTTM: optional reference, one recording/channel.
  ORBIT_FINAL_PASS_SCRIPT: optional UTF-8 plain transcript, without speaker labels.

Example: timeout 1800 server/.venv/bin/python server/tests/acceptance_final_pass.py
Explicit pytest execution also works: it launches this script in a child so the
ordinary suite's network fence stays intact. Neither entry point runs by default.

Output is JSON with numeric values only. Voice durations follow first appearance.
Speaker accuracy uses the reference voice with greatest word-time overlap, then
an optimal one-to-one label assignment; words outside reference speech count as
incorrect and are counted separately. It is not DER. WER uses case-folded Unicode
words without punctuation. References are read in memory. The actual hub stores
only encrypted audio/text in a private temporary DB, removed even on failure;
there are no plaintext copies or retained recordings or transcripts.
"""

import asyncio
import hashlib
import io
import json
import math
import os
import re
import subprocess
import sys
import time
import tomllib
import wave
from contextlib import ExitStack
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch
from uuid import uuid4

from orbit_hub import meeting_final_pass as final
from orbit_hub.config import Settings
from orbit_hub.meeting_diarization import DiarizationConfiguration
from orbit_hub.meeting_transcription import MeetingTranscriber
from orbit_hub.meetings import turns
from orbit_hub.protocol import HubError
from orbit_hub.storage import Store, initialize


def read_configuration(path):
    with path.open("rb") as handle:
        return tomllib.load(handle)


def seed_recording(root, source):
    data, key = root / "data", root / "key"
    initialize(data, key)
    store = Store(data, key)
    try:
        device = store.pair(store.create_code(1000), "Final pass acceptance", 1001)["device_id"]
        recording, sequence, frames = str(uuid4()), 0, 0
        with wave.open(str(source), "rb") as audio:
            channels, width, rate = audio.getnchannels(), audio.getsampwidth(), audio.getframerate()
            if channels not in (1, 2) or width != 2 or not 8000 <= rate <= 192000:
                raise ValueError("WAV format outside the capture contract")
            # Ten seconds match ordinary capture chunks. At 192 kHz stereo,
            # the upload ceiling can be tighter; account for its WAV header.
            chunk_frames = min(rate * 10, (Settings.max_chunk_bytes - 44) // (channels * width))
            while pcm := audio.readframes(chunk_frames):
                count = len(pcm) // (channels * width)
                buffer = io.BytesIO()
                with wave.open(buffer, "wb") as chunk:
                    chunk.setnchannels(channels)
                    chunk.setsampwidth(width)
                    chunk.setframerate(rate)
                    chunk.writeframes(pcm)
                body = buffer.getvalue()
                store.put_chunk(
                    device,
                    f"{recording}-{sequence}",
                    recording,
                    "remote_app",
                    sequence,
                    hashlib.sha256(body).hexdigest(),
                    body,
                    {
                        "frames": count,
                        "sample_rate": rate,
                        "channels": channels,
                        "first_sample": frames,
                        "started_at_ms": 1000 + frames * 1000 // rate,
                    },
                )
                frames += count
                sequence += 1
            if frames != audio.getnframes():
                raise ValueError("WAV truncated relative to its header")
        if not frames:
            raise ValueError("The WAV contains no frames")
        store.finalize_recording(
            device,
            recording,
            {
                "started_at_ms": 1000,
                "finalized_at_ms": 1000 + frames * 1000 // rate,
                "gaps": 0,
                "tracks": [
                    {
                        "track": "remote_app",
                        "chunk_count": sequence,
                        "last_sequence": sequence - 1,
                        "total_frames": frames,
                    }
                ],
            },
            final_pass=True,
        )
        return store, device, recording
    except BaseException:
        store.close()
        raise


def maximum_assignment(weights):
    """Maximum one-to-one agreement; Hungarian assignment, cubic rather than factorial."""
    size = max(len(weights), len(weights[0]) if weights else 0)
    if not size:
        return 0
    costs = [
        [-row[column] if column < len(row) else 0 for column in range(size)] for row in weights
    ]
    costs.extend([[0] * size for _ in range(size - len(costs))])
    row_potential, column_potential, matched, previous = ([0] * (size + 1) for _ in range(4))
    for row in range(1, size + 1):
        matched[0], column = row, 0
        minimum, visited = [math.inf] * (size + 1), [False] * (size + 1)
        while True:
            visited[column] = True
            active_row, delta, next_column = matched[column], math.inf, 0
            for candidate in range(1, size + 1):
                if visited[candidate]:
                    continue
                cost = (
                    costs[active_row - 1][candidate - 1]
                    - row_potential[active_row]
                    - column_potential[candidate]
                )
                if cost < minimum[candidate]:
                    minimum[candidate], previous[candidate] = cost, column
                if minimum[candidate] < delta:
                    delta, next_column = minimum[candidate], candidate
            for candidate in range(size + 1):
                if visited[candidate]:
                    row_potential[matched[candidate]] += delta
                    column_potential[candidate] -= delta
                else:
                    minimum[candidate] -= delta
            column = next_column
            if matched[column] == 0:
                break
        while column:
            before = previous[column]
            matched[column], column = matched[before], before
    return -sum(costs[matched[column] - 1][column - 1] for column in range(1, size + 1))


def read_rttm(path):
    spans, recordings = [], set()
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 10 or fields[0] != "SPEAKER":
            raise ValueError("RTTM requires SPEAKER rows of ten fields")
        start, duration = float(fields[3]), float(fields[4])
        if not math.isfinite(start) or not math.isfinite(duration) or start < 0 or duration <= 0:
            raise ValueError("Invalid RTTM interval")
        recordings.add((fields[1], fields[2]))
        spans.append(
            {
                "start_ms": round(start * 1000),
                "end_ms": round((start + duration) * 1000),
                "voice": fields[7],
            }
        )
    if not spans or len(recordings) != 1:
        raise ValueError("RTTM requires a single recording and channel with speech")
    return spans


def speaker_accuracy(words, reference):
    voices = sorted({w["voice"] for w in words if w["voice"] != "remote_app"})
    labels = sorted({span["voice"] for span in reference})
    weights = [[0] * len(labels) for _ in voices]
    without_reference = 0
    for word in words:
        overlaps = {}
        for span in reference:
            overlap = max(
                0, min(word["end_ms"], span["end_ms"]) - max(word["start_ms"], span["start_ms"])
            )
            overlaps[span["voice"]] = overlaps.get(span["voice"], 0) + overlap
        best = max(overlaps.values(), default=0)
        if not best:
            without_reference += 1
        elif word["voice"] in voices:
            row = weights[voices.index(word["voice"])]
            for label, overlap in overlaps.items():
                if overlap == best:
                    row[labels.index(label)] += 1
    correct = maximum_assignment(weights)
    result = {
        "speaker_correct_words": correct,
        "speaker_words": len(words),
        "words_without_reference": without_reference,
    }
    if words:
        result["speaker_accuracy_pct"] = correct * 100 / len(words)
    return result


def word_error_rate(reference, hypothesis):
    expected = re.findall(r"[^\W_]+", reference.casefold())
    heard = re.findall(r"[^\W_]+", hypothesis.casefold())
    if not expected:
        raise ValueError("The script contains no reference words")
    previous = list(range(len(heard) + 1))
    for index, word in enumerate(expected, 1):
        current = [index]
        for column, other in enumerate(heard, 1):
            current.append(
                min(current[-1] + 1, previous[column] + 1, previous[column - 1] + (word != other))
            )
        previous = current
    return {
        "reference_words": len(expected),
        "word_errors": previous[-1],
        "wer_pct": previous[-1] * 100 / len(expected),
    }


async def run_acceptance(environment):
    began, timings = time.perf_counter(), {}
    configuration = await asyncio.to_thread(
        read_configuration, Path(environment["ORBIT_FINAL_PASS_CONFIG"])
    )
    diarization = DiarizationConfiguration.model_validate(configuration["diarization"])
    transcriber = MeetingTranscriber(
        {
            "endpoint": environment["ORBIT_FINAL_PASS_WHISPER_ENDPOINT"],
            "model": configuration.get("meeting_transcription", {}).get(
                "model", "openai/whisper-large-v3"
            ),
            "language": "es",
        }
    )

    def measured(function, name):
        def call(*args, **kwargs):
            start = time.perf_counter()
            try:
                return function(*args, **kwargs)
            finally:
                timings[name] = timings.get(name, 0) + time.perf_counter() - start

        return call

    def measured_async(function, name):
        async def call(*args, **kwargs):
            start = time.perf_counter()
            try:
                return await function(*args, **kwargs)
            finally:
                timings[name] = timings.get(name, 0) + time.perf_counter() - start

        return call

    with TemporaryDirectory(prefix="orbit-final-acceptance-") as directory:
        store, device, recording = await asyncio.to_thread(
            measured(seed_recording, "import_seconds"),
            Path(directory),
            Path(environment["ORBIT_FINAL_PASS_WAV"]),
        )
        try:
            worker = final.FinalPassWorker(
                store,
                transcriber,
                diarization,
                asyncio.Event(),
                lambda device, recording: None,
                None,
            )
            with ExitStack() as patches:
                for name, label in (
                    ("prepare_track", "prepare_seconds"),
                    ("encode_window", "opus_seconds"),
                ):
                    patches.enter_context(
                        patch.object(final, name, measured(getattr(final, name), label))
                    )
                patches.enter_context(
                    patch.object(
                        final, "separate", measured_async(final.separate, "diarization_seconds")
                    )
                )
                patches.enter_context(
                    patch.object(
                        transcriber,
                        "transcribe",
                        measured_async(transcriber.transcribe, "whisper_seconds"),
                    )
                )
                patches.enter_context(
                    patch.object(
                        worker.repository,
                        "complete",
                        measured(worker.repository.complete, "persist_seconds"),
                    )
                )
                pipeline_start = time.perf_counter()
                await worker.run_once()
                timings["pipeline_seconds"] = time.perf_counter() - pipeline_start
            with store.read_snapshot() as db:
                state = final.status(db, device, recording)
                if state["state"] != "completed":
                    error = state["error"] or {}
                    raise HubError(
                        error.get("code", "final_pass_not_completed"),
                        error.get("phase", state["state"]),
                        numbers={k: v for k, v in error.items() if type(v) in (int, float)},
                    )
                row = db.execute("SELECT body FROM recording_final_transcripts").fetchone()
                transcript = json.loads(
                    store.decrypt(row["body"], f"recording-final-transcript:{device}:{recording}")
                )
                voices = worker.repository.voices({"device_id": device, "recording": recording})
                turn_count = len(turns.read(store, db, device, recording).turns)
            seconds = dict.fromkeys((span["voice"] for span in voices["spans"]), 0)
            for span in voices["spans"]:
                seconds[span["voice"]] += (span["end_ms"] - span["start_ms"]) / 1000
            result = {
                "voices": len(seconds),
                "voice_seconds": list(seconds.values()),
                "turns": turn_count,
                "audio_ms": transcript["audio_ms"],
            }
            if environment.get("ORBIT_FINAL_PASS_RTTM"):
                reference = await asyncio.to_thread(
                    read_rttm, Path(environment["ORBIT_FINAL_PASS_RTTM"])
                )
                result.update(speaker_accuracy(transcript["words"], reference))
            if environment.get("ORBIT_FINAL_PASS_SCRIPT"):
                script = await asyncio.to_thread(
                    Path(environment["ORBIT_FINAL_PASS_SCRIPT"]).read_text, encoding="utf-8"
                )
                result.update(
                    word_error_rate(script, " ".join(w["text"] for w in transcript["words"]))
                )
            timings["total_seconds"] = time.perf_counter() - began
            return {**result, **timings}
        finally:
            store.close()


def test_real_final_pass():
    # The ordinary suite's fence stays installed. Only this explicitly selected
    # manual acceptance launches the real pipeline in its own interpreter.
    with wave.open(os.environ["ORBIT_FINAL_PASS_WAV"], "rb") as audio:
        duration = audio.getnframes() / audio.getframerate()
    # Runner guard counts possible 15 s windows, preparation/diagnosis and
    # lock/model startup, with the existing per-operation budgets. One audio
    # duration covers importing/scoring; the worker's deadline is unchanged.
    budget = (
        duration * (final.CPU_SECONDS_PER_AUDIO_SECOND + 1)
        + (2 * math.ceil(duration / final.MIN_WINDOW_SECONDS) + 3) * final.REQUEST_TIMEOUT_SECONDS
    )
    completed = subprocess.run(
        [sys.executable, __file__], capture_output=True, text=True, check=False, timeout=budget
    )
    if completed.returncode:
        raise AssertionError(
            f"Final pass acceptance: the process exited with code {completed.returncode}"
        )
    print(completed.stdout.strip())


if __name__ == "__main__":
    try:
        print(json.dumps(asyncio.run(run_acceptance(os.environ)), allow_nan=False))
    except HubError as failure:
        print(
            json.dumps(
                {
                    "success": 0,
                    f"failure_{failure.code}_{failure.message}": 1,
                    **(failure.numbers or {}),
                },
                allow_nan=False,
            )
        )
        sys.exit(1)
    except Exception:  # noqa: BLE001 - redact private references and provider diagnostics
        # Input paths/references and arbitrary provider diagnostics stay private.
        print(json.dumps({"success": 0}))
        sys.exit(1)
