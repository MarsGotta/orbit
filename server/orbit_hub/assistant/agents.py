"""The agents a hub offers, each one an `AssistantBackend` of its kind.

`orbit.toml` lists them as `[[agents]]`, each with an `id` (stable, what
sessions store), a `name` (what the Mac shows and what the user says: «Oye
Lyra») and a `kind`, plus the kind's own keys. The first entry is the default
agent unless `default_agent` names another. Without `[[agents]]` the hub is
one implicit agent, `default`, made of `assistant_name` and `profile` as
before agents existed, so a 0.9.0 `orbit.toml` loads as it did.

Adding a kind is one entry in `KINDS`, an `AgentKind` with four functions:

    KINDS = {
        "hermes": AgentKind(hermes.parse_agent, hermes.agent_backend, hermes.agent_speech, …),
        "openai": AgentKind(openai.parse_settings, openai.build_backend, openai.build_speech, …),
    }

- `parse(keys: dict[str, Any]) -> S` receives the entry's own keys (all but
  `id`, `name` and `kind`) as TOML read them and returns the kind's settings,
  any frozen value. It raises `ValueError` naming the key and the cause
  (missing, unknown, invalid); the message is prefixed with the entry. It
  must refuse keys it does not know: a misspelt key silently ignored would
  configure a different agent than the one written. It runs while the
  configuration loads, so it reads no files and opens nothing.
- `backend(settings: Settings, agent: AgentConfiguration, instructions) -> AssistantBackend`
  builds the agent's backend from the hub's settings and the configured agent
  (its `id`, `name` and what `parse` returned as `agent.settings`), once at
  startup; every device's connections to that agent come from it. Files the
  kind names (keys, commands) are checked here, refused with `ValueError`.
  `instructions()` reads the personality and enabled skills the hub keeps for
  the agent (`agent_config.AgentInstructions`) when the kind takes them from
  the hub; a kind that keeps its own (Hermes) ignores it.
- `speech(settings: Settings, agent: AgentConfiguration) -> SpeechBackend | None`
  is the agent's dictation and read-aloud, or None for a kind without a voice;
  its backend's `capabilities.voice` then says False.
- `admin(settings: Settings, agent, configs: AgentConfigStore) -> AgentAdmin`
  is where the agent's personality and skills are read and changed from
  Orbit (`agent_config.AgentAdmin`): the hub's database, or the kind's own
  store. Access groups are the hub's for every kind.

Everything else (ids, names, the default, the refusals of `chat.send`, the
`ready` list) is the same for every kind and lives here.
"""

from __future__ import annotations

import json
import re
import unicodedata
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from ..errors import HubError
from . import acp, hermes, openai
from .agent_config import (
    AgentAdmin,
    AgentConfigStore,
    AgentInstructions,
    StoredAdmin,
    no_instructions,
)
from .backend import AssistantBackend
from .hermes_panel import HermesPanelAdmin
from .speech import SpeechBackend

if TYPE_CHECKING:
    from ..config import Settings


InstructionSource = Callable[[], AgentInstructions]


@dataclass(frozen=True)
class AgentKind:
    parse: Callable[[dict[str, Any]], Any]
    backend: Callable[[Settings, AgentConfiguration, InstructionSource], AssistantBackend]
    speech: Callable[[Settings, AgentConfiguration], SpeechBackend | None]
    admin: Callable[[Settings, Any, AgentConfigStore], AgentAdmin]


def _no_speech(settings: Settings, agent: AgentConfiguration) -> None:
    return None


def _stored_admin(settings: Settings, agent: Any, configs: AgentConfigStore) -> AgentAdmin:
    return StoredAdmin(configs, agent.id)


def _hermes_admin(settings: Settings, agent: Any, configs: AgentConfigStore) -> AgentAdmin:
    return HermesPanelAdmin(settings, agent.id, agent.settings.profile)


