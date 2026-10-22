"""A `hermes` agent's personality and skills, through Hermes's own panel API.

The personality is the profile's `SOUL.md` and the skills are the profile's,
read and written over the REST routes Hermes's dashboard uses, with the
credential the hub already holds for Hermes (`X-Hermes-Session-Token`):

    GET  /api/profiles/{name}/soul        {content, exists}
    PUT  /api/profiles/{name}/soul        {content}
    GET  /api/skills?profile=             [{name, description, enabled, …}]
    GET  /api/skills/content?name=&profile=   {name, content}
    POST /api/skills                      {name, content, profile}   (create)
    PUT  /api/skills/content              {name, content, profile}   (replace)
    PUT  /api/skills/toggle               {name, enabled, profile}

Those routes carry no version and no promise of stability, so they are checked
against the installed Hermes at each release (the runbook says how); in
development only a fake Hermes panel answers them. Hermes's panel has no route
to delete a skill: Orbit refuses that for a `hermes` agent and says so.

A route Hermes answers with an error is `assistant_admin_refused {route,
status}`: the route as written above, never the profile or skill in it.
"""

from __future__ import annotations

import json
from typing import TYPE_CHECKING
from urllib.parse import quote

import httpx

from ..log import report
from ..protocol import HubError
from .agent_config import not_found, too_many
from .hermes import credential
from .skills import MAX_SKILLS, Skill, SkillEntry

if TYPE_CHECKING:
    from ..config import Settings

# Hermes's panel writes a file and answers. 10 s to connect, as every other
# call of the hub to Hermes; 30 s for its answer, which protects the Mac's
# request (it gives up after 90 s) from a Hermes that stops answering. Not
# measured against a real Hermes; neither bounds anything else.
PANEL_CONNECT_TIMEOUT_SECONDS = 10
PANEL_TIMEOUT_SECONDS = 30
# One answer of the panel. What it protects: the hub's memory and the Mac's
# configuration read. Why 1 MiB: a SKILL.md is at most `MAX_SKILL_BYTES`
# (64 KiB) and a listing of `MAX_SKILLS` (128) skills at the standard's maxima
# about 140 KiB; the panel's own fields beside them fit many times over.
MAX_PANEL_RESPONSE_BYTES = 1024 * 1024

SOUL = "/api/profiles/{name}/soul"
SKILLS = "/api/skills"
SKILL_CONTENT = "/api/skills/content"
SKILL_TOGGLE = "/api/skills/toggle"


