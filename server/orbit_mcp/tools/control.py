"""Using a Mac: the engine's tools, an action with one of them, and its outcome."""

from __future__ import annotations

import json

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult, TextContent

from orbit_hub.identifiers import canonical_uuid

from ..bridge import ToolAction as GeneralAction
from ..hub import HubClientError, validate_parameters
from ..refusals import error_result, mac_sentence
from .runner import ToolRunner, ok, text


def general_action(value: object) -> dict:
    validate_parameters("perform_action", {"action": value})
    return GeneralAction.model_validate(value).model_dump()


def _operation_id(value: str) -> str:
    try:
        canonical_uuid(value)
    except (ValueError, TypeError, AttributeError):
        raise HubClientError("invalid_parameters") from None
    return value


def _described_outcome(result: dict) -> dict:
    """A failed or unknown outcome with the sentence its code has as a refusal."""
    if result.get("outcome") in {"failed", "unknown"}:
        code = result.get("code")
        if isinstance(code, str):
            detail = {
                "refusal": result.get("refusal"),
                "mac_notice": result.get("notice"),
                "device_version": result.get("device_version"),
            }
            result["descripcion"] = "".join(
                block.text
                for block in error_result(HubClientError(code, detail=detail)).content
                if isinstance(block, TextContent)
            )
            if "notice" in result:
                result["notice"] = mac_sentence(detail)
    return result


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Lists the control tools the engine declares on that Mac, with their descriptions and argument schemas. This is the authoritative list: orbit_perform_action only accepts names from it. Executes nothing."
        ),
        structured_output=False,
    )
    async def orbit_control_tools(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            # The Mac answers in pages; the walk happens inside the client so
            # that Deimos asks once and receives the catalogue entire.
            tools = await hub_client.control_tools(selection.device_id)
            if not all(
                isinstance(tool, dict) and isinstance(tool.get("name"), str) for tool in tools
            ):
                raise HubClientError("invalid_response")
            if not tools:
                return ok(text("That Mac's engine declares no tools."))
            return ok(text(json.dumps(tools, ensure_ascii=False)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Runs an engine tool on that Mac. The name and arguments are those declared by orbit_control_tools; Orbit passes them unchanged. The Orbit user already granted use of their computer in macOS: nobody needs to approve again. If the action is still running, returns in_progress with its operation_id — this means running, not waiting for anyone — and its outcome is read with orbit_action_result. If the engine is unavailable, reports that with its cause and does not try another path."
        ),
        structured_output=False,
    )
    async def orbit_perform_action(
        action: GeneralAction,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = {"action": action.model_dump()}
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "perform_action", parameters
            )
            return ok(text(json.dumps(_described_outcome(result), ensure_ascii=False)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Reads the outcome of an action requested with orbit_perform_action. Distinguishes in progress, executed, unknown outcome and failed. in_progress means it is still running on the Mac. Also reads historical receipts saved by older versions."
        ),
        structured_output=False,
    )
    async def orbit_action_result(
        operation_id: str,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            identity = _operation_id(operation_id)
            selection = await select()
            result = await hub_client.action_result(selection.device_id, identity)
            return ok(text(json.dumps(_described_outcome(result), ensure_ascii=False)))

        return await runner.invoke(operation, device)
