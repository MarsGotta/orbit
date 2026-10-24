"""How every tool call runs: the current configuration, the Mac it is about,
and every failure turned into a tool result."""

from __future__ import annotations

from collections.abc import Callable
from time import monotonic

from mcp.types import CallToolResult, ImageContent, TextContent

from orbit_hub.log import bind_op

from ..config import Config, ConfigError
from ..devices import device_selection
from ..hub import HubClient, HubClientError
from ..refusals import error_result, unexpected_result


def text(value: str) -> TextContent:
    return TextContent(type="text", text=value)


def ok(*content: TextContent | ImageContent) -> CallToolResult:
    return CallToolResult(content=list(content), isError=False)


def page_start(value, name: str) -> int:
    """Where a paged answer starts: `after_index`, 0 when absent."""
    if value is None:
        return 0
    if type(value) is not int or not 0 <= value < 2**63:
        raise HubClientError(
            "invalid_parameters",
            detail={"refusal": {"code": "invalid_parameters", "parameter": name}},
        )
    return value


class ToolRunner:
    """What every tool shares: the hub, the configuration as last read, the clock."""

    def __init__(self, config: Config, hub_client: HubClient, clock: Callable[[], float]):
        self._config = config
        self.hub_client = hub_client
        self.clock = clock

    def current_config(self) -> Config:
        try:
            refreshed = self._config.reload_if_changed()
        except ConfigError:
            raise HubClientError("config_invalid") from None
        if refreshed is not self._config:
            self._config = refreshed
            self.hub_client.update_config(refreshed)
        return self._config

    async def invoke(self, operation, device=None) -> CallToolResult:
        started = monotonic()
        bind_op(None)
        selection = None

        async def select():
            nonlocal selection
            selection = await device_selection(self.current_config(), self.hub_client, device)
            return selection

        try:
            result = await operation(select)
        except HubClientError as exc:
            result = error_result(exc, selection)
        except Exception:  # noqa: BLE001 -- MCP must return every failure as a tool result
            selected = selection.requested_name if selection is not None else device
            result = unexpected_result(selected, started)
        if selection is not None and selection.notice is not None:
            result.content.append(text(selection.notice))
        return result
