"""Dictation and read-aloud for an `openai` agent: the hub's meeting transcriber,
and an OpenAI-compatible `/audio/speech` when `[agents.speech]` names one."""

from typing import TYPE_CHECKING

import httpx

from ..log import report
from ..meeting_transcription import MeetingTranscriber
from ..protocol import HubError
from .openai_settings import SpeechSettings, read_api_key
from .speech import MAX_SPEECH_CLIP_BYTES, SpeechClip

if TYPE_CHECKING:
    from .agents import AgentConfiguration

# Read-aloud: the same 180 s provider and 10 s connection budgets as Hermes's.
SPEECH_TIMEOUT_SECONDS = 180
SPEECH_CONNECT_TIMEOUT_SECONDS = 10


class OpenAISpeech:
    """Dictation by the hub's meeting transcriber; read-aloud by `speech`, if any."""

    def __init__(
        self, agent: "AgentConfiguration", transcriber: MeetingTranscriber, *, credential=None
    ):
        self._credential = credential
        self.agent, self.settings, self.transcriber = agent, agent.settings, transcriber

    async def transcribe(self, audio: bytes) -> str:
        return (await self.transcriber.transcribe(audio)).text

    async def synthesize(self, text: str) -> SpeechClip:
        speech = self.settings.speech
        if speech is None:
            raise HubError(
                "speech_not_configured",
                f"{self.agent.name} («{self.agent.id}») has no voice for reading aloud: [agents.speech] is missing from its configuration",
                http_status=503,
                numbers={"agent_id": self.agent.id},
            )
        headers = {"Authorization": "Bearer " + self._credential} if self._credential else {}
        if speech.api_key_file is not None:
            key = read_api_key(
                f'agent "{self.agent.id}".speech', "api_key_file", speech.api_key_file
            )
            headers["Authorization"] = f"Bearer {key}"
        try:
            async with (
                httpx.AsyncClient(
                    timeout=httpx.Timeout(
                        SPEECH_TIMEOUT_SECONDS, connect=SPEECH_CONNECT_TIMEOUT_SECONDS
                    ),
                    follow_redirects=False,
                    trust_env=False,
                ) as client,
                client.stream(
                    "POST",
                    speech.base_url + "/audio/speech",
                    headers=headers,
                    json={
                        "model": speech.model,
                        "voice": speech.voice,
                        "input": text,
                        "response_format": "mp3",
                    },
                ) as response,
            ):
                mime_type, body = await _clip(speech, response)
        except HubError:
            raise
        except httpx.TimeoutException as exc:
            report(event="assistant_speech_failed", code="speech_timeout")
            connecting = isinstance(exc, httpx.ConnectTimeout)
            seconds = SPEECH_CONNECT_TIMEOUT_SECONDS if connecting else SPEECH_TIMEOUT_SECONDS
            raise HubError(
                "speech_timeout",
                f"The speech service ({speech.base_url}) did not {('accept the connection' if connecting else 'respond')} en {seconds * 1000} ms",
                http_status=504,
                numbers={"timeout_ms": seconds * 1000},
            ) from exc
        except httpx.HTTPError as exc:
            report(event="assistant_speech_failed", code="speech_unreachable")
            raise HubError(
                "speech_unreachable",
                f"Could not connect to the speech service ({speech.base_url})",
                http_status=503,
            ) from exc
        if not body:
            raise HubError(
                "speech_empty", "The speech service delivered an empty clip", http_status=502
            )
        # The Mac shows where a clip came from: the speech model, as printable ASCII.
        provider = "".join(c if " " <= c <= "~" else "?" for c in speech.model)[:128]
        return SpeechClip(mime_type, provider, bytes(body))


async def _clip(speech: SpeechSettings, response: httpx.Response) -> tuple[str, bytearray]:
    """The audio's type and bytes, refused by status, type or size as they stream."""
    if response.status_code != 200:
        raise HubError(
            "speech_http_error",
            f"The speech service ({speech.base_url}) returned HTTP {response.status_code}",
            http_status=502,
            numbers={"provider_status": response.status_code},
        )
    mime_type = response.headers.get("content-type", "").split(";")[0].strip()
    if not mime_type.startswith("audio/") or len(mime_type) > 128:
        raise HubError(
            "speech_not_audio",
            f"The speech service ({speech.base_url}) did not return audio",
            http_status=502,
        )
    body = bytearray()
    async for chunk in response.aiter_bytes():
        if len(chunk) > MAX_SPEECH_CLIP_BYTES - len(body):
            raise HubError(
                "speech_too_large",
                "The speech clip exceeds the 4 MiB limit",
                numbers={"bytes": len(body) + len(chunk), "max_bytes": MAX_SPEECH_CLIP_BYTES},
            )
        body.extend(chunk)
    return mime_type, body
