"""Claude Code and Codex as the hub's assistant, over the Agent Client Protocol.

The hub is an ACP v1 client (`agent-client-protocol`, the Python SDK). Each
active session is one process of the adapter the operator installed
(`@agentclientprotocol/claude-agent-acp` or `@agentclientprotocol/codex-acp`),
spoken to over its stdio:

    initialize           once per process; the hub declares neither `fs` nor `terminal`
    session/new          orbit-mcp as the one MCP server, the agent's folder as `cwd`
    session/load         to resume, when the agent declares `loadSession`
    session/prompt       the turn's text and images; its stop reason ends the turn
    session/cancel       a cancellation; pending permissions answer `cancelled`
    request_permission   the Mac's approval, answered with the option it chose
    session/update       message chunks, tool calls, plans and thoughts, as typed events

Not declaring `fs` and `terminal` does not take Claude Code's or Codex's own
tools away: both run them inside their process. Each agent is therefore also
locked down with its own options (`claude_lock_down`, `codex_lock_down`), and
its files live in a folder of its own under the hub's data directory.

The personality and enabled skills the hub keeps (`agent_config`) reach each
agent as the same text, `instructions_text`, where its lock-down still reads
it: Claude Code in each session's `_meta.systemPrompt.append` (with
`settingSources: []` it loads no `CLAUDE.md` or skills folder, and with
`tools: []` it could not open one), Codex in the `AGENTS.md` of its folder.
A skill's instructions come from orbit-mcp's `orbit_skill`.

The agent signs in with the vendor's own tool, run by the hub in the agent's
folder (`sign_in`); the hub never sees a token. What the agent says of its
account reaches the registry through `AcpBackend.observe`: a refusal for want
of a sign-in (the session expired), the plan's limit (`agent_limit_reached`,
with its reset time when the adapter forwards it), the plan's use (Claude
Code's `_claude/rateLimit` in `usage_update`, the one structured report
either adapter gives) and the account's address and plan
(`_auth/status_update`).
"""

import asyncio
import base64
import json
import os
import signal
import sys
from collections.abc import Sequence
from contextlib import suppress
from pathlib import Path
from typing import TYPE_CHECKING, Any

import acp
from acp.core import ClientSideConnection
from acp.schema import (
    ClientCapabilities,
    EnvVariable,
    FileSystemCapabilities,
    ImageContentBlock,
    Implementation,
    McpServerStdio,
    PermissionOption,
    TextContentBlock,
)

from ..errors import HubError
from ..log import clear_op, report
from .acp_client import AcpClient
from .acp_connection import AcpConnection
from .acp_instructions import write_instructions
from .acp_settings import HOME_FOLDERS, AcpAgentSettings
from .acp_transport import LineTransport
from .agent_config import (
    MAX_INSTRUCTIONS_TEXT_BYTES,
    instructions_text,
    no_instructions,
)
from .backend import (
    Capabilities,
    ContentBlock,
    EventSink,
    ImageBlock,
    MessageComplete,
    TextBlock,
    TurnFailed,
)

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

KIND = "acp"

# A personal hub serves a few Macs and a few conversations at once, and each
# agent process is a Node adapter plus the agent's own binary (several hundred
# MB together: estimated from the packages, not measured here). This bounds
# the host memory and CPU one agent's processes take. It does not bound
# sessions: an idle session gives its process up and is reloaded with
# `session/load` on its next turn.
MAX_AGENT_PROCESSES = 4
# One ACP message from an agent, which is one line of JSON. A tool update can
# carry orbit-mcp's screenshots (700,000 bytes of PNG each, 933,336 in
# base64), so 8 MiB leaves room for several. It bounds the hub's memory per
# message; it does not limit a turn's length or the conversation.
MAX_AGENT_MESSAGE_BYTES = 8 * 1024 * 1024
# Not measured against the real adapters (no Claude Code or Codex runs in
# development): generous budgets for a Node start-up, and for the agent's own
# start of its binary and MCP servers or the replay of a long conversation.
# They bound those handshakes, never a turn.
START_TIMEOUT_SECONDS = 30
OPEN_TIMEOUT_SECONDS = 120
# The SDK's own grace between closing a process's stdin and signalling it.
STOP_GRACE_SECONDS = 2
# The agent's own words in a refusal shown to the user: one line, not a dump.
MAX_AGENT_REASON_CHARS = 300

