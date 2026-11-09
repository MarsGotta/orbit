"""The audio of a final pass: each track rebuilt from its verified chunks, and
the windows sent to Whisper, all in private files (`private_audio`) through
ffmpeg."""

import hashlib
import io
import subprocess
import wave

from . import meeting_transcript_integrity as integrity
from .meeting_transcription import MAX_AUDIO_BYTES, REQUEST_TIMEOUT_SECONDS
from .meetings import repository
from .private_audio import for_child, private_file
from .protocol import HubError

OPUS_BITRATE = 24_000


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
    with private_file() as source:
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
        (source_path, source_fds), (target_path, target_fds) = map(for_child, (source, target))
        ffmpeg(
            [
                "-i",
                source_path,
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                target_path,
            ],
            source_fds + target_fds,
        )
    target.seek(0)
    return True


def encode_window(pcm):
    with private_file() as source, private_file() as target:
        source.write(pcm)
        source.seek(0)
        (source_path, source_fds), (target_path, target_fds) = map(for_child, (source, target))
        ffmpeg(
            [
                "-f",
                "s16le",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-i",
                source_path,
                "-c:a",
                "libopus",
                "-b:a",
                str(OPUS_BITRATE),
                "-vbr",
                "off",
                "-f",
                "ogg",
                target_path,
            ],
            source_fds + target_fds,
        )
        target.seek(0)
        audio = target.read(MAX_AUDIO_BYTES + 1)
        if len(audio) > MAX_AUDIO_BYTES:
            raise HubError(
                "final_window_too_large",
                "The Opus window exceeds the hub model's limit",
                numbers={"bytes": len(audio), "max_bytes": MAX_AUDIO_BYTES},
            )
        return audio


def silence_report(audio):
    """ffmpeg's silence detection over a prepared track (its private file),
    read by `window_ends`.

    Diagnostic ffmpeg has its own existing 300 s budget."""
    path, descriptors = for_child(audio)
    return ffmpeg(
        [
            "-v",
            "info",
            "-i",
            path,
            "-af",
            (
                f"silencedetect=noise={integrity.SILENCE_NOISE_DB}dB:"
                f"d={integrity.SILENCE_DURATION_MS / 1000}"
            ),
            "-f",
            "null",
            "-",
        ],
        descriptors,
        True,
    )
