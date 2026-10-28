"""orbit_devices: the Macs the hub knows, their connection and when they were seen."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult

from orbit_hub.protocol import DeviceSummary

from ..config import Config
from ..devices import hub_selection, seen_ago, validated_devices
from ..hub import HubClientError
from .runner import ToolRunner, ok, text


def _configured_lines(config: Config) -> list[str]:
    """The file's names, when the hub has no device listing to give."""
    return [
        f"- {name}{' (default)' if name == config.default_device else ''} — configured in the file"
        for name in config.devices
    ]


def _default_id(config: Config, devices: list[DeviceSummary]) -> str | None:
    if config.default_device in config.devices:
        return config.devices[config.default_device]
    if devices:
        try:
            return hub_selection(devices, config.default_device).device_id
        except HubClientError:
            pass
    return None


def _device_line(config: Config, device: DeviceSummary, default_id: str | None) -> str:
    aliases = [name for name, device_id in config.devices.items() if device_id == device.id]
    markers = []
    if aliases:
        markers.append("file: " + ", ".join(aliases))
    if device.id == default_id:
        markers.append("default")
    suffix = f" — {', '.join(markers)}" if markers else ""
    return (
        f"- {device.name} — connected {'yes' if device.connected else 'no'} — "
        f"last seen {seen_ago(device.last_seen)}{suffix}"
    )


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description="Lists the Macs known to the hub, their connection state and when they were last seen.",
        structured_output=False,
    )
    async def orbit_devices() -> CallToolResult:
        async def operation(select):
            active_config = runner.current_config()
            try:
                devices = validated_devices(await hub_client.devices())
            except HubClientError as exc:
                if exc.status != 404 or not active_config.devices:
                    raise
                lines = _configured_lines(active_config)
                return ok(text("Orbit devices:\n" + "\n".join(lines)))
            default_id = _default_id(active_config, devices)
            lines = [_device_line(active_config, device, default_id) for device in devices]
            return ok(text("Orbit devices:\n" + ("\n".join(lines) or "None.")))

        return await runner.invoke(operation)
