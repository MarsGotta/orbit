"""Voice transport routes."""

from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from ..errors import HubError
from ..protocol import SpeechRequest


def mount(
    app, store, identity, bounded_body, validate_wav, settings, adapter, transcriptions, turn_inputs
):
    @app.post("/v1/audio/transcribe")
    async def transcribe(request: Request):
        device = identity(request)
        body = await bounded_body(request, settings.max_audio_bytes)
        validate_wav(body)
        text = await transcriptions.transcribe(device, body)
        if not store.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        turn_inputs.mark_transcribed(device, text)
        return {"text": text}

    @app.post("/v1/audio/speech")
    async def speech(request: Request):
        device = identity(request)
        try:
            payload = SpeechRequest.model_validate_json(await bounded_body(request, 128 * 1024))
        except ValidationError as exc:
            raise HubError("invalid_request", "Texto de síntesis no válido") from exc
        audio = await adapter.synthesize(payload.text)
        if not store.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        return Response(
            content=audio.body,
            media_type=audio.mime_type,
            headers={"X-Orbit-Speech-Kind": "clip", "X-Orbit-Speech-Provider": audio.provider},
        )
