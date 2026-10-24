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
            "La agenda de quien usa Orbit, leída de su Mac en el momento de preguntar, de todos sus "
            "calendarios. Acepta cualquier rango en una sola llamada: un día, el mes que "
            "viene, diciembre, el año pasado o un año entero; Orbit lo lee entero por dentro "
            "y devuelve todos los eventos. Es la herramienta que hay que usar para cualquier "
            "pregunta sobre su agenda. start_ms y end_ms son milisegundos desde 1970 (UTC), "
            "con end_ms posterior a start_ms."
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
