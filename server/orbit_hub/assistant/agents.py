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

Everything else (ids, names, the default, the refusals of `chat.send`, the
`ready` list) is the same for every kind and lives here.
"""

from __future__ import annotations

import json
import re
import unicodedata
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, replace
from functools import partial
from typing import TYPE_CHECKING, Any
from uuid import uuid4

from ..errors import HubError
from . import acp_settings, hermes, openai_settings
from .agent_config import (
    AgentAdmin,
    AgentConfigStore,
    AgentInstructions,
    StoredAdmin,
    no_instructions,
)
from .backend import AssistantBackend
from .hermes_panel import HermesPanelAdmin
from .roster import (
    COLORS,
    MAX_AGENTS,
    UNKNOWN_ACCOUNT,
    Account,
    Added,
    Profile,
    Roster,
    profile_refusal,
)
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


# The ACP SDK of Claude Code and Codex takes a quarter of a second or more to
# import: only a hub that builds an `acp` agent imports it.
def _acp_backend(
    settings: Settings, agent: AgentConfiguration, instructions: InstructionSource
) -> AssistantBackend:
    from . import acp

    return acp.build_backend(settings, agent, instructions)


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
        openai_settings.parse_settings, _openai_backend, _openai_speech, _stored_admin
    ),
    # Claude Code and Codex speak no audio over ACP: no dictation, no read-aloud.
    "acp": AgentKind(acp_settings.parse_settings, _acp_backend, _no_speech, _stored_admin),
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


@dataclass
class Agent:
    id: str
    # What Orbit shows and the wake phrase hears: the profile's when someone
    # renamed it in Orbit, else `orbit.toml`'s.
    name: str
    kind: str
    settings: Any
    backend: AssistantBackend
    # Its dictation and read-aloud; None for a kind without a voice.
    speech: SpeechBackend | None = None

    @property
    def skills_in_hub(self) -> bool:
        """Whether the hub keeps this agent's skills (`StoredAdmin`) and hands
        them out through orbit-mcp's `orbit_skill`; a Hermes agent's are its
        profile's."""
        return self.kind in KINDS and KINDS[self.kind].admin is _stored_admin

    @property
    def service(self) -> str:
        """What answers: `claude-code` or `codex` for an `acp` agent, else its kind."""
        return self.settings.agent if self.kind == "acp" else self.kind

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

    def capabilities(self) -> list[str]:
        # A list of names, as contracts/hub-v1.md fixes and the Mac decodes
        # (`HubAgent.capabilities: Vec<String>`): an object would make the
        # Mac refuse the whole `ready`.
        capabilities = self.backend.capabilities
        return [
            name
            for name, accepted in (
                ("images", capabilities.images),
                ("voice", capabilities.voice),
                ("resume", capabilities.resume),
            )
            if accepted
        ]


# Builds an agent added from Orbit: its configuration is the hub's adapter
# for the service with the hub's own id; refused with `ValueError` when the
# adapter's command cannot start.
AgentBuilder = Callable[[AgentConfiguration], Agent]


