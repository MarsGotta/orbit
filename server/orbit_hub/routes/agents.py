"""Settings › Agents: each agent's personality, skills and access groups.

The Mac reads and changes them with its device credential, like the meeting
dictionaries (contracts/hub-v1.md, «Agent configuration»). Every write answers
with the agent's whole configuration, as the read does. Neither errors nor
logs carry a personality or a skill's text.
"""

from fastapi import Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict

from ..assistant.agent_config import (
    MAX_PERSONALITY_BYTES,
    AgentConfigStore,
    personality_refusal,
)
from ..assistant.agents import KINDS, Agents
from ..assistant.skills import MAX_SKILL_BYTES, MAX_SKILLS, check_name, parse, size_refusal
from ..assistant.workspaces import MAX_WORKING_FOLDER_BYTES, WorkspacePolicy
from ..errors import HubError
from ..notes.http import parse as parse_body

# A request body's ceiling is its text's, escaped as JSON: a control character
# is six bytes (`\u0000`), so six times the text's bound plus 1 KiB of
# envelope holds every valid request. What is larger is refused as `too_large`
# before it is parsed; what fits is measured and refused with the field's own
# code and numbers.
PERSONALITY_BODY_BYTES = 6 * MAX_PERSONALITY_BYTES + 1024
SKILL_BODY_BYTES = 6 * MAX_SKILL_BYTES + 1024
# Access-group booleans or one: far under 1 KiB.
SMALL_BODY_BYTES = 1024


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True, hide_input_in_errors=True)


class Personality(Strict):
    text: str


class WorkingFolder(Strict):
    path: str


class Groups(Strict):
    screen: bool
    control: bool
    calendar: bool
    meetings: bool
    files: bool
    notes: bool = True


class Access(Strict):
    groups: Groups


class SkillFile(Strict):
    content: str


class Enabled(Strict):
    enabled: bool


def mount(app, settings, agents: Agents, configs: AgentConfigStore, identity, bounded_body):
    def admin(agent):
        return KINDS[agent.kind].admin(settings, agent, configs)

    async def configuration(agent) -> dict:
        kept = admin(agent)
        personality = await kept.personality()
        items = await kept.skills()
        return {
            "agent_id": agent.id,
            "kind": agent.kind,
            **working_folder_config(settings, agent, configs),
            # Every kind lets Orbit change all three today; a field a kind did
            # not would say `editable: false` and its `cause`.
            "personality": {
                "text": personality,
                "editable": True,
                "max_bytes": MAX_PERSONALITY_BYTES,
            },
            "access": {"groups": configs.access(agent.id), "editable": True},
            "skills": {
                "editable": True,
                "max_skills": MAX_SKILLS,
                "max_skill_bytes": MAX_SKILL_BYTES,
                "items": [entry.public() for entry in items],
            },
        }

    def body(model, raw: bytes, what: str):
        return parse_body(raw, model, "invalid_agent_config", what)

    @app.get("/v1/agents/{agent_id}/config")
    async def read_configuration(agent_id: str, request: Request):
        identity(request)
        return JSONResponse(await configuration(agents.get(agent_id)))

    mount_workspaces(app, settings, agents, configs, identity, bounded_body, configuration)

    @app.put("/v1/agents/{agent_id}/personality")
    async def write_personality(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        text = body(
            Personality, await bounded_body(request, PERSONALITY_BODY_BYTES), "Personalidad"
        ).text
        if (refusal := personality_refusal(text)) is not None:
            raise refusal
        await admin(agent).set_personality(text)
        return JSONResponse(await configuration(agent))

    @app.put("/v1/agents/{agent_id}/access")
    async def write_access(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        groups = body(Access, await bounded_body(request, SMALL_BODY_BYTES), "Acceso").groups
        # An older Mac does not know `notes`; preserve its stored choice.
        configs.set_access(
            agent.id, {**configs.access(agent.id), **groups.model_dump(exclude_unset=True)}
        )
        return JSONResponse(await configuration(agent))

    @app.get("/v1/agents/{agent_id}/skills/{name}")
    async def read_skill(agent_id: str, name: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        entry, content = await admin(agent).skill(name)
        return JSONResponse({**entry.public(), "content": content})

    @app.put("/v1/agents/{agent_id}/skills/{name}/enabled")
    async def write_enabled(agent_id: str, name: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        enabled = body(Enabled, await bounded_body(request, SMALL_BODY_BYTES), "Skill").enabled
        await admin(agent).set_skill_enabled(name, enabled)
        return JSONResponse(await configuration(agent))

    # The Mac sends the name its file declares as one encoded segment, and the
    # router decodes `%2F`: `:path` takes a name with a slash, so it is refused
    # with the rule it breaks, not answered with an uncoded 404 that reads as
    # a hub without agents. Registered after `…/enabled`, which it would match.
    @app.put("/v1/agents/{agent_id}/skills/{name:path}")
    async def write_skill(agent_id: str, name: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        check_name(name)
        content = body(SkillFile, await bounded_body(request, SKILL_BODY_BYTES), "Skill").content
        if (refusal := size_refusal(name, content)) is not None:
            raise refusal
        await admin(agent).put_skill(parse(name, content))
        return JSONResponse(await configuration(agent))

    @app.delete("/v1/agents/{agent_id}/skills/{name}")
    async def delete_skill(agent_id: str, name: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        await admin(agent).delete_skill(name)
        return JSONResponse(await configuration(agent))


def mount_workspaces(app, settings, agents, configs, identity, bounded_body, configuration):
    @app.put("/v1/agents/{agent_id}/working-folder")
    async def write_working_folder(agent_id: str, request: Request):
        identity(request)
        agent = agents.get(agent_id)
        if agent.kind not in {"claude-code", "codex"}:
            raise HubError(
                "invalid_working_folder",
                "Este asistente no usa una carpeta de trabajo del hub",
                http_status=400,
                numbers={"cause": "este asistente no usa una carpeta de trabajo del hub"},
            )
        path = parse_body(
            await bounded_body(request, 6 * MAX_WORKING_FOLDER_BYTES + 1024),
            WorkingFolder,
            "invalid_agent_config",
            "Carpeta de trabajo",
        ).path
        validated = WorkspacePolicy(settings, agent.id, configs).validate(path)
        configs.set_working_folder(agent.id, str(validated))
        return JSONResponse(await configuration(agent))


def working_folder_config(settings, agent, configs):
    if agent.kind not in {"claude-code", "codex"}:
        return {}
    return {"working_folder": WorkspacePolicy(settings, agent.id, configs).configured()}
