"""orbit_calendar: the user's agenda, read from their Mac at the time of asking."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult

from .. import calendar as agenda
from ..hub import HubClientError
from .runner import ToolRunner, ok, text


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "The Orbit user’s agenda, read directly from their Mac at the time of asking, across all their calendars. Accepts any range in one call: a day, next month, December, last year or an entire year; Orbit reads the whole range internally and returns every event. Use this tool for any question about their agenda. start_ms and end_ms are milliseconds since 1970 (UTC), with end_ms later than start_ms."
        ),
        structured_output=False,
    )
    async def orbit_calendar(
        start_ms: int,
        end_ms: int,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            if not isinstance(start_ms, int) or not isinstance(end_ms, int):
                raise HubClientError("invalid_parameters")
            if end_ms <= start_ms:
                raise HubClientError("invalid_parameters")
            selection = await select()
            # Whole milliseconds, as numbers, held to
            # `contracts/fixtures/device-bridge-request-v1.json`. The whole
            # range goes to the Mac, which answers it in pages; the walk
            # joins them, so this is one call however long the range is.
            reading = await hub_client.calendar_read(selection.device_id, start_ms, end_ms)
            return ok(text(agenda.render(reading)))

        return await runner.invoke(operation, device)
