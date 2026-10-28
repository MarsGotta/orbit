"""Orbit's tools for an `openai` agent: orbit-mcp, launched by the hub over stdio
with the `mcp` client, as a pydantic-ai toolset."""

import asyncio
import base64
import sys
from typing import TYPE_CHECKING, Any

from mcp import ClientSession, StdioServerParameters, stdio_client
from mcp.shared.exceptions import MCPError
from mcp.types import CONNECTION_CLOSED, REQUEST_TIMEOUT, CallToolResult, ImageContent, TextContent
from pydantic_ai import BinaryContent, RunContext, ToolDefinition, ToolFailed
from pydantic_ai.toolsets import AbstractToolset
from pydantic_ai.toolsets.abstract import ToolsetTool
from pydantic_core import SchemaValidator, core_schema

from ..log import clear_op, report
from ..protocol import HubError

if TYPE_CHECKING:
    from .agents import AgentConfiguration

# orbit-mcp's start: launching Python and answering `initialize` and
# `tools/list`, about one second on the hub's server. 30 s keeps a slow disk
# from failing the first message without holding it for ever.
TOOLS_START_TIMEOUT_SECONDS = 30
# One Orbit tool call. orbit-mcp's longest own wait is the Mac's answer through
# the device bridge (`BRIDGE_TIMEOUT_SECONDS`, 35 s); 60 s leaves it room to
# return its own refusal first. A call that outlives it fails the turn, and its
# outcome stays unknown: it is never repeated.
TOOL_CALL_TIMEOUT_SECONDS = 60

# Arguments arrive as the model's JSON object; orbit-mcp validates them itself
# and answers a refusal the model can read.
_ARGUMENTS = SchemaValidator(
    core_schema.dict_schema(core_schema.str_schema(), core_schema.any_schema())
)


class ToolTransportFailed(Exception):
    """An Orbit tool call that orbit-mcp never answered: the turn ends with it."""

    def __init__(self, tool: str, message: str, numbers: dict[str, int] | None = None):
        super().__init__(message)
        self.tool, self.message, self.numbers = tool, message, numbers


