"""Pure meeting-window selection and WAV assembly; no clocks, files or providers."""

import hashlib
import io
import json
import wave
from dataclasses import dataclass

from .protocol import HubError

TARGET_SECONDS = 20
FLUSH_AFTER_SECONDS = 30
MAX_GROUP_SOURCES = 128
MAX_GROUP_AUDIO_BYTES = 20 * 1024 * 1024
# Native timestamps have callback jitter, but pauses must not become continuous ASR.
MAX_CLOCK_SKEW_NS = 50_000_000
SOURCE_FIELDS = ('capture_epoch_id', 'first_sample', 'frames', 'sample_rate', 'channels', 'monotonic_ns', 'started_at_ms', 'wall_time_precision')  # fmt: skip


@dataclass(frozen=True)
class Source:
    chunk_id: str
    sequence: int
    sha256: str
    size: int
    metadata: dict
    received_at: float

    def provenance(self) -> dict:
        return {
            "chunk_id": self.chunk_id,
            "sequence": self.sequence,
            "sha256": self.sha256,
            **{key: self.metadata[key] for key in SOURCE_FIELDS if key in self.metadata},
        }


def contiguous(previous: Source, current: Source) -> bool:
    left, right = previous.metadata, current.metadata
    required = {"first_sample", "frames", "sample_rate", "channels", "monotonic_ns"}
    return (
        required <= left.keys()
        and required <= right.keys()
        and current.sequence == previous.sequence + 1
        and left.get("capture_epoch_id") == right.get("capture_epoch_id")
        and left["sample_rate"] == right["sample_rate"]
        and left["channels"] == right["channels"]
        and right["first_sample"] == left["first_sample"] + left["frames"]
        and right["monotonic_ns"] >= left["monotonic_ns"]
        and abs(
            right["monotonic_ns"]
            - left["monotonic_ns"]
            - (left["frames"] * 1_000_000_000 + left["sample_rate"] - 1) // left["sample_rate"]
        )
        <= MAX_CLOCK_SKEW_NS
    )


def select_window(
    sources: list[Source], *, now: float, finalized: bool
) -> tuple[list[Source], str] | None:
    """Choose one immutable contiguous prefix; late arrivals never reopen sealed groups."""
    if not sources:
        return None
    selected, total_bytes, seconds = [], 44, 0.0
    for source in sources:
        if selected and source.sequence != selected[-1].sequence + 1:
            # An upload arriving out of order is not a captured discontinuity.
            # Give the missing sequence the existing durable arrival deadline;
            # never merge across the hole or reopen a group after that deadline.
            if finalized:
                return selected, "finalized"
            if now >= min(item.received_at for item in selected) + FLUSH_AFTER_SECONDS:
                return selected, "deadline"
            return None
        if selected and not contiguous(selected[-1], source):
            return selected, "boundary"
        if selected and (
            len(selected) == MAX_GROUP_SOURCES or total_bytes + source.size > MAX_GROUP_AUDIO_BYTES
        ):
            return selected, "bound"
        selected.append(source)
        total_bytes += source.size
        rate, frames = source.metadata.get("sample_rate"), source.metadata.get("frames")
        if isinstance(rate, int) and rate > 0 and isinstance(frames, int) and frames > 0:
            seconds += frames / rate
        if seconds >= TARGET_SECONDS:
            return selected, "target"
    if finalized:
        return selected, "finalized"
    if now >= min(source.received_at for source in selected) + FLUSH_AFTER_SECONDS:
        return selected, "deadline"
    return None


def group_manifest(device: str, recording: str, track: str, sources: list[Source]) -> dict:
    provenance = [source.provenance() for source in sources]
    canonical = {
        "device_id": device,
        "recording_id": recording,
        "track": track,
        "sources": provenance,
        "pipeline_version": 2,
    }
    identity = (
        "group_"
        + hashlib.sha256(
            json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()
    )
    result = {
        "group_id": identity,
        "track": track,
        "sequence": sources[0].sequence,
        "last_sequence": sources[-1].sequence,
        "source_chunk_ids": [source.chunk_id for source in sources],
        "sources": provenance,
        "timing_precision": "group",
        "speaker": "unknown",
        "pipeline_version": canonical["pipeline_version"],
    }
    first = sources[0].metadata
    if all(isinstance(source.metadata.get("frames"), int) for source in sources):
        result["frames"] = sum(source.metadata["frames"] for source in sources)
    result.update({key: first[key] for key in SOURCE_FIELDS if key in first and key != "frames"})
    if len(sources) == 1:
        result.update(chunk_id=sources[0].chunk_id, source_sha256=sources[0].sha256)
    return result


def assemble_wav(sources: list[dict], bodies: list[bytes]) -> bytes:
    """Concatenate PCM, not RIFF containers; never change format or invent silence."""
    if not 1 <= len(sources) == len(bodies) <= MAX_GROUP_SOURCES:
        raise HubError("invalid_group_audio", "Número de fuentes de audio fuera de contrato")
    output, expected_format, size = io.BytesIO(), None, 44
    try:
        with wave.open(output, "wb") as target:
            for source, body in zip(sources, bodies, strict=True):
                if hashlib.sha256(body).hexdigest() != source["sha256"]:
                    raise ValueError("Source checksum mismatch")
                with wave.open(io.BytesIO(body), "rb") as audio:
                    format = (audio.getnchannels(), audio.getsampwidth(), audio.getframerate())
                    if format[0] not in {1, 2} or format[1] != 2 or not 8000 <= format[2] <= 192000:
                        raise ValueError("Unsupported source format")
                    if expected_format is None:
                        expected_format = format
                        target.setnchannels(format[0])
                        target.setsampwidth(2)
                        target.setframerate(format[2])
                    if format != expected_format:
                        raise ValueError("Mixed source formats")
                    for name, actual in (
                        ("frames", audio.getnframes()),
                        ("sample_rate", format[2]),
                        ("channels", format[0]),
                    ):
                        if name in source and source[name] != actual:
                            raise ValueError("Source metadata does not match WAV")
                    expected_bytes = audio.getnframes() * format[0] * 2
                    if size + expected_bytes > MAX_GROUP_AUDIO_BYTES:
                        raise ValueError("Group payload exceeds bound")
                    pcm = audio.readframes(audio.getnframes())
                    if len(pcm) != expected_bytes or not pcm:
                        raise ValueError("Truncated source WAV")
                    target.writeframesraw(pcm)
                    size += len(pcm)
    except (wave.Error, EOFError, ValueError) as exc:
        raise HubError(
            "invalid_group_audio", "Las fuentes no forman un WAV contiguo verificable"
        ) from exc
    return output.getvalue()
