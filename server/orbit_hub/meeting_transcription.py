"""Direct STT for durable meetings, separate from interactive Hermes dictation.

Endpoint, model and decoding hints belong to the mandatory hub configuration.
"""

import asyncio
import io
import math
import re
import wave
from dataclasses import dataclass
from typing import Self

import httpx
from pydantic import BaseModel, ConfigDict, Field, model_validator

from .log import report
from .protocol import HubError
from .transcription_failure import from_provider
from .urls import protected_url

# Long enough for a multi-minute group on a busy GPU, bounded so a wedged
# endpoint fails the job rather than holding the single inference worker for
# ever, which would also stop interactive voice.
REQUEST_TIMEOUT_SECONDS = 300
CONNECT_TIMEOUT_SECONDS = 10
# The endpoint reads the whole body into memory; the hub's own chunk ceiling is
# smaller, so this only guards against a caller that grew one without noticing.
MAX_AUDIO_BYTES = 24 * 1024 * 1024
# A language the transcriber names as ISO 639-1, with or without a region
# ("es", "es-ES", "es_ES"). Anything else ("auto", "spanish", a number) is not a
# language the hub can name, and the configured one stands in for it.
_LANGUAGE = re.compile(r"^([a-z]{2})(?:[-_][a-z]{2})?$")


# Digital silence: a sample at or under this magnitude (of 32 767, about
# -78 dBFS). A microphone's own noise floor sits far above it, so only exact
# or near-exact zeros are trimmed: the system-audio track while nobody plays
# anything, or a virtual device. Whisper invents words over leading digital
# silence — «¿Qué pasa con el presupuesto?» for 3 to 6 s of zeros before the
# first word (2026-10-17; reproduced with json and verbose_json
# alike) — and answers the same audio correctly once it is trimmed.
DIGITAL_SILENCE_PEAK = 4


# A run of digital-silence samples, little-endian 16-bit: 0..4 is
# `[\x00-\x04]\x00`, -4..-1 is `[\xfc-\xff]\xff`; the second pattern reads the
# same run backwards. A regex walks the bytes in C: a whole silent group of
# 20 s at 48 kHz stereo is searched in milliseconds.
_SILENT_RUN = re.compile(rb"(?:[\x00-\x04]\x00|[\xfc-\xff]\xff)*")
_SILENT_RUN_BACKWARDS = re.compile(rb"(?:\x00[\x00-\x04]|\xff[\xfc-\xff])*")
assert DIGITAL_SILENCE_PEAK == 4, "the byte patterns above spell out ±4"


def trim_digital_silence(audio: bytes) -> bytes | None:
    """The WAV without its leading and trailing digital silence; None when
    nothing but silence is left. Anything that is not a 16-bit WAV is sent
    as given: the hub only ever builds those (`grouping.assemble_wav`)."""
    try:
        with wave.open(io.BytesIO(audio), "rb") as source:
            channels, width, rate = (
                source.getnchannels(),
                source.getsampwidth(),
                source.getframerate(),
            )
            pcm = source.readframes(source.getnframes())
    except (wave.Error, EOFError):
        return audio
    if width != 2:
        return audio
    frame = 2 * channels
    lead = _SILENT_RUN.match(pcm).end() // frame * frame
    if lead >= len(pcm) - len(pcm) % frame:
        return None
    tail = _SILENT_RUN_BACKWARDS.match(pcm[::-1]).end() // frame * frame
    kept = pcm[lead : len(pcm) - tail]
    if lead == 0 and tail == 0:
        return audio
    output = io.BytesIO()
    with wave.open(output, "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(2)
        target.setframerate(rate)
        target.writeframes(kept)
    return output.getvalue()


@dataclass(frozen=True)
class Transcription:
    text: str
    # The transcriber's language, else the one it was sent; None if neither.
    language: str | None
    words: tuple[dict, ...] = ()


def language_from(value, configured: str | None) -> str | None:
    match = _LANGUAGE.match(value.lower()) if isinstance(value, str) else None
    return match.group(1) if match else configured


class MeetingTranscriptionConfiguration(BaseModel):
    """Complete settings for the hub's mandatory meeting transcriber."""

    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)
    endpoint: str = Field(min_length=1, max_length=2048)
    model: str = Field(min_length=1, max_length=256)
    language: str | None = Field(default=None, pattern=r"^[a-z]{2}$")

    @model_validator(mode="after")
    def protected_endpoint(self) -> Self:
        protected_url(self.endpoint, allow_insecure_localhost=True)
        return self


