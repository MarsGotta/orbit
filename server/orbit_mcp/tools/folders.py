"""The Mac folders the user gave the assistant: list, search and read, read-only."""

from __future__ import annotations

import base64

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult, ImageContent

from .. import files
from ..hub import HubClientError, validate_parameters
from .runner import ToolRunner, ok, text


def _present(**values) -> dict:
    return {key: value for key, value in values.items() if value is not None}


def _read_content(result: dict) -> CallToolResult:
    if result["content"] == "image":
        data = result.get("data_base64")
        if not isinstance(data, str) or result.get("mime_type") != "image/png":
            raise HubClientError("invalid_response")
        try:
            base64.b64decode(data, validate=True)
        except (ValueError, TypeError):
            raise HubClientError("invalid_response") from None
        return ok(
            ImageContent(type="image", data=data, mimeType="image/png"),
            text(files.render_read(result)),
        )
    return ok(text(files.render_read(result)))


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Lista las carpetas del Mac que quien usa Orbit dio al asistente y, con path, lo "
            "que hay dentro de una de ellas: carpetas, archivos con su tamaño y enlaces. Sin "
            "path dice cuáles son y si se pueden leer. Sólo lectura. Una carpeta grande llega "
            "por páginas: la respuesta da el after_index con el que seguir."
        ),
        structured_output=False,
    )
    async def orbit_files_list(
        path: str | None = None,
        after_index: int | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(path=path, after_index=after_index)
            validate_parameters("files_list", parameters)
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "files_list", parameters)
            return ok(text(files.render_list(result)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Busca en las carpetas del Mac que quien usa Orbit dio al asistente: por nombre de "
            "archivo o carpeta, sin distinguir mayúsculas ni acentos, y con content=true también "
            "dentro de los archivos de texto. path limita la búsqueda a una carpeta. Tiene "
            "techos de resultados, de entradas recorridas y de bytes leídos, y dice cuál la "
            "detuvo. Sólo lectura."
        ),
        structured_output=False,
    )
    async def orbit_files_search(
        query: str,
        path: str | None = None,
        content: bool | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(query=query, path=path, content=content)
            validate_parameters("files_search", parameters)
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "files_search", parameters
            )
            return ok(text(files.render_search(result)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=(
            "Lee un archivo de las carpetas del Mac que quien usa Orbit dio al asistente: texto "
            "o Markdown por páginas (after_index sigue donde acabó la anterior), el texto de un "
            "PDF por páginas (after_page y after_index) o una imagen, que llega como imagen. "
            "path es la ruta absoluta que dan orbit_files_list u orbit_files_search. Nada "
            "fuera de esas carpetas, tampoco a través de enlaces o «..». Sólo lectura."
        ),
        structured_output=False,
    )
    async def orbit_files_read(
        path: str,
        after_index: int | None = None,
        after_page: int | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(path=path, after_index=after_index, after_page=after_page)
            validate_parameters("files_read", parameters)
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "files_read", parameters)
            return _read_content(result)

        return await runner.invoke(operation, device)
