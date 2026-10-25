"""Claude turns through Anthropic's official Python Agent SDK and Claude CLI.

Only the SDK starts and talks to Claude. Orbit never opens account files or
implements Anthropic authentication. Each turn owns its SDK context; later
turns resume the same vendor session from the agent's configuration folder.
"""

import asyncio
import base64
import logging
import os

from claude_agent_sdk import (
    AssistantMessage,
    ClaudeAgentOptions,
    ClaudeSDKClient,
    CLIConnectionError,
    CLIJSONDecodeError,
    PermissionResultAllow,
    PermissionResultDeny,
    ProcessError,
    RateLimitEvent,
    ResultMessage,
    StreamEvent,
    SystemMessage,
    ThinkingBlock,
    ToolResultBlock,
    ToolUseBlock,
    UserMessage,
)
from claude_agent_sdk import TextBlock as SdkTextBlock

from .agent_config import instructions_text
from .backend import ActivityItem, ImageBlock, MessageChunk, MessageComplete, TextBlock
from .claude_transport import ConfiguredTransport
from .deletion import command_deletion
from .official_tools import MAX_AGENT_MESSAGE_BYTES, OPEN_TIMEOUT_SECONDS, VendorProcess
from .tool_availability import launch_failure


# SDK diagnostics may contain a malformed CLI frame or stderr. A filter is
# required: uvicorn's dictConfig can re-enable previously disabled loggers.
# Suppress before dispatch to *any* handler, including root/Sentry/capture.
class _NoDiagnostics(logging.Filter):
    def filter(self, record):
        return False


for _name in (
    "claude_agent_sdk",
    "claude_agent_sdk._internal.query",
    "claude_agent_sdk._internal.transport.subprocess_cli",
    "claude_agent_sdk._internal.message_parser",
):
    logging.getLogger(_name).addFilter(_NoDiagnostics())


