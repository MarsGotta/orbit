"""Describe the existing operator configuration without changing its consumers."""

from urllib.parse import urlsplit

from .privacy import combine, describe_privacy, sentence


def endpoint_privacy(function, url, model):
    provider = {"kind": "local", "base_url": url, "name": urlsplit(url).netloc}
    return describe_privacy(function, provider, model)


def describe(registry, function):
    settings = registry.settings
    if function == "meeting_transcription":
        # The hub's transcriber is the operator's `[meeting_transcription]`,
        # read by the name the operator gave it. Choosing it explicitly is
        # still `hub`/`whisper` on the wire (`Registry.hub_provider`).
        transcription = settings.meeting_transcription
        return (
            "operator",
            transcription.model,
            endpoint_privacy(function, transcription.endpoint, transcription.model),
        )
    if function in ("meeting_notes", "vault"):
        if settings.processing is not None:
            return (
                "operator",
                settings.processing.model,
                endpoint_privacy(function, settings.processing.endpoint, settings.processing.model),
            )
        return unconfigured()
    agents, default_id = settings.agent_configurations()
    default = next(a for a in agents if a.id == default_id)
    if function == "assistants":
        descriptions = [
            (a.settings.model, endpoint_privacy(function, a.settings.base_url, a.settings.model))
            for a in agents
            if a.kind == "openai" and a.settings.provider == "openai"
        ]
    elif function == "speech":
        descriptions = [speech(settings, a) for a in agents]
        descriptions = [d for d in descriptions if d is not None]
    elif default.kind == "openai":
        descriptions = [
            (
                settings.meeting_transcription.model,
                endpoint_privacy(function, settings.meeting_transcription.endpoint, ""),
            )
        ]
    elif default.kind == "hermes":
        descriptions = [("Hermes listening", hermes_destination(settings, "audio"))]
    else:
        descriptions = []
    if not descriptions:
        return unconfigured()
    return (
        "operator",
        " · ".join(dict.fromkeys(d[0] for d in descriptions)),
        combine(d[1] for d in descriptions),
    )


def unconfigured():
    return "operator", "No model configured", sentence("unconfigured")


def hermes_destination(settings, material):
    host = urlsplit(settings.hermes_url).netloc
    return sentence("hermes", material=material, destination=host)


def speech(settings, agent):
    if agent.kind == "hermes":
        return "Hermes speech", hermes_destination(settings, "text")
    if agent.kind == "openai" and agent.settings.speech is not None:
        voice = agent.settings.speech
        return voice.model, endpoint_privacy("speech", voice.base_url, voice.model)
    return None