AUTH_REQUIRED = acp.RequestError.auth_required().code
# ACP's stop reasons for a turn the agent ended before finishing its answer.
UNFINISHED = {
    "max_tokens": "llegó a su máximo de tokens de respuesta antes de terminar",
    "max_turn_requests": "llegó a su máximo de peticiones al modelo en un turno antes de terminar",
    "refusal": "se negó a seguir con el turno",
}


def build_backend(
    hub: "Settings", agent: "AgentConfiguration", instructions=no_instructions
) -> "AcpBackend":
    """The agent's backend (`agents.KINDS`), refused at startup when its
    command cannot start. `instructions` reads the personality and enabled
    skills the agent is told as each session opens."""
    path = Path(agent.settings.command[0])
    if not path.exists():
        cause = "does not exist"
    elif not path.is_file():
        cause = "is not a file"
    elif not os.access(path, os.X_OK):
        cause = "is not executable"
    else:
        return AcpBackend(agent, hub, instructions)
    raise ValueError(f"agent {agent.id!r}: `command` {path} {cause}")


def claude_lock_down() -> dict:
    """Claude Code's Agent SDK options, which claude-agent-acp reads from
    `_meta.claudeCode.options`: no built-in tools, only the MCP servers ACP
    names (orbit-mcp), and no user, project or local settings files."""
    return {"claudeCode": {"options": {"tools": [], "strictMcpConfig": True, "settingSources": []}}}  # fmt: skip


def codex_lock_down() -> dict:
    """Codex's configuration, which codex-acp merges into each session's
    (`CODEX_CONFIG`): no shell, a read-only sandbox, no approvals, no web search."""
    return {
        "features": {"shell_tool": False},
        "sandbox_mode": "read-only",
        "approval_policy": "never",
        "web_search": "disabled",
    }