KINDS: dict[str, AgentKind] = {
    # Hermes keeps its own SOUL.md and skills: its panel is where Orbit edits them.
    "hermes": AgentKind(
        hermes.parse_agent, hermes.agent_backend, hermes.agent_speech, _hermes_admin
    ),
    "openai": AgentKind(
        openai.parse_settings, openai.build_backend, openai.build_speech, _stored_admin
    ),
    # Claude Code and Codex speak no audio over ACP: no dictation, no read-aloud.
    "acp": AgentKind(acp.parse_settings, acp.build_backend, _no_speech, _stored_admin),
}

# The id of the one agent a configuration without `[[agents]]` has. Sessions
# opened with it store it; sessions stored before agents existed store none.
IMPLICIT_AGENT_ID = "default"
# What a session stores and the Mac keeps for each agent; 32 is long enough
# for a readable slug and bounds what every listed session repeats.
AGENT_ID = re.compile(r"[a-z0-9-]{1,32}")
# Bound the spoken invocation and UI label to a short name (48 Unicode
# scalars, including spaces), independently of prompts and profile IDs.
MAX_AGENT_NAME_CHARS = 48


def name_refusal(name: str) -> str | None:
    """Why `name` cannot be an agent's name, with its count and the ceiling."""
    count = len(name)
    if any(unicodedata.category(character) == "Cc" for character in name):
        cause = "contains control characters"
    elif not name.strip():
        cause = "is empty"
    elif count > MAX_AGENT_NAME_CHARS:
        cause = "is too long"
    else:
        return None
    return f"{cause}: {count} characters; maximum {MAX_AGENT_NAME_CHARS}"


def spoken(name: str) -> str:
    """The name as it is said aloud: without case, accents or extra spaces."""
    decomposed = unicodedata.normalize("NFKD", name)
    bare = "".join(character for character in decomposed if not unicodedata.combining(character))
    return " ".join(bare.casefold().split())


@dataclass(frozen=True)
class AgentConfiguration:
    id: str
    name: str
    kind: str
    # What the kind's `parse` made of the entry's own keys.
    settings: Any


def configured_agents(
    entries: Sequence[dict[str, Any]] | None,
    default_agent: str | None,
    *,
    assistant_name: str,
    profile: str,
) -> tuple[tuple[AgentConfiguration, ...], str]:
    """The agents a configuration describes, in order, and the default's id.

    Raises `ValueError` naming the entry and the cause of the first refusal.
    """
    if entries is None:
        cause = name_refusal(assistant_name)
        if cause is not None:
            raise ValueError(f"assistant_name {cause}")
        agents = (
            AgentConfiguration(
                IMPLICIT_AGENT_ID,
                assistant_name,
                "hermes",
                hermes.parse_agent({"profile": profile}),
            ),
        )
    elif not entries:
        raise ValueError(
            "agents is empty: list at least one [[agents]], or remove it for the implicit agent"
        )
    else:
        agents = _entries(entries)
    ids = [agent.id for agent in agents]
    if default_agent is not None and default_agent not in ids:
        raise ValueError(
            f"default_agent {_shown(default_agent)} is not an agent; "
            f"the agents are {', '.join(ids)}"
        )
    return agents, default_agent or ids[0]


def _entries(entries: Sequence[dict[str, Any]]) -> tuple[AgentConfiguration, ...]:
    agents: list[AgentConfiguration] = []
    ids: dict[str, int] = {}
    names: dict[str, tuple[int, str]] = {}
    for number, entry in enumerate(entries, 1):
        where = f"[[agents]] entry {number}"
        keys = dict(entry)
        agent_id = keys.pop("id", None)
        if agent_id is None:
            raise ValueError(f"{where}: missing id")
        if not isinstance(agent_id, str) or AGENT_ID.fullmatch(agent_id) is None:
            raise ValueError(
                f"{where}: id {_shown(agent_id)} must be 1 to 32 characters of a-z, 0-9 and -"
            )
        where += f' (id "{agent_id}")'
        if agent_id in ids:
            raise ValueError(f"{where}: duplicate id; entry {ids[agent_id]} already uses it")
        name = keys.pop("name", None)
        if name is None:
            raise ValueError(f"{where}: missing name")
        cause = name_refusal(name) if isinstance(name, str) else "must be text"
        if cause is not None:
            raise ValueError(f"{where}: name {cause}")
        said = spoken(name)
        if said in names:
            other, other_name = names[said]
            raise ValueError(
                f"{where}: duplicate name {_shown(name)}: entry {other} ({_shown(other_name)}) "
                "is said the same, ignoring case and accents"
            )
        kind = keys.pop("kind", None)
        if kind is None:
            raise ValueError(f"{where}: missing kind")
        if kind not in KINDS:
            raise ValueError(
                f"{where}: unknown kind {_shown(kind)}; known kinds: {', '.join(sorted(KINDS))}"
            )
        try:
            settings = KINDS[kind].parse(keys)
        except ValueError as exc:
            raise ValueError(f"{where}: {exc}") from None
        ids[agent_id], names[said] = number, (number, name)
        agents.append(AgentConfiguration(agent_id, name, kind, settings))
    return tuple(agents)