class OrbitTools(AbstractToolset[None]):
    """orbit-mcp, launched over stdio from this hub's own environment.

    One process per device connection. A task of its own owns the stdio
    client: the `mcp` client runs in task groups that must be entered and
    left by the same task, and the connection opens and closes from others.
    """

    def __init__(self, agent: "AgentConfiguration"):
        self.agent, self.settings = agent, agent.settings
        self._session: ClientSession | None = None
        self._tools: dict[str, ToolsetTool[None]] = {}
        self._owner: asyncio.Task | None = None
        self._stop = asyncio.Event()

    @property
    def id(self) -> str:
        return "orbit"

    @property
    def alive(self) -> bool:
        return self._session is not None and self._owner is not None and not self._owner.done()

    def parameters(self) -> StdioServerParameters:
        return StdioServerParameters(
            command=sys.executable,
            args=["-m", "orbit_mcp.cli"],
            # Added to the `mcp` client's short list of safe variables (PATH,
            # HOME…): nothing else of the hub's environment reaches it.
            env={
                "ORBIT_MCP_CONFIG": str(self.settings.orbit_mcp_config),
                # Which agent asks, so orbit-mcp can offer each its own tools;
                # the same variable an official vendor agent's orbit-mcp receives.
                "ORBIT_MCP_AGENT": self.agent.id,
            },
        )

    async def start(self) -> None:
        ready: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        self._owner = asyncio.create_task(self._own(ready))
        try:
            await asyncio.wait_for(asyncio.shield(ready), TOOLS_START_TIMEOUT_SECONDS)
        except TimeoutError:
            await self.stop()
            raise HubError(
                "assistant_tools_unavailable",
                f"orbit-mcp did not start within {TOOLS_START_TIMEOUT_SECONDS * 1000} ms; the assistant has no Orbit tools",
                numbers={"timeout_ms": TOOLS_START_TIMEOUT_SECONDS * 1000},
            ) from None
        except Exception as exc:
            await self.stop()
            report(event="orbit_tools_start_failed", code="assistant_tools_unavailable")
            raise HubError(
                "assistant_tools_unavailable",
                "orbit-mcp exited during startup (the cause stays in the hub log); the assistant has no Orbit tools",
            ) from exc

    async def _own(self, ready: asyncio.Future) -> None:
        clear_op()
        try:
            async with (
                stdio_client(self.parameters()) as (read_stream, write_stream),
                ClientSession(read_stream, write_stream) as session,
            ):
                await session.initialize()
                listing = await session.list_tools()
                self._tools = {
                    tool.name: ToolsetTool(
                        toolset=self,
                        tool_def=ToolDefinition(
                            name=tool.name,
                            description=tool.description or "",
                            parameters_json_schema=tool.input_schema,
                        ),
                        max_retries=1,
                        args_validator=_ARGUMENTS,
                    )
                    for tool in listing.tools
                }
                self._session = session
                ready.set_result(None)
                await self._stop.wait()
        except Exception as exc:  # noqa: BLE001 - the start reports it, or `alive` turns false
            if not ready.done():
                ready.set_exception(exc)
            else:
                report(event="orbit_tools_lost", code="assistant_tools_lost")
        finally:
            self._session = None
            if not ready.done():
                ready.set_exception(RuntimeError("orbit-mcp stopped before it was ready"))

    async def stop(self) -> None:
        """Close orbit-mcp's stdin and wait for it; 5 s, then cancel its owner."""
        self._stop.set()
        owner = self._owner
        if owner is None:
            return
        done, _ = await asyncio.wait({owner}, timeout=5)
        if not done:
            owner.cancel()
            await asyncio.gather(owner, return_exceptions=True)

    async def get_tools(self, ctx: RunContext[None]) -> dict[str, ToolsetTool[None]]:
        return self._tools

    async def call_tool(
        self, name: str, tool_args: dict[str, Any], ctx: RunContext[None], tool: ToolsetTool[None]
    ) -> Any:
        session = self._session
        if session is None or not self.alive:
            raise ToolTransportFailed(
                name, f"Tool {name} could not be used: orbit-mcp is no longer running"
            )
        try:
            result = await session.call_tool(
                name, tool_args, read_timeout_seconds=TOOL_CALL_TIMEOUT_SECONDS
            )
        except MCPError as exc:
            if exc.code == REQUEST_TIMEOUT:
                milliseconds = round(TOOL_CALL_TIMEOUT_SECONDS * 1000)
                raise ToolTransportFailed(
                    name,
                    f"Tool {name} did not respond within {milliseconds} ms; its outcome is uncertain and it will not be repeated",
                    {"timeout_ms": milliseconds},
                ) from exc
            if exc.code == CONNECTION_CLOSED or not self.alive:
                raise ToolTransportFailed(
                    name, f"Tool {name} failed: orbit-mcp exited during the call"
                ) from exc
            raise ToolTransportFailed(
                name,
                f"orbit-mcp refused the call to {name} (code {exc.code})",
                {"mcp_code": exc.code},
            ) from exc
        if not isinstance(result, CallToolResult):
            raise ToolTransportFailed(
                name, f"Tool {name} requested data this assistant cannot provide"
            )
        parts: list[str | BinaryContent] = []
        for block in result.content:
            if isinstance(block, TextContent):
                parts.append(block.text)
            elif isinstance(block, ImageContent) and self.settings.images:
                parts.append(
                    BinaryContent(data=base64.b64decode(block.data), media_type=block.mime_type)
                )
            elif isinstance(block, ImageContent):
                parts.append(
                    "[Orbit: this tool image is omitted because this model does not support images (images = false)]"
                )
        text_only = all(isinstance(part, str) for part in parts)
        if result.is_error:
            # orbit-mcp's refusals are written for the model to read and correct.
            raise ToolFailed("\n".join(part for part in parts if isinstance(part, str)))
        if text_only:
            return "\n".join(parts)
        return parts
