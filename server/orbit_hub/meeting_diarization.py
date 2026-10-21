"""Local, offline pyannote subprocess. No model is imported by the hub."""

import asyncio
import json
import os
import signal
from contextlib import suppress
from pathlib import Path

from pydantic import BaseModel, ConfigDict

from .protocol import HubError

# JSON, not audio: 16 MiB admits word-scale intervals for a day, but prevents
# a broken process from filling the hub's memory with stdout.
MAX_DIARIZATION_BYTES = 16 * 1024 * 1024


class DiarizationConfiguration(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    python: Path
    bundle: Path
    gpu_lock: Path


def normalize_voices(payload: dict, duration_ms: int) -> dict:
    """Validate intervals and number every distinct voice by first appearance."""
    spans = payload["spans"]
    if not isinstance(spans, list):
        raise TypeError("Invalid diarization intervals")
    first = {}
    for span in spans:
        start, end, voice = span["start_ms"], span["end_ms"], span["voice"]
        if (
            type(start) is not int
            or type(end) is not int
            or not 0 <= start < end <= duration_ms
            or not isinstance(voice, str)
        ):
            raise ValueError("Invalid voice interval")
        first[voice] = min(first.get(voice, start), start)
    names = {
        voice: f"voice_{index + 1}" for index, voice in enumerate(sorted(first, key=first.get))
    }
    return {
        "spans": sorted(
            [
                {
                    "start_ms": span["start_ms"],
                    "end_ms": span["end_ms"],
                    "voice": names[span["voice"]],
                }
                for span in spans
            ],
            key=lambda s: (s["start_ms"], s["end_ms"], s["voice"]),
        )
    }


async def separate(configuration, audio_fd: int, duration_ms: int) -> dict:
    if configuration is None:
        raise HubError("diarization_not_configured", "Falta la configuración local de separación")
    script = Path(__file__).with_name("diarization_process.py")
    try:
        process = await asyncio.create_subprocess_exec(
            str(configuration.python),
            str(script),
            f"/proc/self/fd/{audio_fd}",
            str(configuration.bundle),
            str(configuration.gpu_lock),
            pass_fds=(audio_fd,),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
            start_new_session=True,
            env={**os.environ, "HF_HUB_OFFLINE": "1", "PYANNOTE_METRICS_ENABLED": "0"},
        )
    except OSError as exc:
        raise HubError(
            "diarization_start_failed",
            "No se pudo iniciar el separador",
            numbers={"errno": exc.errno or 0},
        ) from exc
    try:
        output = bytearray()
        while chunk := await process.stdout.read(65536):
            output.extend(chunk)
            if len(output) > MAX_DIARIZATION_BYTES:
                raise HubError(
                    "diarization_output_too_large",
                    "Respuesta del separador excesiva",
                    numbers={"bytes": len(output), "max_bytes": MAX_DIARIZATION_BYTES},
                )
        status = await process.wait()
        if status:
            raise HubError(
                "diarization_process_failed",
                "Falló el proceso de separación",
                numbers={"exit_status": status},
            )
        try:
            return normalize_voices(json.loads(output), duration_ms)
        except (ValueError, TypeError, KeyError, OverflowError) as exc:
            raise HubError(
                "diarization_invalid_response",
                "Respuesta del separador no válida",
                numbers={"bytes": len(output)},
            ) from exc
    finally:
        if process.returncode is None:
            with suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            await process.wait()
