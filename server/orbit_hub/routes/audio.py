"""Voice transport routes."""

from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from ..errors import HubError
from ..protocol import SpeechRequest


def mount(
    app, store, identity, bounded_body, validate_wav, settings, agent, transcriptions, turn_inputs
):
    """`agent` is the one whose voice these routes use: the default agent,
    since neither route names a conversation. An agent without a voice is
    refused with its id and the cause before any audio is read."""

    @app.post("/v1/audio/transcribe")
    async def transcribe(request: Request):
        device = identity(request)
        agent.voice()
        body = await bounded_body(request, settings.max_audio_bytes, "audio")
        validate_wav(body)
        text = await transcriptions.transcribe(device, body)
        if not store.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        turn_inputs.mark_transcribed(device, text)
        return {"text": text}

    @app.post("/v1/audio/speech")
    async def speech(request: Request):
        device = identity(request)
        voice = agent.voice()
        # 128 KiB holds every valid request: 16,000 characters of text encode,
        # in the Mac's JSON (control characters as \u00XX), to at most 96,011
        # bytes (computed 2026-10-21). It protects the hub from reading an
        # unbounded body before validating it; it does not limit the clip.
        try:
            payload = SpeechRequest.model_validate_json(
                await bounded_body(request, 128 * 1024, "audio")
            )
        except ValidationError as exc:
            raise HubError("invalid_request", "Texto de síntesis no válido") from exc
        audio = await voice.synthesize(payload.text)
        if not store.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        return Response(
            content=audio.body,
            media_type=audio.mime_type,
            headers={"X-Orbit-Speech-Kind": "clip", "X-Orbit-Speech-Provider": audio.provider},
        )