class Agents:
    """The hub's agents, `orbit.toml`'s in their order and then those added
    from Orbit, the configured default one, and what the hub keeps of each
    (`roster`): its profile, the primary, its account.

    Two defaults, kept apart on purpose. `default` is `orbit.toml`'s: it owns
    conversations stored before agents existed, speaks for an orbit-mcp that
    names no agent and lends its voice to dictation and to an agent without
    one, so changing the primary never moves those. `primary_id` is the
    profile's: `ready`'s `default_agent_id` and the agent of a turn that
    names none."""

    def __init__(
        self,
        agents: Sequence[Agent],
        default_id: str,
        *,
        roster: Roster | None = None,
        adapters: Mapping[str, Any] | None = None,
        build: AgentBuilder | None = None,
    ):
        self.by_id = {agent.id: agent for agent in agents}
        self.default = self.by_id[default_id]
        self.roster = roster
        # The services an agent can be added with: an `[acp_adapters]` table each.
        self.adapters = dict(adapters or {})
        self.build = build
        self.configured_names = {agent.id: agent.name for agent in agents}
        self.profiles: dict[str, Profile] = {}
        self.accounts: dict[str, Account] = {}
        self.added: set[str] = set()
        # The plan's use each agent's provider last reported (`observe`).
        self.usage: dict[str, dict] = {}
        self.stored_primary: str | None = None
        # Called with no argument after every change a Mac must see
        # (`agents.changed`); the app sets it.
        self.changed: Callable[[], None] = lambda: None
        if roster is not None:
            self.profiles = roster.profiles()
            self.accounts = roster.accounts()
            self.stored_primary = roster.primary()
            for agent in self.by_id.values():
                if (profile := self.profiles.get(agent.id)) is not None:
                    self._rename(agent, profile.name)
        for agent in self.by_id.values():
            self._listen(agent)

    @classmethod
    def from_settings(
        cls,
        settings: Settings,
        backend: AssistantBackend | None = None,
        speech: SpeechBackend | None = None,
        configs: AgentConfigStore | None = None,
        roster: Roster | None = None,
    ) -> Agents:
        """The configured agents and those added from Orbit (`roster`), each
        on its kind's backend and speech, or all on `backend` and `speech`
        when they are given: tests drive the hub with doubles. `configs` holds
        what the hub keeps of each agent's personality and skills; without
        it, an agent has none. Raises `ValueError` naming the agent and the
        cause when one cannot be built or there are more than `MAX_AGENTS`."""
        configured, default_id = settings.agent_configurations()
        adapters = settings.adapter_settings()

        def instructions(agent_id: str) -> InstructionSource:
            if configs is None:
                return no_instructions
            return lambda: configs.instructions(agent_id)

        def build(agent: AgentConfiguration) -> Agent:
            return Agent(
                agent.id,
                agent.name,
                agent.kind,
                agent.settings,
                backend
                if backend is not None
                else KINDS[agent.kind].backend(settings, agent, instructions(agent.id)),
                speech if speech is not None else KINDS[agent.kind].speech(settings, agent),
            )

        agents = [build(agent) for agent in configured]
        added = roster.added() if roster is not None else []
        if len(agents) + len(added) > MAX_AGENTS:
            raise ValueError(
                f"the hub has {len(agents)} agents in its configuration and {len(added)} "
                f"added from Orbit; at most {MAX_AGENTS} together. Remove some from "
                "[[agents]] before starting"
            )
        profiles = roster.profiles() if roster is not None else {}
        for entry in added:
            if entry.service not in adapters:
                raise ValueError(
                    f"agent {entry.agent_id!r} was added from Orbit as {entry.service}, and "
                    f"[acp_adapters.{entry.service}] is missing: restore it to start the hub"
                )
            profile = profiles.get(entry.agent_id)
            name = profile.name if profile is not None else entry.agent_id
            agents.append(
                build(AgentConfiguration(entry.agent_id, name, "acp", adapters[entry.service]))
            )
        registry = cls(agents, default_id, roster=roster, adapters=adapters, build=build)
        registry.added = {entry.agent_id for entry in added}
        return registry

    def get(self, agent_id: str) -> Agent:
        agent = self.by_id.get(agent_id)
        if agent is None:
            ids = list(self.by_id)
            raise HubError(
                "agent_not_found",
                f"Este hub no tiene el agente «{agent_id}»; tiene: {', '.join(ids)}",
                # Comma-joined: a refusal's numbers stay integers, strings or
                # booleans, and the Mac's reader of a refusal drops an array.
                numbers={"agent_id": agent_id, "agent_ids": ",".join(ids)},
            )
        return agent

    def owner(self, session: dict) -> str:
        """The id of the agent a stored session belongs to: the one it names,
        or the default agent for a session stored before agents existed."""
        return self.known(session["agent_id"])

    def known(self, agent_id: str | None) -> str:
        """The agent an id stored or remembered before means now. None (before
        agents existed) and `default` (the implicit agent of a hub without
        `[[agents]]`, unless an entry keeps that id) are the default agent:
        the guide lists the old profile's agent first under an id of its own."""
        if agent_id is None or (agent_id == IMPLICIT_AGENT_ID and agent_id not in self.by_id):
            return self.default.id
        return agent_id

    @property
    def primary_id(self) -> str:
        """The profile's primary, or the configured default when none was
        chosen or the chosen one has left."""
        if self.stored_primary in self.by_id:
            return self.stored_primary
        return self.default.id

    def public(self) -> list[dict]:
        """The agents as `ready` and `agents.changed` list them to the Mac."""
        return [self.public_agent(agent, position) for position, agent in enumerate(self.by_id.values())]  # fmt: skip

    def public_agent(self, agent: Agent, position: int) -> dict:
        profile = self.profiles.get(agent.id)
        public = {
            "id": agent.id,
            "name": agent.name,
            "kind": agent.kind,
            "capabilities": agent.capabilities(),
            "service": agent.service,
            "description": profile.description if profile is not None else "",
            # Until someone chooses one, the four colours in turn.
            "color": profile.color if profile is not None else COLORS[position % len(COLORS)],
            "removable": agent.id in self.added,
        }
        if agent.kind == "acp":
            account = self.accounts.get(agent.id, UNKNOWN_ACCOUNT)
            public["account"] = account.state
            if account.email is not None:
                public["account_email"] = account.email
            if account.plan is not None:
                public["plan"] = account.plan
            if account.limited:
                public["limit"] = {"until_ms": account.until_ms}
            if (usage := self.usage.get(agent.id)) is not None:
                public["usage"] = usage
        return public

    def listing(self) -> dict:
        """What the routes of «Assistants from Orbit» answer and `agents.changed` carries."""
        return {
            "agents": self.public(),
            "default_agent_id": self.primary_id,
            "addable_agents": list(self.adapters),
        }

    def _listen(self, agent: Agent) -> None:
        """An ACP backend tells the registry what it learns of its account."""
        if agent.kind == "acp" and hasattr(agent.backend, "observe"):
            agent.backend.observe = partial(self.observe, agent.id)

    def _rename(self, agent: Agent, name: str) -> None:
        agent.name = name
        rename = getattr(agent.backend, "rename", None)
        if callable(rename):
            rename(name)

    def name_taken(self, name: str, other_than: str | None) -> Agent | None:
        """The agent already called `name`, ignoring case and accents: both
        would sound the same to «Oye {nombre}»."""
        said = spoken(name)
        return next(
            (
                agent
                for agent in self.by_id.values()
                if agent.id != other_than and spoken(agent.name) == said
            ),
            None,
        )

    def check_profile(self, profile: Profile, other_than: str | None) -> None:
        if (refusal := profile_refusal(profile.name, profile.description, profile.color)) is not None:  # fmt: skip
            raise refusal
        if (taken := self.name_taken(profile.name, other_than)) is not None:
            raise HubError(
                "agent_name_taken",
                "Ya tienes otro asistente que se llama así",
                http_status=409,
                numbers={"field": "name", "agent_id": taken.id},
            )

    def set_profile(self, agent_id: str, profile: Profile, primary: bool) -> None:
        """Store the agent's profile; `primary` makes it the primary. The
        primary cannot stop being it on its own: another one is made primary."""
        agent = self.get(agent_id)
        self.check_profile(profile, agent.id)
        if not primary and agent.id == self.primary_id:
            raise HubError(
                "agent_primary_required",
                f"{agent.name} es el asistente principal: elige otro como principal para cambiarlo",
                http_status=409,
                numbers={"agent_id": agent.id},
            )
        self._roster().save_profile(agent.id, profile, primary)
        self.profiles[agent.id] = profile
        if primary:
            self.stored_primary = agent.id
        self._rename(agent, profile.name)
        self.changed()

    def add(self, service: str, profile: Profile, now_ms: int) -> Agent:
        """Add a Claude Code or Codex agent on the hub's adapter for `service`."""
        if service not in self.adapters:
            raise HubError(
                "agent_adapter_missing",
                f"Este hub no tiene instalado el adaptador de {SERVICE_NAMES.get(service, service)}: "
                "pídeselo a quien gestiona tu hub",
                http_status=409,
                numbers={"service": service},
            )
        if len(self.by_id) >= MAX_AGENTS:
            raise HubError(
                "too_many_agents",
                f"Este hub ya tiene {len(self.by_id)} asistentes; máximo {MAX_AGENTS}",
                http_status=409,
                numbers={"count": len(self.by_id), "max": MAX_AGENTS},
            )
        self.check_profile(profile, None)
        agent_id = self._new_id(service)
        if self.build is None:
            raise HubError("agent_add_unsupported", "Este hub no puede añadir asistentes")
        try:
            agent = self.build(
                AgentConfiguration(agent_id, profile.name, "acp", self.adapters[service])
            )
        except ValueError as exc:
            # The command the operator configured does not start: the cause
            # names the path, the hub's own configuration, never the Mac's.
            raise HubError(
                "agent_adapter_missing",
                f"El adaptador de {SERVICE_NAMES[service]} no arranca en este hub ({exc}): "
                "pídeselo a quien gestiona tu hub",
                http_status=409,
                numbers={"service": service},
            ) from None
        self._roster().add(Added(agent_id, service), profile, now_ms)
        self.by_id[agent_id] = agent
        self.added.add(agent_id)
        self.profiles[agent_id] = profile
        self.accounts[agent_id] = Account("signed_out")
        self._rename(agent, profile.name)
        self._listen(agent)
        self.changed()
        return agent

    def remove(self, agent_id: str) -> Agent:
        """Forget an agent added from Orbit; its conversations stay with its id."""
        agent = self.get(agent_id)
        if agent.id not in self.added:
            raise HubError(
                "agent_not_removable",
                f"{agent.name} lo da la configuración de tu hub: solo quien lo gestiona puede quitarlo",
                http_status=403,
                numbers={"agent_id": agent.id},
            )
        self._roster().remove(agent.id)
        del self.by_id[agent.id]
        self.added.discard(agent.id)
        self.profiles.pop(agent.id, None)
        self.accounts.pop(agent.id, None)
        self.usage.pop(agent.id, None)
        self.changed()
        return agent

    def account(self, agent_id: str) -> Account:
        return self.accounts.get(agent_id, UNKNOWN_ACCOUNT)

    def observe(self, agent_id: str, fact: str, **values: Any) -> None:
        """What the hub learned of an agent's account, from its sign-in, its
        adapter or a turn; published when it changed. The facts:

        - `signed_in`: a sign-in finished; `signed_out`: a sign-out did.
        - `answered`: a turn ended with an answer, so the account works and
          is not at its limit.
        - `auth_required`: the agent refused for want of a sign-in: its
          session expired, unless it never had one.
        - `limit`: the plan's limit; `until_ms` when the provider said when
          it resets.
        - `status`: the adapter's `_auth/status_update`, `kind` (`none`
          for no account), and the account's `email` and `plan` when given.
        - `usage`: the plan's use the provider reported, `used_percent`,
          `window` and `resets_at_ms`; kept in memory, the latest only.
        """
        if agent_id not in self.by_id:
            return
        if fact == "usage":
            if self.usage.get(agent_id) != values:
                self.usage[agent_id] = values
                self.changed()
            return
        current = self.account(agent_id)
        if fact in {"signed_in", "answered"}:
            account = replace(current, state="signed_in", limited=False, until_ms=None)
            if fact == "signed_in":
                self.usage.pop(agent_id, None)
        elif fact == "signed_out":
            account = Account("signed_out")
            self.usage.pop(agent_id, None)
        elif fact == "auth_required" or (fact == "status" and values.get("kind") == "none"):
            state = "signed_out" if current.state == "signed_out" else "expired"
            account = replace(current, state=state)
        elif fact == "status":
            account = replace(
                current,
                state="signed_in",
                email=values.get("email") or current.email,
                plan=values.get("plan") or current.plan,
            )
        elif fact == "limit":
            account = replace(current, limited=True, until_ms=values.get("until_ms"))
        else:
            return
        if account == current:
            return
        if self.roster is not None:
            self.roster.save_account(agent_id, account)
        self.accounts[agent_id] = account
        self.changed()

    def _new_id(self, service: str) -> str:
        prefix = "claude" if service == "claude-code" else service
        while True:
            agent_id = f"{prefix}-{uuid4().hex[:8]}"
            if agent_id not in self.by_id:
                return agent_id

    def _roster(self) -> Roster:
        if self.roster is None:
            raise HubError("agent_add_unsupported", "Este hub no guarda perfiles de asistentes")
        return self.roster
