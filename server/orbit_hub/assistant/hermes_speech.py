"""Dictation and read-aloud by a Hermes profile, over Hermes's HTTP audio endpoints."""

from __future__ import annotations

import base64
import json
from typing import TYPE_CHECKING

import httpx

from ..log import logger, report
from ..protocol import HubError
from ..transcription_failure import from_provider
from .hermes_credential import credential
from .speech import MAX_SPEECH_CLIP_BYTES, SpeechClip

if TYPE_CHECKING:
    from ..config import Settings

MAX_SPEECH_RESPONSE_BYTES = 6 * 1024 * 1024
# Existing 180 s provider and 10 s connection budgets, shared with the
# refusal's millisecond numbers. They bound one dictation call, not meeting
# ASR, queue wait, or an automatic retry.
TRANSCRIPTION_TIMEOUT_SECONDS = 180
TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS = 10


class HermesSpeech:
    """Dictation and read-aloud by the Hermes profile `profile`."""

    def __init__(self, settings: Settings, profile: str):
        self.settings, self.profile = settings, profile

    async def transcribe(self, audio: bytes) -> str:
        async with httpx.AsyncClient(
            timeout=httpx.Timeout(
                TRANSCRIPTION_TIMEOUT_SECONDS, connect=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS
            ),
            follow_redirects=False,
        ) as client:
            try:
                response = await client.post(
                    self.settings.hermes_url.rstrip("/") + "/api/audio/transcribe",
                    params={"profile": self.profile},
                    headers={"X-Hermes-Session-Token": credential(self.settings)},
                    json={
                        "data_url": "data:audio/wav;base64," + base64.b64encode(audio).decode(),
                        "mime_type": "audio/wav",
                    },
                )
                response.raise_for_status()
                payload = response.json()
                if payload.get("ok") is not True or not isinstance(payload.get("transcript"), str):
                    raise ValueError("Invalid transcription response")
                return payload["transcript"]
            except Exception as exc:
                report(event="hermes_transcription_failed", code="transcription_failed")
                raise from_provider(
                    exc,
                    timeout_ms=TRANSCRIPTION_TIMEOUT_SECONDS * 1000,
                    connect_timeout_ms=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS * 1000,
                ) from exc

    async def synthesize(self, text: str) -> SpeechClip:
        try:
            # 10 s to connect to Hermes and 180 s for each read, write or pooled
            # connection, as for dictation: it protects the request from a
            # Hermes that stops answering. It is not a total: a slow stream is
            # bounded by its 6 MiB, not by time. The Mac gives up on this route
            # after 90 s (HUB_HTTP_TIMEOUT in orbit-daemon's hub.rs), so past
            # that the hub waits for nobody; 180 s was not measured for speech.
            async with (
                httpx.AsyncClient(
                    timeout=httpx.Timeout(180, connect=10), follow_redirects=False
                ) as client,
                client.stream(
                    "POST",
                    self.settings.hermes_url.rstrip("/") + "/api/audio/speak",
                    headers={"X-Hermes-Session-Token": credential(self.settings)},
                    json={"text": text, "profile": self.profile},
                ) as response,
            ):
                response.raise_for_status()
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(chunk) > MAX_SPEECH_RESPONSE_BYTES - len(body):
                        # Hermes's whole answer (JSON around the base64 clip),
                        # cut while it streams: `bytes` is what had arrived.
                        raise HubError(
                            "speech_too_large",
                            "The Hermes speech response exceeds the 6 MiB limit",
                            numbers={
                                "bytes": len(body) + len(chunk),
                                "max_bytes": MAX_SPEECH_RESPONSE_BYTES,
                            },
                        )
                    body.extend(chunk)
            payload = json.loads(body)
            mime_type = payload.get("mime_type")
            provider = payload.get("provider")
            data_url = payload.get("data_url")
            if (
                payload.get("ok") is not True
                or not isinstance(mime_type, str)
                or not 1 <= len(mime_type) <= 128
                or any(ord(character) < 33 or ord(character) > 126 for character in mime_type)
                or not isinstance(provider, str)
                or not 1 <= len(provider) <= 128
                or any(ord(character) < 32 or ord(character) > 126 for character in provider)
                or not isinstance(data_url, str)
            ):
                raise ValueError("Invalid speech response")
            prefix, separator, encoded = data_url.partition(",")
            if separator != "," or prefix != f"data:{mime_type};base64":
                raise ValueError("Invalid speech data URL")
            maximum_encoded = 4 * ((MAX_SPEECH_CLIP_BYTES + 2) // 3)
            if len(encoded) > maximum_encoded:
                # The clip's size from its base64 length, before decoding it.
                raise HubError(
                    "speech_too_large",
                    "The speech clip exceeds the 4 MiB limit",
                    numbers={
                        "bytes": len(encoded) // 4 * 3 - (len(encoded) - len(encoded.rstrip("="))),
                        "max_bytes": MAX_SPEECH_CLIP_BYTES,
                    },
                )
            audio = base64.b64decode(encoded, validate=True)
            if not audio:
                raise ValueError("Empty speech clip")
            if len(audio) > MAX_SPEECH_CLIP_BYTES:
                raise HubError(
                    "speech_too_large",
                    "The speech clip exceeds the 4 MiB limit",
                    numbers={"bytes": len(audio), "max_bytes": MAX_SPEECH_CLIP_BYTES},
                )
            logger.info(
                "hermes_speech_clip",
                extra={"event": "hermes_speech_clip", "code": f"{provider}|{mime_type}"},
            )
            return SpeechClip(mime_type, provider, audio)
        except HubError:
            raise
        except Exception as exc:
            report(event="hermes_speech_clip_failed", code="speech_unavailable")
            raise HubError(
                "speech_unavailable", "Hermes did not deliver the configured speech clip"
            ) from exc
