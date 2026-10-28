"""Voice transport routes."""

from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from ..errors import HubError
from ..log import current_op, get_logger
from ..protocol import SpeechRequest
from ..transcription_failure import dictation_refusal, wav_duration_ms

logger = get_logger(__name__)


def mount(
    app, store, identity, bounded_body, validate_wav, settings, agents, transcriptions, turn_inputs
):
    """Dictation is the default agent's: it is the user's input to whichever
    agent they talk to, not anyone's voice. Read-aloud is the voice of the
    agent the request names, or the default agent's when it names none or that
    agent has no voice of its own. Without any voice, the refusal names the
    agent and the cause before any audio is read or made."""

    @app.post("/v1/audio/transcribe")
    async def transcribe(request: Request):
        device = identity(request)
        if getattr(transcriptions.meetings, "requires_default_speech", True):
            agents.default.voice(listening=True)
        body = await bounded_body(request, settings.max_audio_bytes, "audio")
        validate_wav(body)
        try:
            text = await transcriptions.transcribe(device, body)
        except HubError as error:
            refusal = dictation_refusal(error, wav_duration_ms(body))
            if refusal is not None:
                raise refusal from error
            # A short clip the provider refused holds no speech: no words,
            # which the Mac drops without a failure. Not a failure here
            # either: an info line with its numbers, never its audio.
            logger.info(
                "dictation_no_speech",
                extra={
                    "event": "dictation_no_speech",
                    "code": "provider_refused_short_clip",
                    "op": current_op(),
                    "device": device,
                    "numbers": {
                        "provider_status": (error.numbers or {}).get("provider_status"),
                        "duration_ms": wav_duration_ms(body),
                    },
                },
            )
            text = ""
        if not store.devices.is_active(device):
            raise HubError("revoked", "Device revoked")
        if text:
            turn_inputs.mark_transcribed(device, text)
        return {"text": text}

    @app.post("/v1/audio/speech")
    async def speech(request: Request):
        device = identity(request)
        # 128 KiB holds every valid request: 16,000 characters of text encode,
        # in the Mac's JSON (control characters as \u00XX), to at most 96,011
        # bytes (computed 2026-10-21); an agent id of at most 80 characters
        # adds under 1 KiB. It protects the hub from reading an unbounded
        # body before validating it; it does not limit the clip.
        try:
            payload = SpeechRequest.model_validate_json(
                await bounded_body(request, 128 * 1024, "audio")
            )
        except ValidationError as exc:
            raise HubError("invalid_request", "Invalid synthesis text") from exc
        agent = agents.default if payload.agent_id is None else agents.get(payload.agent_id)
        try:
            audio = await agent.voice().synthesize(payload.text)
        except HubError as refusal:
            # An agent without a voice of its own (Claude Code, Codex, a generic
            # model without `[agents.speech]`) is read in the default agent's:
            # the person asked to hear its answer. Without that one either, the
            # refusal names the agent asked for.
            if refusal.code != "speech_not_configured" or agent is agents.default:
                raise
            try:
                audio = await agents.default.voice().synthesize(payload.text)
            except HubError as fallback:
                unvoiced = fallback.code == "speech_not_configured"
                raise (refusal if unvoiced else fallback) from None
        if not store.devices.is_active(device):
            raise HubError("revoked", "Device revoked")
        return Response(
            content=audio.body,
            media_type=audio.mime_type,
            headers={"X-Orbit-Speech-Kind": "clip", "X-Orbit-Speech-Provider": audio.provider},
        )
