"""Describe the existing operator configuration without changing its consumers."""

import json
from urllib.parse import urlsplit


def endpoint_privacy(function, url, model):
    from .registry import privacy_line

    provider = {"kind": "local", "base_url": url, "name": urlsplit(url).netloc}
    return privacy_line(function, provider, model)


def describe(registry, function):
    settings = registry.settings
    if function == "meeting_transcription":
        return (
            "hub",
            "whisper",
            endpoint_privacy(function, settings.meeting_transcription.endpoint, "whisper"),
        )
    if function in ("meeting_notes", "vault"):
        if function == "vault":
            legacy = vault_model(registry.store)
            if legacy:
                return (
                    "operator",
                    legacy,
                    "Modelo de Notas guardado por el operador; el trabajo del hub aún no se ejecuta.",
                )
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
        descriptions = [("Escucha de Hermes", hermes_destination(settings, "audio"))]
    else:
        descriptions = []
    if not descriptions:
        return unconfigured()
    return (
        "operator",
        " · ".join(dict.fromkeys(d[0] for d in descriptions)),
        " ".join(dict.fromkeys(d[1] for d in descriptions)),
    )


def unconfigured():
    return "operator", "Sin modelo configurado", "No hay un modelo del operador para esta función."


def hermes_destination(settings, material):
    host = urlsplit(settings.hermes_url).netloc
    return f"El {material} se envía a Hermes en {host}; su configuración determina si sale a otro proveedor."


def speech(settings, agent):
    if agent.kind == "hermes":
        return "Voz de Hermes", hermes_destination(settings, "texto")
    if agent.kind == "openai" and agent.settings.speech is not None:
        voice = agent.settings.speech
        return voice.model, endpoint_privacy("speech", voice.base_url, voice.model)
    return None


def vault_model(store):
    with store.read_snapshot() as db:
        if not db.execute(
            "SELECT 1 FROM sqlite_master WHERE name='vault_configuration'"
        ).fetchone():
            return None
        row = db.execute("SELECT body FROM vault_configuration WHERE id=1").fetchone()
        return (
            json.loads(store.decrypt(row[0], "vault-configuration:v1")).get("maintenance_model")
            if row
            else None
        )
