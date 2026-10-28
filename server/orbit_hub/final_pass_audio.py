"""The audio of a final pass: each track rebuilt from its verified chunks, and
the windows sent to Whisper, all in anonymous memory files through ffmpeg."""

import hashlib
import io
import os
import subprocess
import wave
from contextlib import contextmanager

from . import meeting_transcript_integrity as integrity
from .meeting_transcription import MAX_AUDIO_BYTES, REQUEST_TIMEOUT_SECONDS
from .meetings import repository
from .protocol import HubError

OPUS_BITRATE = 24_000


@contextmanager
def memory_file():
    # Linux hub only: memfd has no directory entry or plaintext fallback.
    descriptor = os.memfd_create("orbit-final-audio", os.MFD_CLOEXEC)
    with os.fdopen(descriptor, "w+b") as handle:
        yield handle


def ffmpeg(arguments, descriptors, capture_stderr=False):
    try:
        completed = subprocess.run(
            ["ffmpeg", "-nostdin", "-v", "error", "-y", *arguments],
            pass_fds=descriptors,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE if capture_stderr else subprocess.DEVNULL,
            check=True,
            timeout=REQUEST_TIMEOUT_SECONDS,
        )
        return completed.stderr
    except subprocess.TimeoutExpired as exc:
        raise HubError(
            "final_audio_timeout",
            "Audio conversion exceeded its deadline",
            numbers={"timeout_ms": REQUEST_TIMEOUT_SECONDS * 1000},
        ) from exc
    except (OSError, subprocess.CalledProcessError) as exc:
        raise HubError(
            "final_audio_conversion_failed",
            "Could not convert the audio",
            numbers={
                "exit_status": getattr(exc, "returncode", 0),
                "errno": getattr(exc, "errno", 0) or 0,
            },
        ) from exc


def prepare_track(repo, job, track, target):
    """Verify original chunks and preserve each wall-clock gap in the final timeline."""
    started, expected = None, None
    with repo.store.read_snapshot() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (job["device_id"], job["recording"]),
        ).fetchone()
        manifest = repository._open(
            repo.store, row["body"], f"manifest:{job['device_id']}:{job['recording']}"
        )
        # A closed manifest declares exactly which tracks were uploaded.
        # A remote-only recording must not try to build an empty microphone WAV.
        if not any(item["track"] == track for item in manifest["tracks"]):
            return False
        began = manifest["started_at_ms"]
    with memory_file() as source:
        with wave.open(source, "wb") as output:
            for body, metadata, checksum in repo.chunks(job, track):
                if hashlib.sha256(body).hexdigest() != checksum:
                    raise ValueError("Corrupt audio")
                with wave.open(io.BytesIO(body), "rb") as audio:
                    shape = (audio.getnchannels(), audio.getsampwidth(), audio.getframerate())
                    if shape[0] not in (1, 2) or shape[1] != 2 or not 8000 <= shape[2] <= 192000:
                        raise ValueError("Invalid PCM format")
                    if started is None:
                        started, expected = began, shape
                        output.setnchannels(shape[0])
                        output.setsampwidth(shape[1])
                        output.setframerate(shape[2])
                    if shape != expected or metadata["frames"] != audio.getnframes():
                        raise ValueError("Inconsistent audio")
                    wall = metadata.get("started_at_ms")
                    if type(wall) is int:
                        if not began <= wall <= manifest["finalized_at_ms"]:
                            raise HubError(
                                "final_audio_time_out_of_range",
                                "The chunk starts outside the recording",
                                numbers={
                                    "started_at_ms": wall,
                                    "recording_start_ms": began,
                                    "recording_end_ms": manifest["finalized_at_ms"],
                                },
                            )
                        gap = max(0, (wall - began) * shape[2] // 1000 - output.getnframes())
                        # Write gaps in second-sized pieces, never one giant allocation.
                        while gap:
                            frames = min(gap, shape[2])
                            output.writeframesraw(bytes(frames * shape[0] * 2))
                            gap -= frames
                    pcm = audio.readframes(audio.getnframes())
                    if len(pcm) != audio.getnframes() * shape[0] * 2:
                        raise ValueError("Truncated PCM")
                    output.writeframesraw(pcm)
        if started is None:
            return False
        source.seek(0)
        ffmpeg(
            [
                "-i",
                f"/proc/self/fd/{source.fileno()}",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                f"/proc/self/fd/{target.fileno()}",
            ],
            (source.fileno(), target.fileno()),
        )
    target.seek(0)
    return True


def encode_window(pcm):
    with memory_file() as source, memory_file() as target:
        source.write(pcm)
        source.seek(0)
        ffmpeg(
            [
                "-f",
                "s16le",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-i",
                f"/proc/self/fd/{source.fileno()}",
                "-c:a",
                "libopus",
                "-b:a",
                str(OPUS_BITRATE),
                "-vbr",
                "off",
                "-f",
                "ogg",
                f"/proc/self/fd/{target.fileno()}",
            ],
            (source.fileno(), target.fileno()),
        )
        target.seek(0)
        audio = target.read(MAX_AUDIO_BYTES + 1)
        if len(audio) > MAX_AUDIO_BYTES:
            raise HubError(
                "final_window_too_large",
                "The Opus window exceeds the Whisper limit",
                numbers={"bytes": len(audio), "max_bytes": MAX_AUDIO_BYTES},
            )
        return audio


def silence_report(descriptor):
    """ffmpeg's silence detection over a prepared track, read by `window_ends`.

    Diagnostic ffmpeg has its own existing 300 s budget."""
    return ffmpeg(
        [
            "-v",
            "info",
            "-i",
            f"/proc/self/fd/{descriptor}",
            "-af",
            (
                f"silencedetect=noise={integrity.SILENCE_NOISE_DB}dB:"
                f"d={integrity.SILENCE_DURATION_MS / 1000}"
            ),
            "-f",
            "null",
            "-",
        ],
        (descriptor,),
        True,
    )
