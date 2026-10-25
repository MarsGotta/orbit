"""The kinds of agent a hub offers, and the agents its configuration lists.

`orbit.toml` lists them as `[[agents]]`, each with an `id` (stable, what
sessions store), a `name` (what the Mac shows and what the user says: «Oye
Lyra») and a `kind`, plus the kind's own keys. The first entry is the default
agent unless `default_agent` names another. Without `[[agents]]` the hub is
one implicit agent, `default`, made of `assistant_name` and `profile` as
before agents existed, so a 0.9.0 `orbit.toml` loads as it did.

Adding a kind is one entry in `KINDS`, an `AgentKind` with four functions:

    KINDS = {
        "hermes": AgentKind(hermes.parse_agent, hermes.agent_backend, hermes.agent_speech, …),
        "openai": AgentKind(openai_settings.parse_settings, _openai_backend, _openai_speech, …),
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

Everything else is the same for every kind: the ids, names and refusals of
`[[agents]]` here, and the default, the refusals of `chat.send` and the
`ready` list in the registry (`agents.Agents`).
"""

from __future__ import annotations

import json
import re
import unicodedata
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from functools import partial
from typing import TYPE_CHECKING, Any

from . import hermes, openai_settings, tool_settings
from .agent_config import AgentAdmin, AgentConfigStore, AgentInstructions, StoredAdmin
from .backend import AssistantBackend
from .hermes_panel import HermesPanelAdmin
from .roster import MAX_AGENTS
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


# The generic backend imports pydantic-ai and both providers' SDKs, over a
# second together: only a hub that builds an `openai` agent imports it, never
# the administrator's commands, which load the configuration alone.
def _openai_backend(
    settings: Settings, agent: AgentConfiguration, instructions: InstructionSource
) -> AssistantBackend:
    from . import openai

    return openai.build_backend(settings, agent, instructions)


def _openai_speech(settings: Settings, agent: AgentConfiguration) -> SpeechBackend:
    from . import openai

    return openai.build_speech(settings, agent)


# Vendor SDK imports belong only to a hub that actually builds these agents.
def _official_backend(settings, agent, instructions) -> AssistantBackend:
    from .official_tools import build_backend

    return build_backend(settings, agent, instructions)


def stored_admin(settings: Settings, agent: Any, configs: AgentConfigStore) -> AgentAdmin:
    return StoredAdmin(configs, agent.id)


def _hermes_admin(settings: Settings, agent: Any, configs: AgentConfigStore) -> AgentAdmin:
    return HermesPanelAdmin(settings, agent.id, agent.settings.profile)


KINDS: dict[str, AgentKind] = {
    # Hermes keeps its own SOUL.md and skills: its panel is where Orbit edits them.
    "hermes": AgentKind(
        hermes.parse_agent, hermes.agent_backend, hermes.agent_speech, _hermes_admin
    ),
    "openai": AgentKind(
        openai_settings.parse_settings, _openai_backend, _openai_speech, stored_admin
    ),
    "claude-code": AgentKind(
        partial(tool_settings.parse_tool, "claude-code"),
        _official_backend,
        _no_speech,
        stored_admin,
    ),
    "codex": AgentKind(
        partial(tool_settings.parse_tool, "codex"), _official_backend, _no_speech, stored_admin
    ),
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
# The services an agent can be added with from Orbit, as the user knows them.
SERVICE_NAMES = {"claude-code": "Claude", "codex": "Codex"}


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
    elif len(entries) > MAX_AGENTS:
        raise ValueError(
            f"agents lists {len(entries)} entries; a hub offers at most {MAX_AGENTS} agents"
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