class AcpBackend:
    """One ACP agent and its processes, at most `MAX_AGENT_PROCESSES` at once."""

    # Images go as ACP image blocks to an agent that declares
    # `promptCapabilities.image`; to one that does not, the turn names them
    # for `orbit_attachment` instead (`AgentProcess.turn_prompt`). Resume is
    # `session/load`, refused with its cause by an agent that lacks it. History:
    # the agent keeps each conversation and replays it itself on load, which is
    # over when it answers, so the resume says so and no `HistoryLoaded` follows.
    # Voice: ACP carries no audio.
    capabilities = Capabilities(images=True, resume=True, history=True, voice=False)

    def __init__(self, agent: "AgentConfiguration", hub: "Settings", instructions=no_instructions):
        self.agent_id = agent.id
        # The agent as the user knows it, for the texts that name it.
        self.agent_name = agent.name
        self.settings: AcpAgentSettings = agent.settings
        self.instructions = instructions
        self.folder = hub.data_dir / "agents" / agent.id
        self.config_dir = self.folder / HOME_FOLDERS[self.settings.agent]
        # Every running process, the one used longest ago first.
        self.processes: list[AgentProcess] = []
        self._admission = asyncio.Lock()
        # What the agent's processes learn of its account goes to the
        # registry (`Agents.observe`), which sets this.
        self.observe = _ignore

    def rename(self, name: str) -> None:
        """The profile's name, for the texts that name the agent from now on."""
        self.agent_name = name

    def connection(self, device: str, device_name: str, on_event: EventSink) -> "AcpConnection":
        return AcpConnection(self, on_event)

    def environment(self) -> dict[str, str]:
        environment = acp.default_environment()
        environment.update(
            {name: os.environ[name] for name in self.settings.env if name in os.environ}
        )
        if self.settings.agent == "claude-code":
            environment["CLAUDE_CONFIG_DIR"] = str(self.config_dir)
        else:
            environment["CODEX_HOME"] = str(self.config_dir)
            # And the whole AGENTS.md the hub writes, which past its default
            # 32 KiB Codex would cut without a word.
            environment["CODEX_CONFIG"] = json.dumps(
                {**codex_lock_down(), "project_doc_max_bytes": MAX_INSTRUCTIONS_TEXT_BYTES}
            )
            # codex-acp sends its mode's sandbox and approval policy with every
            # turn, over `sandbox_mode` and `approval_policy`; this is the mode
            # whose sandbox is read-only. Its approval policy is `on-request`.
            environment["INITIAL_AGENT_MODE"] = "read-only"
        return environment

    def mcp_servers(self) -> list[McpServerStdio]:
        # The translator of this same hub version, which the agent starts.
        return [
            McpServerStdio(
                name="orbit",
                command=sys.executable,
                args=["-m", "orbit_mcp.cli"],
                env=[
                    EnvVariable(name="ORBIT_MCP_CONFIG", value=str(self.settings.orbit_mcp_config)),
                    EnvVariable(name="ORBIT_MCP_AGENT", value=self.agent_id),
                ],
            )
        ]

    def session_meta(self) -> dict:
        """What `session/new` and `session/load` carry in `_meta`: for Claude
        Code, its lock-down and what it is told. claude-agent-acp 0.94.0 makes
        `_meta.systemPrompt` the Agent SDK's `{type: "preset", preset:
        "claude_code", ...}`, so `append` follows Claude Code's own prompt."""
        if self.settings.agent != "claude-code":
            return {}
        meta = claude_lock_down()
        told = instructions_text(self.instructions())
        if told is not None:
            meta["systemPrompt"] = {"append": told}
        return meta

    def prepare_folder(self) -> None:
        for path in (self.folder.parent, self.folder, self.config_dir):
            if path.is_symlink() or (path.exists() and not path.is_dir()):
                raise HubError(
                    "agent_folder_invalid",
                    f"La carpeta del agente «{self.agent_name}» no es un directorio: {path}",
                )
            if not path.exists():
                path.mkdir(mode=0o700)
                # mkdir's mode passes through the umask; the folder is private.
                path.chmod(0o700)

    async def start(self, connection: "AcpConnection") -> "AgentProcess":
        """A started, initialized process, retiring an idle one at the ceiling."""
        async with self._admission:
            if len(self.processes) >= MAX_AGENT_PROCESSES:
                idle = [process for process in self.processes if process.idle]
                if not idle:
                    raise HubError(
                        "agent_process_limit",
                        f"El agente «{self.agent_name}» tiene {len(self.processes)} "
                        f"procesos con un turno en curso; techo {MAX_AGENT_PROCESSES}. "
                        "Espera a que termine uno",
                        numbers={"busy": len(self.processes), "limit": MAX_AGENT_PROCESSES},
                    )
                await idle[0].stop()
            self.prepare_folder()
            if self.settings.agent == "codex":
                write_instructions(self.folder, self.instructions())
            process = AgentProcess(self, connection)
            self.processes.append(process)
        try:
            await process.start()
        except BaseException:
            await process.stop()
            raise
        return process

    def used(self, process: "AgentProcess") -> None:
        if process in self.processes:
            self.processes.remove(process)
            self.processes.append(process)


def _ignore(fact: str, **values: Any) -> None:
    return None


