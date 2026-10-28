"""MAI words over OpenRouter's JSON STT API; never provider diarization or fallback."""

import base64

import httpx

from .errors import HubError
from .meeting_transcription import (
    CONNECT_TIMEOUT_SECONDS,
    MAX_AUDIO_BYTES,
    REQUEST_TIMEOUT_SECONDS,
    Transcription,
    language_from,
    timed_words,
)
from .transcription_failure import from_provider

URL = "https://openrouter.ai/api/v1/audio/transcriptions"
MODEL = "microsoft/mai-transcribe-2"


class OpenRouterTranscriber:
    def __init__(self, api_key, configuration, *, timeout=REQUEST_TIMEOUT_SECONDS):
        self._api_key = api_key
        self.timeout = timeout
        self.configuration = configuration

    def _default_client(self):
        return httpx.AsyncClient(
            timeout=httpx.Timeout(self.timeout, connect=CONNECT_TIMEOUT_SECONDS),
            follow_redirects=False,
        )

    async def transcribe(self, audio, language=None, *, final=False, prompt="", terms=()):
        if not audio:
            raise HubError("transcription_audio_empty", "Transcription audio is empty")
        if len(audio) > MAX_AUDIO_BYTES:
            raise HubError(
                "too_large",
                f"The audio is {len(audio)} bytes; the maximum is {MAX_AUDIO_BYTES}",
                numbers={"resource": "audio", "bytes": len(audio), "max_bytes": MAX_AUDIO_BYTES},
            )
        language = language or self.configuration.language
        payload = {
            "model": MODEL,
            "input_audio": {
                "data": base64.b64encode(audio).decode("ascii"),
                "format": "ogg" if final else "wav",
            },
            "response_format": "verbose_json",
            "timestamp_granularities": ["word"],
            "provider": {
                "zdr": True,
                "options": {
                    "azure": {
                        "diarization": {"enabled": False},
                        "phraseList": {"phrases": list(terms)},
                        "enhancedMode": {"modelOptions": {"transcribeStyle": "verbatim"}},
                    }
                },
            },
        }
        if language:
            payload["language"] = language
        try:
            async with self._default_client() as client:
                response = await client.post(
                    URL, json=payload, headers={"Authorization": f"Bearer {self._api_key}"}
                )
                response.raise_for_status()
                result = response.json()
                text = result["text"]
                if not isinstance(text, str):
                    raise TypeError("Invalid transcription text")
                words = timed_words(result.get("words", []))
                if final and text.strip() and not words:
                    raise HubError(
                        "transcription_words_missing",
                        "MAI did not return word timestamps",
                        numbers={"words": 0, "characters": len(text)},
                    )
                return Transcription(text, language_from(result.get("language"), language), words)
        except HubError:
            raise
        except Exception as error:  # noqa: BLE001 - redact provider exceptions at this boundary
            # No response text, URL, headers or chained exception: an upstream
            # may echo the key or transcript in any of them.
            raise refusal(error, self.timeout) from None


def refusal(error, timeout=REQUEST_TIMEOUT_SECONDS):
    if isinstance(error, httpx.HTTPStatusError):
        status = error.response.status_code
        causes = {
            400: ("transcription_audio_refused", "MAI did not accept the audio request"),
            401: ("transcription_key_invalid", "The OpenRouter key is invalid"),
            403: ("transcription_key_forbidden", "The OpenRouter key has no MAI access"),
            402: ("transcription_quota_exhausted", "OpenRouter credit exhausted"),
            429: (
                "transcription_rate_limited",
                "OpenRouter rate limited requests; try again",
            ),
        }
        if status in causes:
            code, message = causes[status]
            return HubError(code, message, http_status=502, numbers={"provider_status": status})
    if isinstance(error, (KeyError, AttributeError)):
        error = TypeError("Invalid transcription response")
    return from_provider(
        error,
        timeout_ms=timeout * 1000,
        connect_timeout_ms=CONNECT_TIMEOUT_SECONDS * 1000,
    )
