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
            text(f"Attachment {head}."),
        )
    end = start + len(payload["text"])
    following = payload.get("next_after_index")
    where = (
        f"characters {start}–{end} of {payload['total_chars']}; continue with after_index {following}"
        if following is not None
        else f"characters {start}–{end} of {payload['total_chars']}, to the end"
    )
    return ok(text(f"Attachment {head}, {where}.\n{payload['text']}"))


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Reads a file the Orbit user attached to a message, using the id the message "
            "gives for each attachment. Images arrive as images; PDF or text-file text is "
            f"paginated with up to {MAX_ANSWER_CHARS} characters, and the first line states "
            "the range and after_index to continue."
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