class AgentProcess(AcpClient):
    """One adapter process holding one ACP session; the SDK's `Client` for it."""

    def __init__(self, backend: AcpBackend, connection: "AcpConnection"):
        self.backend, self.connection = backend, connection
        # When the plan's limit resets, if the adapter said in this turn.
        self.limit_resets_ms: int | None = None
        self.process: asyncio.subprocess.Process | None = None
        self.transport: LineTransport | None = None
        self.peer: ClientSideConnection | None = None
        self.watcher: asyncio.Task | None = None
        self.can_load = False
        # The agent declared `promptCapabilities.image` when it started.
        self.can_image = False
        self.session_id: str | None = None
        self.turn: asyncio.Task | None = None
        self.text: list[str] = []
        # This turn's tool calls, by id: their name and arguments, which only
        # `tool_call` carries, for the label of each `tool_call_update`.
        self.tools: dict[str, tuple[str, dict | None]] = {}
        # The thought being streamed, ended by the next other update.
        self.thought: list[str] = []
        self.thought_id: str | None = None
        self.planned = False
        # Permission requests the agent waits on: request id -> (answer, options).
        self.permissions: dict[str, tuple[asyncio.Future, list[PermissionOption]]] = {}
        self.stopped = False

    @property
    def idle(self) -> bool:
        return self.session_id is not None and self.turn is None and not self.permissions

    def refusal(self, code: str, cause: str, numbers: dict[str, int] | None = None) -> HubError:
        command = self.backend.settings.command[0]
        return HubError(code, f"El agente «{self.name}» ({command}) {cause}", numbers=numbers)

    async def start(self) -> None:
        from ..app import hub_version

        try:
            self.process = await asyncio.create_subprocess_exec(
                *self.backend.settings.command,
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                # The agent's diagnostics can quote the conversation: never logged.
                stderr=asyncio.subprocess.DEVNULL,
                env=self.backend.environment(),
                cwd=self.backend.folder,
                # Its own process group, so stopping it reaches what it started.
                start_new_session=True,
                limit=MAX_AGENT_MESSAGE_BYTES,
            )
        except FileNotFoundError as exc:
            raise self.refusal("agent_start_failed", "no arranca: no existe") from exc
        except PermissionError as exc:
            raise self.refusal("agent_start_failed", "no arranca: sin permiso de ejecución") from exc  # fmt: skip
        except OSError as exc:
            raise self.refusal("agent_start_failed", f"no arranca: {exc.strerror}") from exc
        self.transport = LineTransport(self.process.stdout, self.process.stdin)
        self.peer = ClientSideConnection(self, self.transport)
        self.watcher = asyncio.create_task(self.watch())
        try:
            answer = await self.request(
                "initialize",
                self.peer.initialize(
                    protocol_version=acp.PROTOCOL_VERSION,
                    client_capabilities=ClientCapabilities(
                        fs=FileSystemCapabilities(read_text_file=False, write_text_file=False),
                        terminal=False,
                    ),
                    client_info=Implementation(
                        name="orbit-hub", title="Orbit", version=hub_version()
                    ),
                ),
                START_TIMEOUT_SECONDS,
            )
        except HubError as exc:
            raise HubError("agent_start_failed", exc.message) from exc
        if answer.protocol_version != acp.PROTOCOL_VERSION:
            raise self.refusal(
                "agent_start_failed",
                f"habla la versión {answer.protocol_version} de ACP; Orbit habla la "
                f"{acp.PROTOCOL_VERSION}",
            )
        capabilities = answer.agent_capabilities
        self.can_load = bool(capabilities and capabilities.load_session)
        prompts = capabilities.prompt_capabilities if capabilities else None
        self.can_image = bool(prompts and prompts.image)

    async def watch(self) -> None:
        """An agent that ends between turns gives its session up, to be reloaded."""
        await self.process.wait()
        if self.turn is None:
            await self.stop()

    async def request(self, method: str, call, timeout: float):
        """One request to the agent, its refusals named with their cause."""
        try:
            return await asyncio.wait_for(call, timeout)
        except acp.RequestError as exc:
            if exc.code == AUTH_REQUIRED:
                self.backend.observe("auth_required")
                raise self.refusal("agent_auth_required", self.sign_in_cause()) from exc
            raise self.refusal(
                "agent_rejected", f"rechazó {method} (código {exc.code}): {_reason(exc)}"
            ) from exc
        except TimeoutError as exc:
            raise self.refusal(
                "agent_timeout", f"no respondió a {method} en {timeout} segundos"
            ) from exc
        except ConnectionError as exc:
            raise self.refusal("agent_stopped", *await self.exit_cause(method)) from exc

    def sign_in_cause(self) -> str:
        service = "Claude" if self.backend.settings.agent == "claude-code" else "ChatGPT"
        return (
            f"no tiene la sesión de {service} iniciada en el hub, o caducó: inicia sesión "
            "desde Orbit, en sus ajustes"
        )

    async def exit_cause(self, method: str) -> tuple[str, dict[str, int] | None]:
        """Why the agent's output ended, and its numbers."""
        if self.transport is not None and self.transport.overrun:
            cause = (
                f"envió un mensaje de más de {MAX_AGENT_MESSAGE_BYTES} bytes, el techo de "
                "un mensaje; se detuvo el agente"
            )
            return cause, {"max_bytes": MAX_AGENT_MESSAGE_BYTES}
        code = None
        if self.process is not None:
            with suppress(TimeoutError):
                code = await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
        if code is None:
            return f"cerró su salida durante {method}; se detuvo el agente", None
        return f"terminó con código {code} durante {method}", {"exit_status": code}

    async def new_session(self) -> str:
        answer = await self.request(
            "session/new",
            self.peer.new_session(
                cwd=str(self.backend.folder),
                mcp_servers=self.backend.mcp_servers(),
                **self.backend.session_meta(),
            ),
            OPEN_TIMEOUT_SECONDS,
        )
        self.session_id = answer.session_id
        return answer.session_id

    async def load_session(self, stored_id: str) -> None:
        if not self.can_load:
            raise self.refusal(
                "session_resume_unsupported",
                "no declara session/load, así que no puede reanudar una conversación guardada; "
                "el historial local sigue disponible",
            )
        await self.request(
            "session/load",
            self.peer.load_session(
                cwd=str(self.backend.folder),
                session_id=stored_id,
                mcp_servers=self.backend.mcp_servers(),
                **self.backend.session_meta(),
            ),
            OPEN_TIMEOUT_SECONDS,
        )
        self.session_id = stored_id

    def turn_prompt(self, blocks: Sequence[ContentBlock]) -> list:
        """The turn as ACP content: its text, then its images when the agent
        takes them; otherwise the text names them for `orbit_attachment`."""
        text = "".join(block.text for block in blocks if isinstance(block, TextBlock))
        images = [block for block in blocks if isinstance(block, ImageBlock)]
        if images and not self.can_image:
            named = "; ".join(f"«{image.name}» (id {image.attachment_id})" for image in images)
            text += (
                f"\n[Este agente no declara recibir imágenes, así que no van en el mensaje; "
                f"míralas con orbit_attachment: {named}]"
            )
            images = []
        return [
            TextContentBlock(type="text", text=text),
            *(
                ImageContentBlock(
                    type="image", data=base64.b64encode(image.data).decode(), mime_type=image.mime
                )
                for image in images
            ),
        ]

    def start_turn(self, prompt: list) -> None:
        if self.turn is not None:
            raise HubError(
                "turn_in_progress", f"El agente «{self.name}» ya tiene un turno en curso"
            )
        self.text = []
        self.tools, self.thought, self.thought_id, self.planned = {}, [], None, False
        self.limit_resets_ms = None
        self.backend.used(self)
        self.turn = asyncio.create_task(self.run_turn(prompt))

    async def run_turn(self, prompt: list) -> None:
        clear_op()
        session_id = self.session_id
        try:
            answer = await self.peer.prompt(session_id=session_id, prompt=prompt)
        except acp.RequestError as exc:
            if exc.code == AUTH_REQUIRED:
                self.backend.observe("auth_required")
                await self.fail(session_id, "agent_auth_required", self.sign_in_cause())
            elif _is_limit(exc):
                until = self.limit_resets_ms
                self.backend.observe("limit", until_ms=until)
                await self.fail(
                    session_id,
                    "agent_limit_reached",
                    f"llegó al límite de tu plan: {_limit_reason(exc)}",
                    {"resets_at_ms": until} if until is not None else None,
                )
            else:
                cause = f"rechazó el turno (código {exc.code}): {_reason(exc)}"
                await self.fail(session_id, "agent_rejected", cause, {"acp_code": exc.code})
        except ConnectionError:
            await self.fail(session_id, "agent_stopped", *await self.exit_cause("el turno"))
        except Exception:  # noqa: BLE001 - the turn must end, whatever the agent answered
            await self.fail(session_id, "agent_protocol", "respondió al turno algo que el hub no lee")  # fmt: skip
        else:
            await self.end_thought(session_id)
            status = {"end_turn": "completed", "cancelled": "interrupted"}.get(answer.stop_reason)
            if status is None:
                # The SDK admits only ACP's stop reasons, so the value is bounded.
                cause = UNFINISHED.get(answer.stop_reason, "paró el turno sin terminarlo")
                await self.fail(
                    session_id,
                    "agent_turn_unfinished",
                    f"{cause} (stopReason {answer.stop_reason})",
                    {"stop_reason": answer.stop_reason},
                )
            else:
                if status == "completed":
                    self.backend.observe("answered")
                await self.connection.emit(MessageComplete(session_id, "".join(self.text), status))
        finally:
            self.turn = None
            await self.expire_permissions()
            if self.process.returncode is not None:
                await self.stop()

    async def fail(
        self, session_id: str, code: str, cause: str, numbers: dict[str, int | str] | None = None
    ) -> None:
        """End the turn without an answer, for its cause: what the agent said
        before it stays live on the Mac, never stored as the answer."""
        report("acp_turn_failed", code, numbers=numbers, exc_info=False)
        if code in {"agent_stopped", "agent_protocol"}:
            await self.stop()
        await self.connection.emit(
            TurnFailed(session_id, code, f"El agente «{self.name}» {cause}", numbers)
        )

    async def cancel(self) -> None:
        try:
            await self.peer.cancel(session_id=self.session_id)
        except ConnectionError as exc:
            raise self.refusal("agent_stopped", "ya no está en marcha") from exc
        # ACP: the client answers each pending permission of a cancelled turn `cancelled`.
        await self.expire_permissions()

    async def stop(self) -> None:
        if self.stopped:
            return
        self.stopped = True
        if self in self.backend.processes:
            self.backend.processes.remove(self)
        self.connection.forget(self)
        await self.expire_permissions()
        if self.process is not None:
            if self.transport is not None:
                await self.transport.close()
            try:
                await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
            except TimeoutError:
                _signal_group(self.process.pid, signal.SIGTERM)
                try:
                    await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
                except TimeoutError:
                    _signal_group(self.process.pid, signal.SIGKILL)
                    await self.process.wait()
            # What the adapter started (its agent binary, orbit-mcp) ends with it.
            _signal_group(self.process.pid, signal.SIGTERM)
        if self.peer is not None:
            await self.peer.close()
        current = asyncio.current_task()
        for task in (self.turn, self.watcher):
            if task is not None and task is not current:
                await asyncio.gather(task, return_exceptions=True)


