"""Redacted causes for both transcription transports."""

import io
import wave

import httpx

from .errors import HubError

# A dictated clip shorter than this that the provider refuses (Hermes answers
# 400 for whatever its speech-to-text refused) is heard as no speech, not as a
# failure. Why 3 s: every call turn carries 300 ms of preroll and 700 ms of
# closing silence (orbit-core `VoicePolicy`), so under 3 s holds under 2 s of
# voice, a word or two at most; the refused clips of 2026-10-23 were 1-2 s of
# noise during a call. A longer clip refused is a real failure and is named
# (`transcription_provider_refused`). It does not limit how long a clip is.
NO_SPEECH_REFUSAL_MAX_MS = 3_000
# What Hermes's `/api/audio/transcribe` answers when its speech-to-text
# refuses the audio; other statuses keep `transcription_http_error`.
PROVIDER_REFUSED_STATUS = 400


def from_provider(error: Exception, *, timeout_ms: int, connect_timeout_ms: int) -> HubError:
    if isinstance(error, httpx.HTTPStatusError):
        status = error.response.status_code
        return HubError(
            "transcription_http_error",
            f"The transcription service returned HTTP {status}",
            http_status=502,
            numbers={"provider_status": status},
        )
    if isinstance(error, httpx.ConnectTimeout):
        return HubError(
            "transcription_connection_timeout",
            f"The transcription service connection took longer than {connect_timeout_ms} ms",
            http_status=504,
            numbers={"timeout_ms": connect_timeout_ms},
        )
    if isinstance(error, httpx.TimeoutException):
        return HubError(
            "transcription_timeout",
            f"The transcription service took longer than {timeout_ms} ms",
            http_status=504,
            numbers={"timeout_ms": timeout_ms},
        )
    if isinstance(error, httpx.RemoteProtocolError):
        return HubError(
            "transcription_protocol_error",
            "The transcription service closed the response before completing it",
            http_status=502,
        )
    if isinstance(error, httpx.ConnectError):
        return HubError(
            "transcription_connection_error",
            "Could not connect to the transcription service",
            http_status=503,
        )
    if isinstance(error, (ValueError, TypeError)):
        return HubError(
            "transcription_invalid_response",
            "The transcription service returned an invalid response",
            http_status=502,
        )
    return HubError(
        "transcription_provider_error",
        "The transcription service failed before delivering a valid response",
        http_status=502,
    )


def durable_code(error: HubError) -> str:
    if (
        error.code != "transcription_http_error"
        and error.numbers
        and "provider_status" in error.numbers
    ):
        return f"{error.code}_status_{error.numbers['provider_status']}"
    if error.code == "transcription_http_error" and error.numbers:
        return f"transcription_http_{error.numbers['provider_status']}"
    if error.code == "transcription_timeout" and error.numbers:
        return f"transcription_timeout_{error.numbers['timeout_ms']}"
    if error.code == "transcription_connection_timeout" and error.numbers:
        return f"transcription_connection_timeout_{error.numbers['timeout_ms']}"
    return error.code


def durable_details(code: str | None) -> dict[str, int | str]:
    if code is None:
        return {"cause": "transcription_cause_missing"}
    if "_status_" in code:
        cause, status = code.rsplit("_status_", 1)
        if status.isdecimal() and 100 <= int(status) <= 599:
            return {"cause": cause, "provider_status": int(status)}
    if code.startswith("transcription_http_"):
        status = code.removeprefix("transcription_http_")
        if status.isdecimal() and 100 <= int(status) <= 599:
            return {"cause": "transcription_http_error", "provider_status": int(status)}
    if code.startswith("transcription_timeout_"):
        timeout = code.removeprefix("transcription_timeout_")
        if timeout.isdecimal():
            return {"cause": "transcription_timeout", "timeout_ms": int(timeout)}
    if code.startswith("transcription_connection_timeout_"):
        timeout = code.removeprefix("transcription_connection_timeout_")
        if timeout.isdecimal():
            return {"cause": "transcription_connection_timeout", "timeout_ms": int(timeout)}
    return {"cause": code}


def wav_duration_ms(body: bytes) -> int:
    """The length of a WAV the route has already validated."""
    with wave.open(io.BytesIO(body), "rb") as audio:
        return audio.getnframes() * 1000 // audio.getframerate()


def dictation_refusal(error: HubError, duration_ms: int) -> HubError | None:
    """What a dictated clip's refusal means: `None` when it is heard as no
    speech (a short clip the provider refused), otherwise the error to answer,
    with the provider's refusal of a longer clip named as such."""
    status = (error.numbers or {}).get("provider_status")
    if error.code != "transcription_http_error" or status != PROVIDER_REFUSED_STATUS:
        return error
    if duration_ms < NO_SPEECH_REFUSAL_MAX_MS:
        return None
    return HubError(
        "transcription_provider_refused",
        f"The transcription service did not accept {duration_ms} ms of audio (HTTP {status})",
        http_status=502,
        numbers={"provider_status": status, "duration_ms": duration_ms},
    )
