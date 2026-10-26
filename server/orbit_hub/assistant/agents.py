"""The hub's agents: each one an `AssistantBackend` of its kind (`agent_kinds`).

`Agents` is the registry the hub runs: `orbit.toml`'s agents in their order,
those added from Orbit, the defaults, each agent's profile and account, and
its backend as an isolated integration (`integration`): its starts bounded
and its last failure in the list the Macs read. The kinds, the configuration
`[[agents]]` and its refusals are `agent_kinds`'s.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field, replace
from functools import partial
from typing import TYPE_CHECKING, Any
from uuid import uuid4

from ..errors import HubError
from ..log import report
from .agent_config import AgentConfigStore, no_instructions
from .agent_kinds import (
    IMPLICIT_AGENT_ID,
    KINDS,
    SERVICE_NAMES,
    AgentConfiguration,
    InstructionSource,
    spoken,
    stored_admin,
)
from .backend import AssistantBackend
from .integration import Integration
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
from .tool_availability import ToolUnavailable, missing

if TYPE_CHECKING:
    from ..config import Settings


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
    # Its backend as the hub runs it: bounded starts, the last one's failure.
    integration: Integration = field(default_factory=Integration)

    @property
    def skills_in_hub(self) -> bool:
        """Whether the hub keeps this agent's skills (`StoredAdmin`) and hands
        them out through orbit-mcp's `orbit_skill`; a Hermes agent's are its
        profile's."""
        return self.kind in KINDS and KINDS[self.kind].admin is stored_admin

    @property
    def service(self) -> str:
        """What answers: `claude-code` or `codex` for an official-tool agent, else its kind."""
        return self.kind

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


# Builds an agent added from Orbit: its configuration is the hub's official tool
# for the service with the hub's own id; refused with `ValueError` when the
# tool's command cannot start.
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
        tools: Mapping[str, Any] | None = None,
        build: AgentBuilder | None = None,
    ):
        self.by_id = {agent.id: agent for agent in agents}
        self.default = self.by_id[default_id]
        self.roster = roster
        # Both shipped services, with any `[assistant_tools]` overrides applied.
        self.tools = dict(tools or {})
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
        tools = settings.assistant_tool_settings()

        def instructions(agent_id: str) -> InstructionSource:
            if configs is None:
                return no_instructions
            return lambda: configs.instructions(agent_id)

        def build(agent: AgentConfiguration) -> Agent:
            built = Agent(
                agent.id,
                agent.name,
                agent.kind,
                agent.settings,
                backend
                if backend is not None
                else KINDS[agent.kind].backend(settings, agent, instructions(agent.id)),
                speech if speech is not None else KINDS[agent.kind].speech(settings, agent),
            )

            if agent.kind in SERVICE_NAMES and hasattr(built.backend, "workspaces"):
                built.backend.workspaces.configs = configs
            return built

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
            if entry.service not in tools:
                raise ValueError(
                    f"agent {entry.agent_id!r} was added from Orbit as {entry.service}, and "
                    "this release does not support that service"
                )
            profile = profiles.get(entry.agent_id)
            name = profile.name if profile is not None else entry.agent_id
            agents.append(
                build(AgentConfiguration(entry.agent_id, name, entry.service, tools[entry.service]))
            )
        registry = cls(agents, default_id, roster=roster, tools=tools, build=build)
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
        if (backend := agent.integration.public()) is not None:
            public["backend"] = backend
        if agent.kind in SERVICE_NAMES:
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
            "addable_agents": list(self.tools),
        }

    def _listen(self, agent: Agent) -> None:
        """Every Mac learns when the agent's backend fails to start or starts
        again; an official backend tells the registry what it learns of its account."""
        agent.integration.changed = lambda: self.changed()
        if agent.kind in SERVICE_NAMES and hasattr(agent.backend, "observe"):
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
        """Add a Claude Code or Codex agent on the hub's official tool for `service`."""
        if service not in self.tools:
            raise HubError(
                "agent_adapter_missing",
                f"Este hub no ofrece {SERVICE_NAMES.get(service, service)}; publica de nuevo el hub",
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
                AgentConfiguration(agent_id, profile.name, service, self.tools[service])
            )
        except ValueError as exc:
            path = self.tools[service].command[0]
            cause = exc.cause if isinstance(exc, ToolUnavailable) else str(exc)
            raise missing(service, path, cause) from None
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
        tool or a turn; published when it changed. The facts:

        - `signed_in`: a sign-in finished; `signed_out`: a sign-out did.
        - `answered`: a turn ended with an answer, so the account works and
          is not at its limit.
        - `auth_required`: the agent refused for want of a sign-in: its
          session expired, unless it never had one.
        - `limit`: the plan's limit; `until_ms` when the provider said when
          it resets.
        - `status`: the tool's account report, `kind` (`none`
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
            try:
                self.roster.save_account(agent_id, account)
            except Exception as failure:  # noqa: BLE001 - a fact learned mid-turn must not end the turn
                # It is learned from an agent's turn or sign-in: a write the
                # disk refuses must not end that turn. The Macs still learn
                # it; the hub reads it again from the agent after a restart.
                from ..storage import is_storage_full

                code = "storage_full" if is_storage_full(failure) else "agent_account_not_saved"
                report("agent_account_not_saved", code, exc_info=False)
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
