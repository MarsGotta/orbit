"""What each agent may use of Orbit's tools: five access groups, each on or off.

A group is a set of `orbit-mcp` tools and the device-bridge capabilities (or
meeting reads) those tools ask the hub for. Every group is on unless the user
turned it off in Settings → Agents; that is a configuration of the agent's
tools, asked nothing at use and never expiring, and macOS's own permission
stays the decision about the Mac.

It is enforced twice: `orbit-mcp` neither lists nor runs a tool of a group its
agent has off, and the hub refuses a bridge request of such a group with the
agent and the group. Both trust the agent id `orbit-mcp` sends, signed with
the bridge key: it configures a cooperative agent's tools, and it is not a
boundary against a hostile process that holds that key on the same machine.
"""

from __future__ import annotations

from types import MappingProxyType

from ..errors import PrivateError
from .skills import Skill

# In the order Settings shows them.
GROUPS = ("screen", "control", "calendar", "meetings", "files")

# The orbit-mcp tools no group gates: `orbit_devices` names the Macs, which
# every tool needs to choose one; `orbit_attachment` reads a file the user
# attached to a message of this agent's own conversation; and `orbit_skill`
# reads the agent's own configuration, nothing of the user's.
UNGATED = frozenset({"orbit_devices", "orbit_attachment", "orbit_skill"})

# The orbit-mcp tools each group opens. Settings › Agents shows these lists
# beside each switch: contracts/constants.json registers both copies
# (AGENT_ACCESS_GROUP_TOOLS) and its guards compare them.
TOOLS = MappingProxyType(
    {
        "screen": ("orbit_windows", "orbit_screenshot", "orbit_look", "orbit_selected_text"),
        "control": ("orbit_control_tools", "orbit_perform_action", "orbit_action_result"),
        "calendar": ("orbit_calendar",),
        "meetings": ("orbit_meetings", "orbit_transcript", "orbit_note"),
        "files": ("orbit_files_list", "orbit_files_search", "orbit_files_read"),
    }
)
TOOL_GROUP = MappingProxyType({tool: group for group, tools in TOOLS.items() for tool in tools})

# The device-bridge capability each tool asks a Mac for, by group. The
# meeting reads have routes of their own and are `meetings`; the action
# result read is `control`. A capability missing here would pass the hub's
# check unguarded, so a test holds this to orbit-mcp's capability list.
CAPABILITY_GROUP = MappingProxyType(
    {
        "screen_sources": "screen",
        "screenshot": "screen",
        "selected_text": "screen",
        "control_tools": "control",
        "perform_action": "control",
        "calendar_read": "calendar",
        "files_list": "files",
        "files_search": "files",
        "files_read": "files",
    }
)


def default_groups() -> dict[str, bool]:
    """Every group on: an agent nobody configured uses every tool, as before."""
    return dict.fromkeys(GROUPS, True)


def disabled_message(agent_id: str, group: str) -> str:
    return (
        f"El agente «{agent_id}» tiene desactivado el acceso «{group}» en Ajustes › Agentes: "
        "no puede usar esa herramienta"
    )


class AgentAccess:
    """The hub's side of the check: which agent a bridge request is for,
    whether its group is on, and which skills it has enabled (`orbit_skill`).
    `agents` is the registry, `configs` the store."""

    def __init__(self, agents, configs):
        self.agents, self.configs = agents, configs

    def agent(self, agent_id: str | None):
        """The agent orbit-mcp speaks for; no id is the default agent, as an
        orbit-mcp started without `ORBIT_MCP_AGENT` is."""
        agent = self.agents.default if agent_id is None else self.agents.by_id.get(agent_id)
        if agent is None:
            raise PrivateError(
                404,
                "agent_not_found",
                # Comma-joined: refusal numbers are scalars.
                {"agent_id": agent_id, "agent_ids": ",".join(self.agents.by_id)},
            )
        return agent

    def groups(self, agent_id: str | None) -> tuple[str, dict[str, bool]]:
        """The agent's id and groups."""
        agent = self.agent(agent_id)
        return agent.id, self.configs.access(agent.id)

    def skill(self, agent_id: str | None, name: str) -> tuple[str, Skill]:
        """The agent's id and its enabled skill `name`, for orbit-mcp's
        `orbit_skill`. It belongs to no group: it reads the agent's own
        configuration. A Hermes agent's skills are its profile's, never the
        hub's rows; a name not enabled is refused with every enabled one."""
        agent = self.agent(agent_id)
        if not agent.skills_in_hub:
            raise PrivateError(409, "skills_not_in_hub", {"agent_id": agent.id, "kind": agent.kind})
        enabled = {skill.name: skill for skill, on in self.configs.skills(agent.id) if on}
        skill = enabled.get(name)
        if skill is None:
            raise PrivateError(
                404,
                "skill_not_enabled",
                # Comma-joined, in name order: refusal numbers are scalars.
                {"agent_id": agent.id, "name": name, "enabled": ",".join(enabled)},
            )
        return agent.id, skill

    def require(self, agent_id: str | None, group: str) -> None:
        resolved, groups = self.groups(agent_id)
        if not groups[group]:
            raise PrivateError(403, "agent_access_disabled", {"agent_id": resolved, "group": group})
