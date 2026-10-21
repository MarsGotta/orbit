"""Explicit real boundary check for the reviewer; excluded from normal test discovery.

Requires ORBIT_DIARIZATION_WAV (synthetic mono 16 kHz WAV) and
ORBIT_DIARIZATION_GPU_LOCK (the exact lock used by Whisper). Never run in CI.
Only counts and timings leave this check; no audio or transcript is saved.
"""

import asyncio
import os
import time
import wave
from pathlib import Path

from orbit_hub.meeting_diarization import DiarizationConfiguration, separate
from orbit_hub.meeting_final_pass import memory_file


async def test_real_local_diarization_boundary():
    configuration = DiarizationConfiguration(
        python=Path("/home/example/orbit-bundles/diarization-runtime/.venv/bin/python"),
        bundle=Path("/home/example/orbit-bundles/diarization-community-1"),
        gpu_lock=Path(os.environ["ORBIT_DIARIZATION_GPU_LOCK"]),
    )
    source = Path(os.environ["ORBIT_DIARIZATION_WAV"])
    with source.open("rb") as original, memory_file() as audio:  # noqa: ASYNC230 - explicit offline acceptance fixture
        while chunk := original.read(1024 * 1024):
            audio.write(chunk)
        audio.seek(0)
        with wave.open(audio, "rb") as wav:
            assert (wav.getnchannels(), wav.getsampwidth(), wav.getframerate()) == (1, 2, 16000)
            duration_ms = wav.getnframes() * 1000 // 16000
        began = time.monotonic()
        async with asyncio.timeout(duration_ms / 500 + 600):
            result = await separate(configuration, audio.fileno(), duration_ms)
    assert result["spans"] and set(result) == {"spans"}
    print(
        {
            "voices": len({span["voice"] for span in result["spans"]}),
            "audio_ms": duration_ms,
            "elapsed_seconds": round(time.monotonic() - began, 3),
        }
    )