def _shown(value: object) -> str:
    return json.dumps(value, ensure_ascii=False)


@dataclass(frozen=True)
class Agent:
    id: str
    name: str
    kind: str
    settings: Any
    backend: AssistantBackend
    # Its dictation and read-aloud; None for a kind without a voice.
    speech: SpeechBackend | None = None

    def voice(self) -> SpeechBackend:
        """The agent's speech, or the refusal that names the agent and why it has none."""
        if self.speech is None:
            raise HubError(
                "speech_not_configured",
                f"{self.name} («{self.id}») no tiene voz: los agentes de tipo {self.kind} "
                "no dictan ni leen en voz alta",
                http_status=503,
                numbers={"agent_id": self.id},
            )
        return self.speech

    def public(self) -> dict:
        """The agent as `ready` lists it to the Mac."""
        capabilities = self.backend.capabilities
        return {
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
            # A list of names, as contracts/hub-v1.md fixes and the Mac decodes
            # (`HubAgent.capabilities: Vec<String>`): an object would make the
            # Mac refuse the whole `ready`.
            "capabilities": [
                name
                for name, accepted in (
                    ("images", capabilities.images),
                    ("voice", capabilities.voice),
                    ("resume", capabilities.resume),
                )
                if accepted
            ],
        }


class Agents:
    """The hub's agents, in configured order, and the default one."""

    def __init__(self, agents: Sequence[Agent], default_id: str):
        self.by_id = {agent.id: agent for agent in agents}
        self.default = self.by_id[default_id]

    @classmethod
    def from_settings(
        cls,
        settings: Settings,
        backend: AssistantBackend | None = None,
        speech: SpeechBackend | None = None,
        configs: AgentConfigStore | None = None,
    ) -> Agents:
        """The configured agents, each on its kind's backend and speech, or all
        on `backend` and `speech` when they are given: tests drive the hub
        with doubles. `configs` holds what the hub keeps of each agent's
        personality and skills; without it, an agent has none."""
        configured, default_id = settings.agent_configurations()

        def instructions(agent_id: str) -> InstructionSource:
            if configs is None:
                return no_instructions
            return lambda: configs.instructions(agent_id)

        return cls(
            [
                Agent(
                    agent.id,
                    agent.name,
                    agent.kind,
                    agent.settings,
                    backend
                    if backend is not None
                    else KINDS[agent.kind].backend(settings, agent, instructions(agent.id)),
                    speech if speech is not None else KINDS[agent.kind].speech(settings, agent),
                )
                for agent in configured
            ],
            default_id,
        )

    def get(self, agent_id: str) -> Agent:
        agent = self.by_id.get(agent_id)
        if agent is None:
            ids = list(self.by_id)
            raise HubError(
                "agent_not_found",
                f"Este hub no tiene el agente «{agent_id}»; tiene: {', '.join(ids)}",
                # Comma-joined: the Mac decodes refusal numbers as integers,
                # strings or booleans only, and an array would refuse the frame.
                numbers={"agent_id": agent_id, "agent_ids": ",".join(ids)},
            )
        return agent

    def owner(self, session: dict) -> str:
        """The id of the agent a stored session belongs to: the one it names,
        or the default agent for a session stored before agents existed."""
        return session["agent_id"] or self.default.id

    def public(self) -> list[dict]:
        return [agent.public() for agent in self.by_id.values()]
