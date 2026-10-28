"""English-described MCP tools backed by the private Orbit hub."""

from __future__ import annotations

import time
from time import monotonic

from mcp.server.mcpserver import MCPServer

from orbit_hub.assistant.access import TOOL_GROUP
from orbit_hub.assistant.agent_config import SKILL_TOOL
from orbit_hub.log import get_logger

from .config import Config
from .hub import HubClient, HubClientError, validate_parameters
from .refusals import error_result
from .tools import agenda, attachments, control, devices, folders, notes, recordings, screen, skills
from .tools.control import general_action
from .tools.runner import ToolRunner

logger = get_logger("orbit_hub.orbit_mcp.server")

# How long the agent's access groups are taken as read. What it protects: the
# hub from one access read per tool call. Why 10 s: as the device list's
# cache; a change in Settings › Agents reaches this translator within it, and
# the hub refuses a disabled group's request at once regardless.
ACCESS_CACHE_SECONDS = 10.0

# The tool families in the order their tools are listed.
TOOL_FAMILIES = (devices, screen, control, agenda, folders, recordings, notes, attachments, skills)


class OrbitMCPServer(MCPServer):
    """orbit-mcp's tools, without those of a group its agent has off."""

    hub_client: HubClient | None = None
    _access: tuple[float, tuple[str, dict[str, bool], bool]] | None = None

    async def agent_access(self) -> tuple[str, dict[str, bool], bool] | None:
        """The agent's id, its access groups and whether the hub keeps its
        skills, or None when the hub could not say.

        Unknown is not "off": every tool is offered then, and the hub, which
        checks each request itself, refuses what is off.
        """
        now = monotonic()
        if self._access is not None and now - self._access[0] < ACCESS_CACHE_SECONDS:
            return self._access[1]
        if self.hub_client is None:
            return None
        try:
            known = await self.hub_client.agent_access()
        except HubClientError as exc:
            logger.info(
                "orbit_mcp_access_unknown",
                extra={"event": "orbit_mcp_access_unknown", "code": exc.code},
            )
            return None
        self._access = (now, known)
        return known

    async def list_tools(self):
        tools = await super().list_tools()
        known = await self.agent_access()
        if known is None:
            return tools
        _agent_id, groups, skills_in_hub = known
        return [
            tool
            for tool in tools
            if groups.get(TOOL_GROUP.get(tool.name, ""), True)
            # A Hermes agent's skills are its profile's: this tool could only refuse.
            and (skills_in_hub or tool.name != SKILL_TOOL)
        ]

    async def call_tool(self, name, arguments, context=None):
        group = TOOL_GROUP.get(name)
        if group is not None:
            known = await self.agent_access()
            if known is not None and not known[1][group]:
                return error_result(
                    HubClientError(
                        "agent_access_disabled",
                        403,
                        {
                            "refusal": {
                                "code": "agent_access_disabled",
                                "agent_id": known[0],
                                "group": group,
                            }
                        },
                    )
                )
        if name == "orbit_perform_action":
            try:
                action = general_action((arguments or {}).get("action"))
            except HubClientError as exc:
                return error_result(exc)
            arguments = {**arguments, "action": action}
        # MCP 2.2's generated argument model drops extra keys before invoking
        # the function, so capture filters must be checked on the raw call.
        capability = {"orbit_screenshot": "screenshot", "orbit_selected_text": "selected_text"}.get(
            name
        )
        if capability is not None:
            parameters = {key: value for key, value in (arguments or {}).items() if key != "device"}
            try:
                validate_parameters(capability, parameters)
            except HubClientError as exc:
                return error_result(exc)
        return await super().call_tool(name, arguments, context)


def build_server(config: Config, hub_client: HubClient, *, clock=time.time) -> MCPServer:
    server = OrbitMCPServer(
        name="orbit",
        instructions="Private Orbit tools.",
    )
    server.hub_client = hub_client
    runner = ToolRunner(config, hub_client, clock)
    for family in TOOL_FAMILIES:
        family.register(server, runner)
    return server
