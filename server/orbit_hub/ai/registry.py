"""One encrypted provider registry, shared by paired Macs and all consumers."""

import json
from urllib.parse import urlsplit

from ..errors import HubError
from ..urls import LOOPBACK_HOSTS
from .types import FUNCTIONS, MAI, OPENROUTER_URL

SCHEMA = """
CREATE TABLE IF NOT EXISTS ai_settings (
    id INTEGER PRIMARY KEY CHECK(id=1), body BLOB NOT NULL);
"""
CONTEXT = "hub-ai-settings"
# Protect the encrypted settings allocation and leave headroom for operator
# descriptions in the existing Mac response envelope. Does not bound inference.
MAX_REGISTRY_BYTES = 512 * 1024


def privacy_line(function, provider, model):
    material = "audio" if function in ("meeting_transcription", "listening") else "texto"
    if provider["kind"] == "openrouter":
        destination = (
            "Microsoft a través de OpenRouter"
            if model == MAI
            else "OpenRouter y el proveedor del modelo"
        )
        return f"El {material} sale a {destination}."
    url = provider.get("base_url")
    if url and urlsplit(url).hostname not in LOOPBACK_HOSTS:
        return f"El {material} sale a {provider['name']}, fuera de tu hub, por tu red o Internet."
    if url or provider["kind"] == "hub":
        return f"El {material} se queda en tu hub."
    return "Se usa la configuración del operador; el servicio puede enviar datos fuera del hub."


class Registry:
    def __init__(self, store, settings=None):
        self.store, self.settings = store, settings

    def read(self):
        with self.store.read_snapshot() as db:
            row = db.execute("SELECT body FROM ai_settings WHERE id=1").fetchone()
            return (
                json.loads(self.store.decrypt(row[0], CONTEXT))
                if row
                else {"providers": {}, "functions": {}}
            )

    def write(self, document):
        body = json.dumps(document).encode()
        if len(body) > MAX_REGISTRY_BYTES:
            raise HubError(
                "ai_registry_full",
                f"La configuración ocupa {len(body)} bytes; el máximo es {MAX_REGISTRY_BYTES}",
                numbers={"bytes": len(body), "max_bytes": MAX_REGISTRY_BYTES},
            )
        sealed = self.store.encrypt(body, CONTEXT)
        with self.store.transaction() as db:
            self.store.ensure_capacity(len(sealed))
            db.execute(
                "INSERT INTO ai_settings VALUES (1,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
                (sealed,),
            )

    def migrate(self):
        """At hub load, migrate once. The old row stays readable by older Macs."""
        with self.store.read_snapshot() as db:
            if db.execute("SELECT 1 FROM ai_settings WHERE id=1").fetchone():
                return
            row = db.execute("SELECT body FROM transcription_preferences WHERE id=1").fetchone()
            if row is None:
                # Starting an unconfigured hub is read-only, including when
                # its disk/quota refuses new content. Defaults need no row.
                return
            previous = json.loads(self.store.decrypt(row[0], "hub-transcription-preference"))
        self.legacy(previous.get("model", "whisper"), previous.get("api_key"))

    def legacy(self, model, key):
        document = self.read()
        if key or model == "mai":
            provider = document["providers"].get(
                "openrouter",
                {
                    "id": "openrouter",
                    "name": "OpenRouter",
                    "kind": "openrouter",
                    "base_url": OPENROUTER_URL,
                    "models": [{"id": MAI, "capabilities": ["transcription", "word_timestamps"]}],
                },
            )
            provider["api_key"] = key
            document["providers"]["openrouter"] = provider
        choice = (
            {"provider_id": "openrouter", "model": MAI}
            if model == "mai"
            else {"provider_id": "hub", "model": "whisper"}
        )
        for function in ("meeting_transcription", "listening"):
            if model == "mai":
                document["functions"][function] = choice
            else:
                document["functions"].pop(function, None)
        self.write(document)

    def selected(self, function):
        doc = self.read()
        choice = doc["functions"].get(function)
        if not choice or choice["provider_id"] in ("hub", "operator"):
            return None
        provider = doc["providers"].get(choice["provider_id"])
        if provider is None:
            raise HubError("ai_provider_missing", "El proveedor elegido ya no está configurado")
        return {**provider, "model": choice["model"]}

    def save_provider(self, provider_id, provider, models, key):
        document = self.read()
        entry = {
            **provider.model_dump(exclude={"models", "api_key"}),
            "id": provider_id,
            "models": models,
            "api_key": key,
        }
        # Never invalidate an in-use selection when correcting capabilities.
        for function, choice in document["functions"].items():
            if choice["provider_id"] == provider_id:
                self.check(function, choice["model"], entry)
        document["providers"][provider_id] = entry
        self.write(document)

    @staticmethod
    def check(function, model, provider):
        if not any(
            m["id"] == model and FUNCTIONS[function] in m["capabilities"]
            for m in provider["models"]
        ):
            raise HubError(
                "ai_capability_missing", "El modelo no tiene la capacidad que necesita esta función"
            )

    def choose(self, function, choice):
        if function not in FUNCTIONS:
            raise HubError("ai_function_invalid", "La función de IA no existe")
        document = self.read()
        if choice.provider_id == "operator":
            document["functions"].pop(function, None)
        else:
            provider = (
                self.hub_provider()
                if choice.provider_id == "hub"
                else document["providers"].get(choice.provider_id)
            )
            if provider is None:
                raise HubError("ai_provider_missing", "El proveedor elegido no existe")
            self.check(function, choice.model, provider)
            document["functions"][function] = choice.model_dump()
        self.write(document)

    def hub_provider(self):
        return {
            "id": "hub",
            "name": "En tu hub",
            "kind": "hub",
            "base_url": None,
            "key_configured": False,
            "models": [
                {"id": "whisper", "capabilities": ["transcription", "word_timestamps"]},
                {"id": "pyannote", "capabilities": []},
            ],
        }

    def default(self, function):
        from .defaults import describe

        return describe(self, function)

    def public(self):
        document = self.read()
        providers = [self.hub_provider()] + [
            {k: v for k, v in p.items() if k != "api_key"}
            | {"key_configured": bool(p.get("api_key"))}
            for p in document["providers"].values()
        ]
        functions = []
        for function, capability in FUNCTIONS.items():
            choice = document["functions"].get(function)
            if choice:
                pid, model = choice["provider_id"], choice["model"]
                provider = next(p for p in providers if p["id"] == pid)
                privacy = privacy_line(function, provider, model)
                if pid == "hub":
                    privacy = self.default("meeting_transcription")[2]
            else:
                pid, model, privacy = self.default(function)
            functions.append(
                {
                    "id": function,
                    "provider_id": pid,
                    "model": model,
                    "capability": capability,
                    "privacy": privacy,
                }
            )
        return {"providers": providers, "functions": functions}
