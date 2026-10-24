"""What is on a Mac's screen: its windows, a screenshot, a look, the selected text."""

from __future__ import annotations

import base64
import json

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult, ImageContent

from ..hub import HubClientError
from .runner import ToolRunner, ok, text


def _screenshot_content(
    result: dict,
    *,
    windows: list[dict] | None = None,
) -> CallToolResult:
    expected = ("application", "title", "width", "height", "mime_type", "data_base64")
    if any(key not in result for key in expected) or result["mime_type"] != "image/png":
        raise HubClientError("invalid_response")
    try:
        base64.b64decode(result["data_base64"], validate=True)
    except (ValueError, TypeError):
        raise HubClientError("invalid_response") from None
    if not isinstance(result["data_base64"], str):
        raise HubClientError("invalid_response")
    details = (
        f"Captura de {result['application']} — {result['title']} "
        f"({result['width']} × {result['height']})."
    )
    if isinstance(result.get("coordinates"), str):
        details += f" {result['coordinates']}"
    if windows is not None:
        details += f" Ventanas disponibles: {json.dumps(windows, ensure_ascii=False)}"
    return ok(
        ImageContent(type="image", data=result["data_base64"], mimeType="image/png"),
        text(details),
    )


def _window_parameters(application: str | None, title: str | None) -> dict:
    parameters = {}
    if application is not None:
        parameters["application"] = application
    if title is not None:
        parameters["title"] = title
    return parameters


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description="Lista las ventanas visibles de un Mac mediante Orbit.",
        structured_output=False,
    )
    async def orbit_windows(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "screen_sources", {})
            windows = result.get("windows")
            if not isinstance(windows, list):
                raise HubClientError("invalid_response")
            return ok(text(json.dumps(windows, ensure_ascii=False)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Obtiene una captura PNG; application y title eligen una ventana, sin ambos usa "
            "la ventana en primer plano, y full_screen=true pide el escritorio completo al motor."
        ),
        structured_output=False,
    )
    async def orbit_screenshot(
        device: str | None = None,
        application: str | None = None,
        title: str | None = None,
        full_screen: bool = False,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            parameters = _window_parameters(application, title)
            if full_screen:
                if parameters:
                    raise HubClientError("invalid_parameters")
                parameters["full_screen"] = True
            result = await hub_client.device_request(selection.device_id, "screenshot", parameters)
            return _screenshot_content(result)

        return await runner.invoke(operation, device)

    @server.tool(
        description="Mira la pantalla: lista las ventanas y captura la ventana en primer plano.",
        structured_output=False,
    )
    async def orbit_look(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            inventory = await hub_client.device_request(selection.device_id, "screen_sources", {})
            windows = inventory.get("windows")
            if not isinstance(windows, list):
                raise HubClientError("invalid_response")
            screenshot = await hub_client.device_request(selection.device_id, "screenshot", {})
            return _screenshot_content(screenshot, windows=windows)

        return await runner.invoke(operation, device)

    @server.tool(
        description="Lee el texto seleccionado en una ventana concreta o en la ventana en primer plano.",
        structured_output=False,
    )
    async def orbit_selected_text(
        device: str | None = None,
        application: str | None = None,
        title: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "selected_text", _window_parameters(application, title)
            )
            if not all(
                isinstance(result.get(key), str) for key in ("application", "title", "text")
            ):
                raise HubClientError("invalid_response")
            return ok(
                text(
                    f"Texto seleccionado en {result['application']} — {result['title']}:\n"
                    f"{result['text']}"
                )
            )

        return await runner.invoke(operation, device)
