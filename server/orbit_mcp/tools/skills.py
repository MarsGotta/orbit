"""orbit_skill: the instructions of one of the agent's skills, kept by the hub."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult

from orbit_hub.assistant.agent_config import SKILL_TOOL
from orbit_hub.assistant.skills import MAX_NAME_CHARS

from ..hub import HubClientError
from .runner import ToolRunner, ok, text


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    # In no access group: it reads the agent's own configuration in the hub,
    # nothing of the user's Mac or meetings.
    @server.tool(
        name=SKILL_TOOL,
        description=(
            "Loads the instructions of one of your skills enabled in Orbit, to follow them. name is its name as listed in your skills list. Reads nothing from the Mac."
        ),
        structured_output=False,
    )
    async def orbit_skill(name: str) -> CallToolResult:
        async def operation(select):
            if len(name) > MAX_NAME_CHARS:
                raise HubClientError(
                    "invalid_parameters",
                    detail={
                        "refusal": {
                            "code": "invalid_parameters",
                            "parameter": "name",
                            "chars": len(name),
                            "max_chars": MAX_NAME_CHARS,
                        }
                    },
                )
            return ok(text(await hub_client.skill(name)))

        return await runner.invoke(operation)