# What tells a plan's limit apart from other refusals of a turn: Codex's
# `codexErrorInfo` and Claude Code's `errorKind`, in the `-32603` refusal's
# data (codex-acp 2.2.2, claude-agent-acp 0.94.0). Neither carries the reset
# time: Codex writes it only in its sentence, Claude Code in `rateLimit`.
LIMIT_MARKS = (("codexErrorInfo", "usageLimitExceeded"), ("errorKind", "rate_limit"))


def _is_limit(exc: acp.RequestError) -> bool:
    data = exc.data if isinstance(exc.data, dict) else {}
    return any(data.get(key) == value for key, value in LIMIT_MARKS)


def _limit_reason(exc: acp.RequestError) -> str:
    """The agent's own sentence, which says when to try again («Try again at
    3:04 PM»), bounded like any reason of an agent."""
    data = exc.data if isinstance(exc.data, dict) else {}
    message = data.get("message")
    text = message if isinstance(message, str) and message.strip() else str(exc)
    text = " ".join(text.split())
    if len(text) <= MAX_AGENT_REASON_CHARS:
        return text
    return text[: MAX_AGENT_REASON_CHARS - 1] + "…"


def _reason(exc: acp.RequestError) -> str:
    text = str(exc)
    if len(text) <= MAX_AGENT_REASON_CHARS:
        return text
    return text[: MAX_AGENT_REASON_CHARS - 1] + "…"


def _signal_group(pid: int, signal_number: int) -> None:
    with suppress(ProcessLookupError, PermissionError):
        os.killpg(pid, signal_number)