class MeetingTranscriber:
    """Calls an OpenAI-compatible transcription endpoint with its own prompt."""

    def __init__(
        self,
        configuration,
        *,
        client_factory=None,
        credential=None,
        timeout=None,
    ):
        self.configuration = MeetingTranscriptionConfiguration.model_validate(configuration)
        self._timeout = timeout
        self._credential = credential
        self._client_factory = client_factory or self._default_client

    @property
    def request_timeout(self):
        return REQUEST_TIMEOUT_SECONDS if self._timeout is None else self._timeout

    def _default_client(self):
        return httpx.AsyncClient(
            timeout=httpx.Timeout(self.request_timeout, connect=CONNECT_TIMEOUT_SECONDS),
            follow_redirects=False,
            trust_env=False,
            headers={"Authorization": "Bearer " + self._credential} if self._credential else {},
        )

    def form(self, language: str | None = None, prompt: str = "") -> dict:
        """The non-audio fields of the request.

        `verbose_json` is `json` plus the language of the transcript;
        a server that leaves it out still answers the text. An empty prompt is
        omitted because some servers treat it as a prompt. `language` is the
        recording's own; without one, the configured one.
        """
        fields = {"model": self.configuration.model, "response_format": "verbose_json"}
        language = language or self.configuration.language
        if language:
            fields["language"] = language
        if prompt:
            fields["prompt"] = prompt
        return fields

    @property
    def url(self) -> str:
        """Return the validated OpenAI-compatible transcription URL."""
        return self.configuration.endpoint.rstrip("/") + "/v1/audio/transcriptions"

    async def transcribe(
        self, audio: bytes, language: str | None = None, *, final: bool = False, prompt: str = ""
    ) -> Transcription:
        """`language` is the recording's; None sends the configured one."""
        url = self.url
        sent = language or self.configuration.language
        if not audio or len(audio) > MAX_AUDIO_BYTES:
            raise HubError("transcription_failed", "Audio de reunión fuera de límites")
        # Off the event loop: a whole group is read and searched (~0.1 s for
        # 20 s of 48 kHz stereo).
        trimmed = audio if final else await asyncio.to_thread(trim_digital_silence, audio)
        if trimmed is None:
            # Only digital silence: nothing was said, and nothing is invented.
            return Transcription("", sent)
        audio = trimmed
        fields = self.form(sent, prompt)
        if final:
            fields.update(
                {
                    "timestamp_granularities[]": "word",
                    "vad_filter": str(FINAL_VAD_FILTER).lower(),
                }
            )
        try:
            async with self._client_factory() as client:
                response = await client.post(
                    url,
                    data=fields,
                    files={
                        "file": (
                            "meeting.ogg" if final else "meeting.wav",
                            audio,
                            "audio/ogg" if final else "audio/wav",
                        )
                    },
                )
                response.raise_for_status()
                payload = response.json()
                text = payload.get("text")
                if not isinstance(text, str):
                    raise TypeError("Invalid transcription response")
                words = timed_words(payload.get("words", []))
                if final and text.strip() and not words:
                    raise HubError(
                        "transcription_words_missing",
                        "Whisper no devolvió tiempos por palabra",
                        numbers={"words": 0, "characters": len(text)},
                    )
                # Whisper names its language in words ("english"): the
                # group is then in the language it was sent.
                return Transcription(text, language_from(payload.get("language"), sent), words)
        except HubError:
            raise
        except Exception as exc:
            # Deliberately opaque: the endpoint's own error text can quote the
            # audio's transcript back, and hub errors are written to logs that
            # must not carry conversation content.
            report(event="meeting_transcription_failed", code="transcription_failed")
            raise from_provider(
                exc,
                timeout_ms=int(self.request_timeout * 1000),
                connect_timeout_ms=int(CONNECT_TIMEOUT_SECONDS * 1000),
            ) from exc


# The second real measurement (October 2026) supersedes the first: whole
# failed audio without VAD lost the same opening in 8/8 runs (78–79% recall); VAD recovered
# 97% in 3/3. Windows + VAD lost zero draft groups on all nine real tracks,
# vs two groups on two tracks with windows and no VAD. The service applies
# faster-whisper's own VAD settings (it takes `vad_filter` and no parameters),
# and those are what was measured.
FINAL_VAD_FILTER = True


def timed_words(values) -> tuple[dict, ...]:
    if not isinstance(values, list) or not all(isinstance(value, dict) for value in values):
        raise TypeError("Invalid word collection")
    words = []
    for value in values:
        start, end = value.get("start"), value.get("end")
        text = value.get("word", value.get("text"))
        if (
            type(start) not in (int, float)
            or type(end) not in (int, float)
            or not math.isfinite(start)
            or not math.isfinite(end)
            or not 0 <= start <= end
            or not isinstance(text, str)
        ):
            raise ValueError("Invalid word times")
        words.append({"start_ms": round(start * 1000), "end_ms": round(end * 1000), "text": text})
    return tuple(words)