class HermesPanelAdmin:
    """The Hermes profile `profile`'s SOUL.md and skills."""

    def __init__(self, settings: Settings, agent_id: str, profile: str):
        self.settings, self.agent_id, self.profile = settings, agent_id, profile

    async def personality(self) -> str:
        answer = await self._call("GET", SOUL)
        content = answer.get("content") if isinstance(answer, dict) else None
        if not isinstance(content, str):
            raise _unreadable("GET", SOUL)
        return content

    async def set_personality(self, text: str) -> None:
        await self._call("PUT", SOUL, payload={"content": text})

    async def skills(self) -> list[SkillEntry]:
        answer = await self._call("GET", SKILLS, params={"profile": self.profile})
        if not isinstance(answer, list):
            raise _unreadable("GET", SKILLS)
        entries = []
        for item in answer:
            if not isinstance(item, dict):
                raise _unreadable("GET", SKILLS)
            name, description = item.get("name"), item.get("description")
            enabled = item.get("enabled", True)
            if not isinstance(name, str) or not isinstance(enabled, bool):
                raise _unreadable("GET", SKILLS)
            entries.append(
                SkillEntry(name, description if isinstance(description, str) else "", enabled)
            )
        return sorted(entries, key=lambda entry: entry.name)

    async def skill(self, name: str) -> tuple[SkillEntry, str]:
        entry = await self._entry(name)
        answer = await self._call(
            "GET", SKILL_CONTENT, params={"name": name, "profile": self.profile}
        )
        content = answer.get("content") if isinstance(answer, dict) else None
        if not isinstance(content, str):
            raise _unreadable("GET", SKILL_CONTENT)
        return entry, content

    async def put_skill(self, skill: Skill) -> None:
        listed = await self.skills()
        body = {"name": skill.name, "content": skill.content, "profile": self.profile}
        if any(entry.name == skill.name for entry in listed):
            await self._call("PUT", SKILL_CONTENT, payload=body)
            return
        if len(listed) >= MAX_SKILLS:
            raise too_many(len(listed))
        await self._call("POST", SKILLS, params={"profile": self.profile}, payload=body)

    async def set_skill_enabled(self, name: str, enabled: bool) -> None:
        await self._entry(name)
        await self._call(
            "PUT", SKILL_TOGGLE, payload={"name": name, "enabled": enabled, "profile": self.profile}
        )

    async def delete_skill(self, name: str) -> None:
        raise HubError(
            "field_not_editable",
            "El panel de Hermes no tiene forma de borrar una skill: desactívala aquí o bórrala "
            "en el servidor de Hermes",
            http_status=403,
            numbers={"agent_id": self.agent_id, "field": "skills", "kind": "hermes"},
        )

    async def _entry(self, name: str) -> SkillEntry:
        for entry in await self.skills():
            if entry.name == name:
                return entry
        raise not_found(name)

    async def _call(self, method: str, route: str, *, params=None, payload=None):
        """One panel call: its JSON answer, or the refusal that names the route."""
        url = self.settings.hermes_url.rstrip("/") + route.replace(
            "{name}", quote(self.profile, safe="")
        )
        timeout = httpx.Timeout(PANEL_TIMEOUT_SECONDS, connect=PANEL_CONNECT_TIMEOUT_SECONDS)
        try:
            async with (
                httpx.AsyncClient(timeout=timeout, follow_redirects=False) as client,
                client.stream(
                    method,
                    url,
                    params=params,
                    json=payload,
                    headers={"X-Hermes-Session-Token": credential(self.settings)},
                ) as response,
            ):
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(body) + len(chunk) > MAX_PANEL_RESPONSE_BYTES:
                        raise HubError(
                            "assistant_admin_too_large",
                            f"Hermes contestó a {method} {route} con más de "
                            f"{MAX_PANEL_RESPONSE_BYTES} bytes",
                            http_status=502,
                            numbers={
                                "route": f"{method} {route}",
                                "bytes": len(body) + len(chunk),
                                "max_bytes": MAX_PANEL_RESPONSE_BYTES,
                            },
                        )
                    body.extend(chunk)
                status = response.status_code
        except HubError:
            raise
        except httpx.TimeoutException as exc:
            report(event="hermes_panel_failed", code="assistant_admin_timeout")
            connecting = isinstance(exc, httpx.ConnectTimeout)
            seconds = PANEL_CONNECT_TIMEOUT_SECONDS if connecting else PANEL_TIMEOUT_SECONDS
            raise HubError(
                "assistant_admin_timeout",
                f"Hermes no {'aceptó la conexión' if connecting else 'contestó'} a "
                f"{method} {route} en {seconds * 1000} ms",
                http_status=504,
                numbers={"route": f"{method} {route}", "timeout_ms": seconds * 1000},
            ) from exc
        except httpx.HTTPError as exc:
            report(event="hermes_panel_failed", code="assistant_admin_unreachable")
            raise HubError(
                "assistant_admin_unreachable",
                f"No se pudo conectar con Hermes para {method} {route}",
                http_status=503,
                numbers={"route": f"{method} {route}"},
            ) from exc
        if not 200 <= status < 300:
            # Hermes's own detail can quote the profile's files: never relayed.
            report(event="hermes_panel_refused", code="assistant_admin_refused", status=status)
            raise HubError(
                "assistant_admin_refused",
                f"Hermes rechazó {method} {route} con HTTP {status}",
                http_status=502,
                numbers={"route": f"{method} {route}", "status": status},
            )
        try:
            return json.loads(bytes(body))
        except ValueError:
            raise _unreadable(method, route) from None


def _unreadable(method: str, route: str) -> HubError:
    return HubError(
        "assistant_admin_invalid",
        f"Hermes contestó a {method} {route} algo que el hub no sabe leer",
        http_status=502,
        numbers={"route": f"{method} {route}"},
    )
