"""Assistants from Orbit: each agent's profile, adding and removing Claude
Code and Codex agents, and signing them in and out (contracts/hub-v1.md,
«Assistants from Orbit»).

Every route uses the device credential, as Settings › Agents does, and every
write answers the whole list as `agents.changed` carries it. What the Mac
sends is a service, a profile and access groups: never a command, a path or
an id the hub did not make.
"""

import shutil
import time
from typing import Literal

from fastapi import Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict

from ..assistant import access
from ..assistant.agent_config import AgentConfigStore
from ..assistant.agents import SERVICE_NAMES, Agents
from ..assistant.roster import Profile
from ..errors import HubError
from ..log import report
from ..notes.http import parse as parse_body

# A profile, a service and five booleans: the longest valid body (24 + 90
# characters, each up to six bytes escaped) is under 1 KiB; 4 KiB leaves the
# envelope room and refuses anything else before it is parsed.
BODY_BYTES = 4 * 1024


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True, hide_input_in_errors=True)


class Groups(Strict):
    screen: bool
    control: bool
    calendar: bool
    meetings: bool
    files: bool
    notes: bool = True


class ProfileBody(Strict):
    name: str
    description: str
    color: str
    primary: bool


class PastedCode(Strict):
    # Checked character by character by `sign_in.PASTED_CODE`; this bounds the body.
    code: str


class NewAgent(Strict):
    # Hermes and `openai` are named so that their refusal says why.
    service: Literal["claude-code", "codex", "hermes", "openai"]
    name: str
    description: str
    color: str
    access: Groups


def _body(model, raw: bytes, what: str):
    return parse_body(raw, model, "invalid_agent_config", what)


async def _stop_processes(runtimes: dict, agent_id: str) -> None:
    """Every device's connection to the agent, and so its processes."""
    for runtime in list(runtimes.values()):
        connection = runtime.connections.pop(agent_id, None)
        if connection is not None:
            await connection.close()


def mount(
    app,
    settings,
    agents: Agents,
    configs: AgentConfigStore,
    runtimes: dict,
    sign_ins,
    identity,
    bounded_body,
    now=time.time,
):
    @app.get("/v1/agents")
    async def listing(request: Request):
        identity(request)
        return JSONResponse(agents.listing())

    @app.put("/v1/agents/{agent_id}/profile")
    async def write_profile(agent_id: str, request: Request):
        identity(request)
        sent = _body(ProfileBody, await bounded_body(request, BODY_BYTES), "Perfil")
        agents.set_profile(
            agent_id, Profile(sent.name.strip(), sent.description.strip(), sent.color), sent.primary
        )
        return JSONResponse(agents.listing())

    @app.post("/v1/agents")
    async def add(request: Request):
        identity(request)
        sent = _body(NewAgent, await bounded_body(request, BODY_BYTES), "Asistente")
        if sent.service not in SERVICE_NAMES:
            raise HubError(
                "agent_add_unsupported",
                "Este tipo se añade en tu hub; Hermes se crea a partir de un perfil de Hermes",
                http_status=403,
                numbers={"service": sent.service},
            )
        agent = agents.add(
            sent.service,
            Profile(sent.name.strip(), sent.description.strip(), sent.color),
            round(now() * 1000),
        )
        configs.set_access(
            agent.id, {group: getattr(sent.access, group) for group in access.GROUPS}
        )
        return JSONResponse({"agent_id": agent.id, **agents.listing()}, status_code=201)

    @app.delete("/v1/agents/{agent_id}")
    async def remove(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        if agent.id not in agents.added:
            agents.remove(agent.id)  # refused with its cause: not removable
        await sign_ins.cancel(agent.id)
        sign_ins.attempts.pop(agent.id, None)
        # Its processes stop before its folder goes.
        await _stop_processes(runtimes, agent.id)
        remove_folder(settings.data_dir / "agents", agent.id)
        agents.remove(agent.id)
        return JSONResponse(agents.listing())

    _mount_sign_in(app, agents, runtimes, sign_ins, identity, bounded_body)


def _mount_sign_in(app, agents: Agents, runtimes: dict, sign_ins, identity, bounded_body):
    """Signing an agent in with its vendor's own tool, and out."""

    @app.post("/v1/agents/{agent_id}/sign-in")
    async def sign_in(agent_id: str, request: Request):
        identity(request)
        return JSONResponse(await sign_ins.start(agent_id))

    @app.get("/v1/agents/{agent_id}/sign-in")
    async def sign_in_status(agent_id: str, request: Request):
        identity(request)
        return JSONResponse(sign_ins.status(agent_id))

    @app.post("/v1/agents/{agent_id}/sign-in/code")
    async def sign_in_code(agent_id: str, request: Request):
        identity(request)
        sent = _body(PastedCode, await bounded_body(request, BODY_BYTES), "Código")
        return JSONResponse(await sign_ins.submit_code(agents.get(agent_id).id, sent.code))

    @app.delete("/v1/agents/{agent_id}/sign-in")
    async def sign_in_cancel(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        cancelled = await sign_ins.cancel(agent.id)
        if cancelled is None:
            return JSONResponse(sign_ins.status(agent.id))  # refused: none to cancel
        return JSONResponse(cancelled)

    @app.post("/v1/agents/{agent_id}/sign-out")
    async def sign_out(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        await sign_ins.sign_out(agent.id)
        # A session open on the old account does not outlive it.
        await _stop_processes(runtimes, agent.id)
        return JSONResponse(agents.listing())


def remove_folder(parent, agent_id: str) -> None:
    """Remove the agent's folder, its account's credentials included. The id
    is the hub's own (`Agents.add`), so the folder is one level under
    `<data_dir>/agents`; a link in its place is unlinked, never followed."""
    folder = parent / agent_id
    try:
        if folder.is_symlink():
            folder.unlink()
        elif folder.exists():
            shutil.rmtree(folder)
    except OSError as exc:
        report("agent_folder_not_removed", "agent_folder_not_removed", exc_info=False)
        raise HubError(
            "agent_folder_not_removed",
            f"No se pudo borrar la carpeta del asistente en el hub ({exc.strerror}); "
            "sigue en la lista. Pídeselo a quien gestiona tu hub",
            http_status=500,
            numbers={"agent_id": agent_id, "errno": exc.errno or 0},
        ) from None
