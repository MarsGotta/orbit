"""Resolve app overrides at request boundaries; operator defaults stay intact."""

from contextlib import asynccontextmanager
from types import SimpleNamespace

from pydantic import SecretStr

from ..assistant.openai_settings import SpeechSettings
from ..assistant.openai_speech import OpenAISpeech
from ..errors import HubError
from ..meeting_transcription import MeetingTranscriber
from ..processing import ProcessingClient, ProcessingConfiguration, ProcessingError
from .discovery import api_root


def transcriber(provider, configuration, **options):
    return MeetingTranscriber(
        {
            "endpoint": api_root(provider["base_url"]).removesuffix("/v1"),
            "model": provider["model"],
            "language": configuration.language,
        },
        credential=provider.get("api_key"),
        **options,
    )


class FunctionSpeech:
    def __init__(self, registry, fallback):
        self.registry, self.fallback = registry, fallback

    @property
    def no_dictation(self):
        return self.fallback is None

    @property
    def unvoiced(self):
        return self.fallback is None and not self.overridden

    @property
    def overridden(self):
        return self.registry.selected("speech") is not None

    async def transcribe(self, audio):
        if self.fallback is None:
            raise HubError(
                "speech_not_configured", "No hay escucha configurada para este asistente"
            )
        return await self.fallback.transcribe(audio)

    async def synthesize(self, text):
        provider = self.registry.selected("speech")
        if provider is None:
            if self.fallback is None:
                raise HubError(
                    "speech_not_configured", "Este asistente no tiene una voz configurada"
                )
            return await self.fallback.synthesize(text)
        # The voice name is the operator's when configured; otherwise OpenAI's
        # standard voice. Credentials remain in memory on the hub, never a file.
        voice = getattr(
            getattr(getattr(self.fallback, "settings", None), "speech", None), "voice", "alloy"
        )
        speech = SpeechSettings(api_root(provider["base_url"]), provider["model"], voice, None)
        agent = SimpleNamespace(id="speech", name="Voz", settings=SimpleNamespace(speech=speech))
        return await OpenAISpeech(agent, None, credential=provider.get("api_key")).synthesize(text)


class FunctionProcessing:
    def __init__(self, registry, fallback):
        self.registry, self.fallback = registry, fallback

    @property
    def configured(self):
        return self.fallback is not None or self.registry.selected("meeting_notes") is not None

    @property
    def configuration(self):
        return self.resolve_configuration(self.registry.selected("meeting_notes"))

    def resolve_configuration(self, selected):
        base = self.fallback.configuration if self.fallback else None
        if selected is None:
            if base is None:
                raise ProcessingError("processing_not_configured")
            return base
        fields = {
            "endpoint": api_root(selected["base_url"]) + "/chat/completions",
            "model": selected["model"],
        }
        if base is not None:
            return base.model_copy(update=fields)
        return ProcessingConfiguration(
            **fields, credential_file=None, allow_insecure_localhost=True
        )

    @asynccontextmanager
    async def session(self):
        # Pin once for the whole note, including all map/reduce parts. A save
        # changes the next note, never the destination halfway through one.
        selected = self.registry.selected("meeting_notes")
        if selected is None:
            if self.fallback is None:
                raise ProcessingError("processing_not_configured")
            yield self.fallback
            return
        client = ProcessingClient(
            self.resolve_configuration(selected),
            credential=SecretStr(selected.get("api_key") or ""),
        )
        try:
            yield client
        finally:
            await client.close()

    async def complete_json(self, **kwargs):
        async with self.session() as client:
            return await client.complete_json(**kwargs)

    async def close(self):
        if self.fallback is not None:
            await self.fallback.close()


def assistant_model(registry, settings):
    """None asks the generic assistant to use its original configured model."""
    if settings.provider != "openai":
        return None
    selected = registry.selected("assistants")
    if selected is None:
        return None
    from pydantic_ai.models.openai import OpenAIChatModel
    from pydantic_ai.providers.openai import OpenAIProvider

    return OpenAIChatModel(
        selected["model"],
        provider=OpenAIProvider(
            base_url=api_root(selected["base_url"]),
            api_key=selected.get("api_key") or "orbit-sin-clave",
        ),
    )


def wire_agents(hub):
    for agent in hub.agents.by_id.values():
        agent.speech = FunctionSpeech(hub.ai, agent.speech)
        if agent.kind == "openai" and hasattr(agent.backend, "model_choice"):
            agent.backend.model_choice = lambda settings=agent.settings: assistant_model(
                hub.ai, settings
            )
