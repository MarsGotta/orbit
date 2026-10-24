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
            "Lista las herramientas de control que el motor declara en ese Mac, con su "
            "descripción y su esquema de argumentos. Es la lista autoritativa: "
            "orbit_perform_action sólo acepta nombres que salgan de aquí. No ejecuta nada."
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
                return ok(text("El motor de ese Mac no declara ninguna herramienta."))
            return ok(text(json.dumps(tools, ensure_ascii=False)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Ejecuta una herramienta del motor en ese Mac. El nombre y los argumentos son "
            "los que declara orbit_control_tools; Orbit los pasa tal cual. Quien usa Orbit ya "
            "concedió en macOS el uso de su ordenador: nadie tiene que decir que sí otra vez. Si la "
            "acción sigue en marcha responde «in_progress» con su operation_id —quiere decir "
            "en curso, no que espere a nadie— y su desenlace se lee con orbit_action_result. "
            "Sin motor disponible responde que no lo está, con la causa, y no lo intenta por "
            "otro camino."
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
            return ok(text(json.dumps(result, ensure_ascii=False)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Consulta el desenlace de una acción pedida con orbit_perform_action. "
            "Distingue en curso, ejecución, resultado desconocido y "
            "fallo. «in_progress» significa que sigue ejecutándose en el Mac. "
            "También lee los recibos históricos guardados por versiones anteriores."
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
