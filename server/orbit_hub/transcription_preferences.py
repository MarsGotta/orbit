"""The hub's encrypted transcription preference, shared by every paired Mac."""

import asyncio
import io
import json
import wave
from typing import Literal

from fastapi import Request
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from .errors import HubError
from .log import in_store
from .meeting_transcription import MeetingTranscriber
from .openrouter_transcription import OpenRouterTranscriber

Model = Literal["whisper", "mai"]
SCHEMA = """
CREATE TABLE IF NOT EXISTS transcription_preferences (
    id INTEGER PRIMARY KEY CHECK(id=1), body BLOB NOT NULL);
"""
CONTEXT = "hub-transcription-preference"


class Selection(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, hide_input_in_errors=True)
    model: Model
    # 4096 holds provider keys with margin; bounds encrypted credential storage,
    # not audio. Never returned, included in refusals or written to diagnostics.
    api_key: str | None = Field(default=None, min_length=1, max_length=4096, repr=False)


class Preferences:
    def __init__(self, store):
        self.store = store

    def read(self):
        with self.store.read_snapshot() as db:
            row = db.execute("SELECT body FROM transcription_preferences WHERE id=1").fetchone()
            return (
                json.loads(self.store.decrypt(row[0], CONTEXT))
                if row
                else {"model": "whisper", "api_key": None}
            )

    def public(self):
        found = self.read()
        return {"model": found["model"], "key_configured": bool(found["api_key"])}

    def save(self, model, key):
        sealed = self.store.encrypt(json.dumps({"model": model, "api_key": key}).encode(), CONTEXT)
        with self.store.transaction() as db:
            self.store.ensure_capacity(len(sealed))
            db.execute(
                "INSERT INTO transcription_preferences VALUES (1,?) "
                "ON CONFLICT(id) DO UPDATE SET body=excluded.body",
                (sealed,),
            )


class Transcriber:
    """Select once per request/pass. Whisper's existing request stays untouched."""

    accepts_terms = True

    def __init__(self, store, configuration):
        self.preferences = Preferences(store)
        self.whisper = MeetingTranscriber(configuration)
        self.configuration = self.whisper.configuration

    @property
    def model(self):
        return self.preferences.read()["model"]

    def for_model(self, model):
        if model == "whisper":
            return self.whisper
        key = self.preferences.read()["api_key"]
        if not key:
            raise HubError(
                "transcription_key_missing", "Configura la clave de OpenRouter en Ajustes"
            )
        return OpenRouterTranscriber(key, self.configuration)

    async def transcribe(
        self, audio, language=None, *, final=False, prompt="", terms=(), model=None
    ):
        selected = self.for_model(model or self.model)
        options = {"terms": terms} if isinstance(selected, OpenRouterTranscriber) else {}
        return await selected.transcribe(audio, language, final=final, prompt=prompt, **options)

    async def interactive(self, speech, audio):
        if self.model == "whisper":
            return await speech.transcribe(audio)
        selected = self.for_model("mai")
        # Same 180 s provider budget as existing interactive STT, inside the
        # worker's 240 s queue+inference budget and the Mac's 250 s HTTP budget.
        selected.timeout = 180
        return (await selected.transcribe(audio)).text


def validation_audio():
    """A quarter second of invented silence; no person's audio validates a key."""
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as wav:
        wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        wav.writeframes(bytes(8000))
    return buffer.getvalue()


def mount(app, transcriber, identity, bounded_body, executor):
    preferences = transcriber.preferences
    lock = asyncio.Lock()

    @app.get("/v1/settings/transcription")
    async def read(request: Request):
        identity(request)
        return await in_store(executor, preferences.public)

    @app.put("/v1/settings/transcription")
    async def save(request: Request):
        device = identity(request)
        # A 4096-character key escaped as JSON takes <=24 KiB plus envelope.
        try:
            selection = Selection.model_validate_json(await bounded_body(request, 25 * 1024))
        except ValidationError:
            raise HubError(
                "invalid_transcription_setting", "Modelo o clave de transcripción no válidos"
            ) from None
        async with lock:
            return await apply(selection, device)

    async def apply(selection, device):
        if not preferences.store.devices.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        # Serialize validation+save so preserving a key cannot overwrite a
        # replacement another paired Mac just saved.
        previous = await in_store(executor, preferences.read)
        key = selection.api_key or previous["api_key"]
        if selection.model == "mai" or selection.api_key:
            if not key:
                raise HubError("transcription_key_missing", "Introduce la clave de OpenRouter")
            # Validation sends only 250 ms of silence. Its 30 s provider
            # budget ends well before the Mac's 90 s settings request budget.
            await OpenRouterTranscriber(key, transcriber.configuration, timeout=30).transcribe(
                validation_audio()
            )
        # Validation may outlive revocation. A revoked device cannot save.
        if not preferences.store.devices.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        await in_store(executor, preferences.save, selection.model, key)
        return await in_store(executor, preferences.public)


async def final_pass_model(request):
    """Empty body is the older Mac's order; an explicit choice is bounded."""
    body = bytearray()
    # A model name and JSON envelope fit well inside 256 bytes. This limits
    # only the comparison order, never audio or transcription duration.
    async for chunk in request.stream():
        body.extend(chunk)
        if len(body) > 256:
            raise HubError(
                "too_large",
                "La elección de modelo es demasiado grande",
                numbers={"bytes": len(body), "max_bytes": 256},
            )
    if not body:
        return None
    try:
        payload = json.loads(body)
        if set(payload) != {"model"} or payload["model"] not in ("whisper", "mai"):
            raise ValueError("Invalid model")
        return payload["model"]
    except (ValueError, TypeError):
        raise HubError(
            "invalid_transcription_setting", "Elige Whisper o MAI para transcribir"
        ) from None
