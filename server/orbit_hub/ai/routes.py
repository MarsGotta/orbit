"""Authenticated provider validation and independent function choices."""

import asyncio
import re

from fastapi import Request
from pydantic import ValidationError

from ..errors import HubError
from ..log import in_store
from .discovery import discover
from .types import OPENROUTER_URL, Choice, Provider

# 64 provider records bounds the encrypted settings document and settings UI;
# independent of the per-provider model count and inference concurrency.
MAX_PROVIDERS = 64
# Credentials escaped in JSON, plus a cloud catalog and manual corrections.
MAX_SETTINGS_BYTES = 1024 * 1024


def refuse_busy(lock):
    if lock.locked():
        raise HubError(
            "ai_settings_busy",
            "Ya hay una comprobación de IA en curso; espera a que termine",
            http_status=429,
            numbers={"in_flight": 1},
        )


def mount(app, registry, identity, bounded_body, executor, changed=lambda: None):
    lock = asyncio.Lock()

    async def payload(request, model):
        try:
            return model.model_validate_json(await bounded_body(request, MAX_SETTINGS_BYTES))
        except ValidationError:
            raise HubError(
                "ai_settings_invalid",
                "Revisa el nombre, la URL, los modelos y la clave del proveedor",
            ) from None

    def active(device):
        if not registry.store.devices.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")

    @app.get("/v1/settings/ai")
    async def read(request: Request):
        identity(request)
        return await in_store(executor, registry.public)

    @app.put("/v1/settings/ai/providers/{provider_id}")
    async def save(provider_id: str, request: Request):
        device = identity(request)
        provider = await payload(request, Provider)
        if re.fullmatch(r"[a-z0-9-]{1,64}", provider_id) is None or provider_id in (
            "hub",
            "operator",
        ):
            raise HubError("ai_provider_invalid", "Identificador de proveedor no válido")
        if (provider.kind == "openrouter") != (provider_id == "openrouter"):
            raise HubError("ai_provider_invalid", "OpenRouter usa su propio registro de proveedor")
        if provider.kind == "openrouter" and provider.base_url != OPENROUTER_URL:
            raise HubError("ai_provider_invalid", "La URL de OpenRouter no se puede cambiar")
        refuse_busy(lock)
        async with lock:
            active(device)
            previous = await in_store(executor, registry.read)
            records = previous["providers"]
            if provider_id not in records and len(records) >= MAX_PROVIDERS:
                raise HubError(
                    "ai_providers_full",
                    f"Hay {len(records)} proveedores; el máximo es {MAX_PROVIDERS}",
                    numbers={"count": len(records), "max_count": MAX_PROVIDERS},
                )
            previous_provider = records.get(provider_id, {})
            # A paired Mac cannot redirect a hub-only credential to a new URL.
            # Preserving a key requires the same validated destination.
            kept = (
                previous_provider.get("api_key")
                if previous_provider.get("base_url") == provider.base_url
                else None
            )
            key = provider.api_key or kept
            if provider.kind == "openrouter" and not key:
                raise HubError("ai_key_missing", "Introduce la clave de OpenRouter")
            detected = await discover(provider, key)
            models = (
                [m.model_dump() for m in provider.models]
                if provider.models is not None
                else detected
            )
            if provider.models is not None and {m["id"] for m in models} != {
                m["id"] for m in detected
            }:
                raise HubError(
                    "ai_models_invalid", "Corrige sólo los modelos que devolvió el proveedor"
                )
            active(device)
            await in_store(executor, registry.save_provider, provider_id, provider, models, key)
            changed()
            return await in_store(executor, registry.public)

    @app.put("/v1/settings/ai/functions/{function}")
    async def choose(function: str, request: Request):
        device = identity(request)
        choice = await payload(request, Choice)
        refuse_busy(lock)
        async with lock:
            active(device)
            await in_store(executor, registry.choose, function, choice)
            changed()
            return await in_store(executor, registry.public)