class ClaudeProcess(VendorProcess):
    async def open(self):
        self.client = None
        self.ready = asyncio.Event()
        self.tools = {}

    def start_turn(self, blocks):
        if self.turn is not None:
            raise self.refusal("turn_in_progress", "ya tiene un turno en curso")
        self.ready.clear()
        self.finishing = False
        super().start_turn(blocks)

    def options(self):
        # SDK env is merged over os.environ, so explicitly blank everything
        # outside the allowlist. Never let ambient API credentials supersede
        # this agent's signed-in account or enable diagnostic transcripts.
        variables = dict.fromkeys(os.environ, "")
        variables.update(self.backend.environment())
        return ClaudeAgentOptions(
            cli_path=self.backend.settings.command[0],
            cwd=self.backend.folder,
            env=variables,
            session_id=None if self.resuming else self.session_id,
            resume=self.session_id if self.resuming else None,
            tools=["AskUserQuestion"],
            setting_sources=[],
            strict_mcp_config=True,
            mcp_servers={"orbit": self.backend.mcp_server()},
            system_prompt={
                "type": "preset",
                "preset": "claude_code",
                "append": instructions_text(self.backend.instructions()) or "",
            },
            permission_mode="default",
            can_use_tool=self.can_use_tool,
            include_partial_messages=True,
            max_buffer_size=MAX_AGENT_MESSAGE_BYTES,
            stderr=lambda line: None,
        )

    async def execute(self, blocks):
        self.tools, self.streamed, self.error = {}, False, None
        options = self.options()
        transport = ConfiguredTransport(self.backend.settings.command, options)
        self.client = ClaudeSDKClient(options=options, transport=transport)
        try:
            # Start and close the SDK from this task: its structured resources
            # must not cross task ownership. No whole-turn timeout is applied.
            async with asyncio.timeout(OPEN_TIMEOUT_SECONDS):
                await self.client.connect()
                await self.client.query(self.prompt_blocks(blocks), session_id=self.session_id)
            self.ready.set()
            async for message in self.client.receive_response():
                if isinstance(message, ResultMessage):
                    self.finishing = True
                    self.resuming = True
                    return self.result(message)
                await self.update(message)
            raise self.refusal("agent_stopped", "cerró su salida sin terminar el turno")
        except TimeoutError:
            raise self.refusal(
                "agent_timeout",
                f"no abrió el turno en {OPEN_TIMEOUT_SECONDS} segundos",
                {"timeout_ms": OPEN_TIMEOUT_SECONDS * 1000},
            ) from None
        except ProcessError as exc:
            numbers = {"exit_status": exc.exit_code} if type(exc.exit_code) is int else None
            raise self.refusal(
                "agent_stopped", "terminó su proceso durante el turno", numbers
            ) from None
        except CLIConnectionError as exc:
            if isinstance(exc.__cause__, OSError):
                raise launch_failure(
                    "claude-code", self.backend.settings.command[0], exc.__cause__
                ) from None
            raise self.refusal(
                "agent_start_failed", f"no pudo conectar con {self.backend.settings.command[0]}"
            ) from None
        except CLIJSONDecodeError as exc:
            # The pinned SDK's overrun is an explicit framing error. Use
            # only that fixed marker, never the arbitrary frame it received.
            if exc.line.startswith("JSON message exceeded maximum buffer size of "):
                raise self.refusal(
                    "agent_stopped",
                    f"envió un mensaje mayor de {MAX_AGENT_MESSAGE_BYTES} bytes",
                    {"max_bytes": MAX_AGENT_MESSAGE_BYTES},
                ) from None
            raise self.refusal(
                "agent_protocol",
                "envió un mensaje JSON inválido",
            ) from None
        finally:
            self.ready.set()
            await self.client.disconnect()
            self.client = None

    def prompt_blocks(self, blocks):
        async def prompt():
            content = []
            for block in blocks:
                if isinstance(block, TextBlock):
                    content.append({"type": "text", "text": block.text})
                elif isinstance(block, ImageBlock):
                    content.append(
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": block.mime,
                                "data": base64.b64encode(block.data).decode(),
                            },
                        }
                    )
            yield {
                "type": "user",
                "message": {"role": "user", "content": content},
                "parent_tool_use_id": None,
                "session_id": self.session_id,
            }

        return prompt()

    def result(self, message):
        if self.error == "authentication_failed":
            raise self.auth_failure()
        if self.error == "rate_limit" or message.api_error_status == 429:
            raise self.limit_failure()
        if message.terminal_reason in {"aborted_streaming", "aborted_tools"}:
            return "interrupted"
        causes = {
            "billing_error": "no pudo usar la facturación de la cuenta",
            "invalid_request": "rechazó el formato de la petición",
            "server_error": "informó de un fallo del servicio de Claude",
            "model_capacity": "no tiene capacidad disponible para ese modelo",
        }
        if self.error == "max_output_tokens":
            raise self.refusal(
                "agent_turn_unfinished",
                "agotó sus tokens de respuesta",
                {"stop_reason": "max_tokens"},
            )
        if self.error in causes:
            raise self.refusal("agent_rejected", causes[self.error], {"stop_reason": self.error})
        if message.subtype not in {
            "success",
            "error_during_execution",
            "error_max_turns",
            "error_max_budget_usd",
            "error_max_structured_output_retries",
        }:
            raise self.refusal("agent_protocol", "envió un tipo de resultado desconocido")
        if message.subtype != "success":
            raise self.refusal(
                "agent_turn_unfinished", "paró antes de terminar", {"stop_reason": message.subtype}
            )
        if message.is_error:
            numbers = (
                {"provider_status": message.api_error_status}
                if type(message.api_error_status) is int
                else None
            )
            raise self.refusal("agent_rejected", "no terminó el turno", numbers)
        return "completed"

    async def update(self, message):
        if isinstance(message, SystemMessage) and message.subtype == "init":
            if message.data.get("session_id") != self.session_id:
                raise self.refusal("agent_protocol", "cambió el id de la conversación")
            self.resuming = True
        elif isinstance(message, RateLimitEvent):
            info = message.rate_limit_info
            reset = info.resets_at * 1000 if info.resets_at is not None else None
            if info.status == "rejected":
                self.limit_resets_ms = reset
            if info.utilization is not None and 0 <= info.utilization <= 1:
                self.backend.observe(
                    "usage",
                    used_percent=round(info.utilization * 100),
                    window=info.rate_limit_type,
                    resets_at_ms=reset,
                )
        elif isinstance(message, StreamEvent) and message.parent_tool_use_id is None:
            if message.event.get("type") == "message_start":
                self.streamed = False
            delta = message.event.get("delta", {})
            if delta.get("type") == "text_delta" and isinstance(delta.get("text"), str):
                self.streamed = True
                await self.chunk(delta["text"])
        elif isinstance(message, AssistantMessage) and message.parent_tool_use_id is None:
            self.error = message.error or self.error
            for block in message.content:
                if isinstance(block, SdkTextBlock) and not self.streamed and not message.error:
                    await self.chunk(block.text)
                elif isinstance(block, ToolUseBlock):
                    self.tools[block.id] = (block.name, block.input)
                    await self.connection.emit(
                        ActivityItem(
                            self.session_id, block.id, "tool", "started", block.name, block.input
                        )
                    )
                elif isinstance(block, ThinkingBlock):
                    await self.connection.emit(
                        ActivityItem(
                            self.session_id,
                            "reasoning",
                            "reasoning",
                            "completed",
                            text=block.thinking,
                        )
                    )
        elif isinstance(message, UserMessage):
            for block in message.content if isinstance(message.content, list) else []:
                if isinstance(block, ToolResultBlock):
                    name, arguments = self.tools.pop(block.tool_use_id, ("herramienta", None))
                    await self.connection.emit(
                        ActivityItem(
                            self.session_id,
                            block.tool_use_id,
                            "tool",
                            "failed" if block.is_error else "completed",
                            name,
                            arguments,
                            cause="La herramienta falló" if block.is_error else None,
                        )
                    )

    async def chunk(self, text):
        self.text.append(text)
        await self.connection.emit(MessageChunk(self.session_id, text))

    async def can_use_tool(self, tool, arguments, context):
        # Orbit's MCP access groups and local system permissions already
        # decide these effects. Do not ask for the same permission twice.
        # strict_mcp_config ensures this namespace is our configured server.
        if tool.startswith("mcp__orbit__"):
            return PermissionResultAllow(updated_input=arguments)
        if tool == "AskUserQuestion":
            questions = arguments.get("questions", [])
            answers = await self.interactions.clarify(questions)
            if not answers:
                return PermissionResultDeny(message="Turno interrumpido", interrupt=True)
            mapped = {q["question"]: answers[str(q.get("id", i))] for i, q in enumerate(questions)}
            return PermissionResultAllow(updated_input={**arguments, "answers": mapped})
        action = arguments.get("command") or arguments.get("file_path") or context.title or tool
        description = f"Permiso para {tool}: {action}"
        deletion = command_deletion(arguments.get("command", ""))
        choice = await self.interactions.permission(description, deletion)
        if choice == "once":
            return PermissionResultAllow(updated_input=arguments)
        return PermissionResultDeny(message="La persona no dio permiso")

    async def cancel(self):
        if self.turn is None:
            return
        if not self.ready.is_set():
            # No SDK handshake yet: cancelling the task closes its owned
            # client/CLI before acknowledging the stop. Do not wait for the
            # CLI to initialize and then submit a question already stopped.
            task = self.turn
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
            if self.turn is task:
                self.turn = None
            await self.connection.emit(
                MessageComplete(self.session_id, "".join(self.text), "interrupted")
            )
            return
        if self.finishing:
            return
        async with asyncio.timeout(OPEN_TIMEOUT_SECONDS):
            await self.ready.wait()
            if self.client is not None:
                await self.client.interrupt()
        await self.interactions.expire()
