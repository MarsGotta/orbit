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
            "Lists the Mac folders the Orbit user gave the assistant and, with path, their contents: folders, files with sizes and links. Without path, reports which folders exist and whether they are readable. Read-only. Large folders are paginated: the response gives after_index to continue."
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
            "Searches the Mac folders the Orbit user gave the assistant: file or folder names, ignoring case and accents, and with content=true also inside text files. path restricts the search to one folder. Has ceilings for results, traversed entries and bytes read, and reports which ceiling stopped it. Read-only."
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
            "Reads a file from the Mac folders the Orbit user gave the assistant: paginated text or Markdown (after_index resumes where the previous page ended), paginated PDF text (after_page and after_index), or an image, delivered as an image. path is the absolute path returned by orbit_files_list or orbit_files_search. Nothing outside those folders, including through links or .. . Read-only."
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
