"""orbit_attachment: a file the user attached to a message, by its id."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult, ImageContent

from orbit_hub.identifiers import canonical_uuid
from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

from ..hub import HubClientError
from .runner import ToolRunner, ok, page_start, text


def _attachment_id(value) -> str:
    try:
        return canonical_uuid(value)
    except (ValueError, TypeError, AttributeError):
        raise HubClientError(
            "invalid_parameters",
            detail={"refusal": {"code": "invalid_parameters", "parameter": "attachment_id"}},
        ) from None


def _attachment_content(payload: dict, start: int) -> CallToolResult:
    attachment = payload["attachment"]
    head = f"«{attachment['name']}» ({attachment['mime']}, {attachment['bytes']} bytes)"
    if attachment["kind"] == "image":
        return ok(
            ImageContent(type="image", data=payload["data_base64"], mimeType=attachment["mime"]),
            text(f"Adjunto {head}."),
        )
    end = start + len(payload["text"])
    following = payload.get("next_after_index")
    where = (
        f"caracteres {start}–{end} de {payload['total_chars']}; sigue con after_index {following}"
        if following is not None
        else f"caracteres {start}–{end} de {payload['total_chars']}, hasta el final"
    )
    return ok(text(f"Adjunto {head}, {where}.\n{payload['text']}"))


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Lee un fichero que quien usa Orbit adjuntó a un mensaje, por el id que el "
            "mensaje da de cada adjunto. Una imagen llega como imagen; el texto de un PDF o "
            "de un fichero de texto llega por páginas de hasta "
            f"{MAX_ANSWER_CHARS} caracteres, y la primera línea dice cuáles son y el "
            "after_index con el que seguir."
        ),
        structured_output=False,
    )
    async def orbit_attachment(
        attachment_id: str,
        device: str | None = None,
        after_index: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            identity = _attachment_id(attachment_id)
            start = page_start(after_index, "after_index")
            payload = await hub_client.attachment(selection.device_id, identity, start)
            return _attachment_content(payload, start)

        return await runner.invoke(operation, device)
