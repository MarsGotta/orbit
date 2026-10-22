"""Claude Code and Codex as the hub's assistant, over the Agent Client Protocol.

The hub is an ACP v1 client (`agent-client-protocol`, the Python SDK). Each
active session is one process of the adapter the operator installed
(`@agentclientprotocol/claude-agent-acp` or `@agentclientprotocol/codex-acp`),
spoken to over its stdio:

    initialize           once per process; the hub declares neither `fs` nor `terminal`
    session/new          orbit-mcp as the one MCP server, the agent's folder as `cwd`
    session/load         to resume, when the agent declares `loadSession`
    session/prompt       the turn's text; its stop reason ends the turn
    session/cancel       a cancellation; pending permissions answer `cancelled`
    request_permission   the Mac's approval, answered with the option it chose
    session/update       message chunks, tool calls, plans and thoughts, as typed events

Not declaring `fs` and `terminal` does not take Claude Code's or Codex's own
tools away: both run them inside their process. Each agent is therefore also
locked down with its own options (`claude_lock_down`, `codex_lock_down`), and
its files live in a folder of its own under the hub's data directory.

The account is the operator's: Orbit never offers a sign-in and never sees a
token. An agent that asks to authenticate is refused with the command to run
on the server.
"""

import asyncio
import json
import os
import re
import shlex
import shutil
import signal
import sys
from collections.abc import Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal
from uuid import uuid4

import acp
from acp.core import ClientSideConnection
from acp.schema import (
    AgentMessageChunk,
    AgentPlanUpdate,
    AgentThoughtChunk,
    AllowedOutcome,
    ClientCapabilities,
    DeniedOutcome,
    EnvVariable,
    FileSystemCapabilities,
    Implementation,
    McpServerStdio,
    PermissionOption,
    RequestPermissionResponse,
    TextContentBlock,
    ToolCallProgress,
    ToolCallStart,
)

from ..errors import HubError
from ..log import clear_op, report
from .agent_config import AgentInstructions, no_instructions
from .backend import (
    ActivityItem,
    Capabilities,
    ClarificationReceipt,
    ContentBlock,
    EventSink,
    MessageChunk,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    PlanStep,
    RequestExpired,
    ResumedSession,
    UnreadableUpdate,
)
from .skills import NAME_CHARACTERS

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

KIND = "acp"

AgentName = Literal["claude-code", "codex"]
AGENTS: tuple[AgentName, ...] = ("claude-code", "codex")

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

VARIABLE_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# What Orbit itself sets in an agent's environment; `env` cannot replace it.
LOCK_DOWN_VARIABLES = ("CLAUDE_CONFIG_DIR", "CODEX_HOME", "CODEX_CONFIG", "INITIAL_AGENT_MODE")
# An entry's own keys: `id`, `name` and `kind` belong to every `[[agents]]`
# entry, and the registry checks them.
KEYS = {"agent", "command", "env", "orbit_mcp_config"}
AUTH_REQUIRED = acp.RequestError.auth_required().code
# JSON-RPC's "method not found", for what the hub does not offer an agent.
NOT_OFFERED = -32601


@dataclass(frozen=True)
class AcpAgentSettings:
    """An `[[agents]]` entry's own keys for kind `acp`; the agent's `id` is the
    registry's (`agents.AgentConfiguration`)."""

    agent: AgentName
    # The adapter to start, absolute path first; the operator installs it.
    command: tuple[str, ...]
    # The translator's own TOML, given to orbit-mcp as `ORBIT_MCP_CONFIG`.
    orbit_mcp_config: Path
    # Names of variables passed from the hub's environment, such as an API key.
    env: tuple[str, ...] = ()


def parse_settings(raw: Mapping[str, Any]) -> AcpAgentSettings:
    """An `[[agents]]` entry's own keys for kind `acp` (all but `id`, `name`
    and `kind`), or a refusal naming the key and why. It reads no file: the
    command is checked by `build_backend` at startup."""
    if unknown := sorted(set(raw) - KEYS):
        raise ValueError(f"unknown keys for kind acp ({len(unknown)}): {', '.join(unknown)}")
    agent = raw.get("agent")
    if agent is None:
        raise ValueError('`agent` is missing; it must be "claude-code" or "codex"')
    if agent not in AGENTS:
        raise ValueError(f'`agent` must be "claude-code" or "codex"; got {agent!r}')
    command = raw.get("command")
    if not isinstance(command, list) or not command:
        raise ValueError("`command` must be a list of at least one string, the adapter's path first")  # fmt: skip
    if any(not isinstance(part, str) or not part for part in command):
        raise ValueError("`command` has an empty or non-string item")
    if not os.path.isabs(command[0]):
        raise ValueError(f"`command` must start with an absolute path; got {command[0]!r}")
    env = raw.get("env", [])
    if not isinstance(env, list) or any(not isinstance(name, str) for name in env):
        raise ValueError("`env` must be a list of variable names")
    for index, name in enumerate(env):
        if not VARIABLE_NAME.fullmatch(name):
            raise ValueError(f"`env` item {name!r} is not a variable name")
        if name in LOCK_DOWN_VARIABLES:
            raise ValueError(f"`env` cannot pass {name}; Orbit sets it for the agent")
        if name in env[:index]:
            raise ValueError(f"`env` names {name} twice")
    mcp_config = raw.get("orbit_mcp_config")
    if mcp_config is None:
        raise ValueError("`orbit_mcp_config` is missing: the translator's TOML path")
    if not isinstance(mcp_config, str) or not os.path.isabs(mcp_config):
        raise ValueError(f"`orbit_mcp_config` must be an absolute path; got {mcp_config!r}")
    return AcpAgentSettings(agent, tuple(command), Path(mcp_config), tuple(env))


def build_backend(
    hub: "Settings", agent: "AgentConfiguration", instructions=no_instructions
) -> "AcpBackend":
    """The agent's backend (`agents.KINDS`), refused at startup when its
    command cannot start. `instructions` reads the personality and enabled
    skills written into the agent's folder before each session."""
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

    # Images: Orbit's turns carry none (see `Capabilities.images`). Resume is
    # `session/load`, refused with its cause by an agent that lacks it. History:
    # the agent keeps each conversation and replays it itself on load, which is
    # over when it answers, so the resume says so and no `HistoryLoaded` follows.
    # Voice: ACP carries no audio.
    capabilities = Capabilities(images=False, resume=True, history=True, voice=False)

    def __init__(self, agent: "AgentConfiguration", hub: "Settings", instructions=no_instructions):
        self.agent_id, self.settings = agent.id, agent.settings
        self.instructions = instructions
        self.folder = hub.data_dir / "agents" / agent.id
        self.config_dir = self.folder / (
            "claude-config" if self.settings.agent == "claude-code" else "codex-home"
        )
        # Every running process, the one used longest ago first.
        self.processes: list[_AgentProcess] = []
        self._admission = asyncio.Lock()

    def connection(self, device: str, device_name: str, on_event: EventSink) -> "AcpConnection":
        return AcpConnection(self, on_event)

    def login_command(self) -> str:
        """What the operator runs on the server to sign the agent in, in its own home."""
        command = shlex.join(self.settings.command)
        home = shlex.quote(str(self.config_dir))
        if self.settings.agent == "claude-code":
            # `--cli` runs the adapter's bundled `claude`. Console, not the
            # subscription, until Anthropic answers about Orbit in writing.
            return f"CLAUDE_CONFIG_DIR={home} {command} --cli auth login --console"
        # `cli` runs codex-acp's bundled `codex`; a device code suits a server.
        return f"CODEX_HOME={home} {command} cli login --device-auth"

    def environment(self) -> dict[str, str]:
        environment = acp.default_environment()
        environment.update(
            {name: os.environ[name] for name in self.settings.env if name in os.environ}
        )
        if self.settings.agent == "claude-code":
            environment["CLAUDE_CONFIG_DIR"] = str(self.config_dir)
        else:
            environment["CODEX_HOME"] = str(self.config_dir)
            environment["CODEX_CONFIG"] = json.dumps(codex_lock_down())
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
        return claude_lock_down() if self.settings.agent == "claude-code" else {}

    def prepare_folder(self) -> None:
        for path in (self.folder.parent, self.folder, self.config_dir):
            if path.is_symlink() or (path.exists() and not path.is_dir()):
                raise HubError(
                    "agent_folder_invalid",
                    f"La carpeta del agente «{self.agent_id}» no es un directorio: {path}",
                )
            if not path.exists():
                path.mkdir(mode=0o700)
                # mkdir's mode passes through the umask; the folder is private.
                path.chmod(0o700)

    async def start(self, connection: "AcpConnection") -> "_AgentProcess":
        """A started, initialized process, retiring an idle one at the ceiling."""
        async with self._admission:
            if len(self.processes) >= MAX_AGENT_PROCESSES:
                idle = [process for process in self.processes if process.idle]
                if not idle:
                    raise HubError(
                        "agent_process_limit",
                        f"El agente «{self.agent_id}» tiene {len(self.processes)} "
                        f"procesos con un turno en curso; techo {MAX_AGENT_PROCESSES}. "
                        "Espera a que termine uno",
                        numbers={"busy": len(self.processes), "limit": MAX_AGENT_PROCESSES},
                    )
                await idle[0].stop()
            self.prepare_folder()
            write_instructions(self.folder, self.settings.agent, self.instructions())
            process = _AgentProcess(self, connection)
            self.processes.append(process)
        try:
            await process.start()
        except BaseException:
            await process.stop()
            raise
        return process

    def used(self, process: "_AgentProcess") -> None:
        if process in self.processes:
            self.processes.remove(process)
            self.processes.append(process)


# What each agent reads from its folder, its `cwd`: the instructions file and
# the skills directory, one `<name>/SKILL.md` per enabled skill.
INSTRUCTIONS_FILE = {"claude-code": "CLAUDE.md", "codex": "AGENTS.md"}
SKILLS_DIRECTORY = {"claude-code": (".claude", "skills"), "codex": (".codex", "skills")}


def write_instructions(folder: Path, agent: AgentName, current: AgentInstructions) -> None:
    """Write the agent's personality and enabled skills into its folder.

    Each file is written whole or not at all (a temporary file, synced, then
    renamed over the old one), and nothing outside `folder` is written: skill
    names are checked again, and a symbolic link where a directory should be
    is refused rather than followed. A skill no longer enabled, or a
    personality now empty, leaves the folder. The folder is the hub's alone.

    The agent's folder is private (`0700`, see `prepare_folder`); inside it,
    these files are `0644` and their directories `0755`, so an agent run as
    a separate system user with access to its folder can read what the hub
    wrote (docs/self-hosting.md).
    """
    root = folder.resolve()
    instructions = folder / INSTRUCTIONS_FILE[agent]
    if current.personality:
        _write_whole(root, instructions, current.personality)
    elif instructions.is_symlink() or instructions.exists():
        instructions.unlink()
    directory = folder
    for part in SKILLS_DIRECTORY[agent]:
        directory = _readable_directory(root, directory / part)
    enabled = set()
    for skill in current.skills:
        if NAME_CHARACTERS.fullmatch(skill.name) is None:
            raise HubError(
                "agent_folder_invalid",
                f"La skill «{skill.name}» saldría de la carpeta del agente: no se escribe",
                numbers={"name": skill.name},
            )
        enabled.add(skill.name)
        _write_whole(
            root, _readable_directory(root, directory / skill.name) / "SKILL.md", skill.content
        )
    for entry in directory.iterdir():
        if entry.name in enabled:
            continue
        if entry.is_dir() and not entry.is_symlink():
            shutil.rmtree(entry)
        else:
            entry.unlink()


def _inside(root: Path, path: Path) -> None:
    if not path.resolve().is_relative_to(root):
        raise HubError(
            "agent_folder_invalid", f"{path} queda fuera de la carpeta del agente: no se escribe"
        )


def _readable_directory(root: Path, path: Path) -> Path:
    if path.is_symlink() or (path.exists() and not path.is_dir()):
        raise HubError("agent_folder_invalid", f"{path} no es un directorio: no se escribe")
    _inside(root, path)
    if not path.exists():
        path.mkdir(mode=0o755)
        path.chmod(0o755)
    return path


def _write_whole(root: Path, path: Path, text: str) -> None:
    _inside(root, path.parent)
    if path.is_symlink():
        raise HubError("agent_folder_invalid", f"{path} es un enlace: no se escribe")
    temporary = path.with_name(f".{path.name}.{uuid4().hex}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            # The umask may have narrowed the mode; the agent's user must read it.
            os.fchmod(handle.fileno(), 0o644)
            handle.write(text.encode("utf-8"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        with suppress(FileNotFoundError):
            temporary.unlink()
        raise


class _LineTransport:
    """ACP's newline-delimited JSON over an agent's stdio, with a ceiling per message.

    The SDK's own transport reads a line of any length; this one stops at
    `MAX_AGENT_MESSAGE_BYTES` (the stream's limit) and ends the connection,
    recording why.
    """

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        self.reader, self.writer = reader, writer
        self.overrun = False
        self._sending = asyncio.Lock()

    async def send(self, message: dict[str, Any]) -> None:
        data = (json.dumps(message, separators=(",", ":")) + "\n").encode()
        async with self._sending:
            self.writer.write(data)
            await self.writer.drain()

    async def receive(self) -> dict[str, Any] | None:
        while True:
            try:
                line = await self.reader.readuntil(b"\n")
            except asyncio.LimitOverrunError:
                self.overrun = True
                return None
            except asyncio.IncompleteReadError as exc:
                line = exc.partial
                if not line.strip():
                    return None
            if not line.strip():
                continue
            try:
                message = json.loads(line)
            except ValueError:
                report("acp_message_unreadable", "upstream_protocol", exc_info=False)
                continue
            if isinstance(message, dict):
                return message

    async def close(self) -> None:
        with suppress(OSError, RuntimeError):
            if not self.writer.is_closing():
                self.writer.close()


class _AgentProcess:
    """One adapter process holding one ACP session; the SDK's `Client` for it."""

    def __init__(self, backend: AcpBackend, connection: "AcpConnection"):
        self.backend, self.connection = backend, connection
        self.name = backend.agent_id
        self.process: asyncio.subprocess.Process | None = None
        self.transport: _LineTransport | None = None
        self.peer: ClientSideConnection | None = None
        self.watcher: asyncio.Task | None = None
        self.can_load = False
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

    def refusal(self, code: str, cause: str) -> HubError:
        command = self.backend.settings.command[0]
        return HubError(code, f"El agente «{self.name}» ({command}) {cause}")

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
        self.transport = _LineTransport(self.process.stdout, self.process.stdin)
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
                raise self.refusal("agent_auth_required", self.sign_in_cause()) from exc
            raise self.refusal(
                "agent_rejected", f"rechazó {method} (código {exc.code}): {_reason(exc)}"
            ) from exc
        except TimeoutError as exc:
            raise self.refusal(
                "agent_timeout", f"no respondió a {method} en {timeout} segundos"
            ) from exc
        except ConnectionError as exc:
            raise self.refusal("agent_stopped", await self.exit_cause(method)) from exc

    def sign_in_cause(self) -> str:
        return (
            "no tiene sesión iniciada en el servidor. Orbit no inicia sesiones ni guarda "
            "credenciales; ejecuta en el servidor, como el usuario del hub: "
            f"{self.backend.login_command()}"
        )

    async def exit_cause(self, method: str) -> str:
        if self.transport is not None and self.transport.overrun:
            return (
                f"envió un mensaje de más de {MAX_AGENT_MESSAGE_BYTES} bytes, el techo de "
                "un mensaje; se detuvo el agente"
            )
        code = None
        if self.process is not None:
            with suppress(TimeoutError):
                code = await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
        if code is None:
            return f"cerró su salida durante {method}; se detuvo el agente"
        return f"terminó con código {code} durante {method}"

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

    def start_turn(self, text: str) -> None:
        if self.turn is not None:
            raise HubError(
                "turn_in_progress", f"El agente «{self.name}» ya tiene un turno en curso"
            )
        self.text = []
        self.tools, self.thought, self.thought_id, self.planned = {}, [], None, False
        self.backend.used(self)
        self.turn = asyncio.create_task(self.run_turn(text))

    async def run_turn(self, text: str) -> None:
        clear_op()
        session_id = self.session_id
        try:
            answer = await self.peer.prompt(
                session_id=session_id, prompt=[TextContentBlock(type="text", text=text)]
            )
        except acp.RequestError as exc:
            if exc.code == AUTH_REQUIRED:
                await self.fail(session_id, "agent_auth_required", self.sign_in_cause())
            else:
                cause = f"rechazó el turno (código {exc.code}): {_reason(exc)}"
                await self.fail(session_id, "agent_rejected", cause)
        except ConnectionError:
            await self.fail(session_id, "agent_stopped", await self.exit_cause("el turno"))
        except Exception:  # noqa: BLE001 - the turn must end, whatever the agent answered
            await self.fail(session_id, "agent_protocol", "respondió al turno algo que el hub no lee")  # fmt: skip
        else:
            status = {"end_turn": "completed", "cancelled": "interrupted"}.get(
                answer.stop_reason, answer.stop_reason
            )
            await self.end_thought(session_id)
            await self.connection.emit(MessageComplete(session_id, "".join(self.text), status))
        finally:
            self.turn = None
            await self.expire_permissions()
            if self.process.returncode is not None:
                await self.stop()

    async def fail(self, session_id: str, code: str, cause: str) -> None:
        report("acp_turn_failed", code, exc_info=False)
        if code in {"agent_stopped", "agent_protocol"}:
            await self.stop()
        await self.connection.emit(
            MessageComplete(session_id, f"El agente «{self.name}» {cause}", "error")
        )

    async def cancel(self) -> None:
        try:
            await self.peer.cancel(session_id=self.session_id)
        except ConnectionError as exc:
            raise self.refusal("agent_stopped", "ya no está en marcha") from exc
        # ACP: the client answers each pending permission of a cancelled turn `cancelled`.
        await self.expire_permissions()

    async def expire_permissions(self) -> None:
        pending, self.permissions = self.permissions, {}
        for request_id, (answer, _options) in pending.items():
            if not answer.done():
                answer.set_result(_cancelled())
                await self.connection.emit(RequestExpired(self.session_id, request_id))

    def answer_permission(self, request_id: str, choice: str) -> PermissionReceipt:
        entry = self.permissions.get(request_id)
        if entry is None or entry[0].done():
            self.permissions.pop(request_id, None)
            return PermissionReceipt(applied=False)
        answer, options = entry
        option = _option(options, CHOICES.get(choice, ()))
        if option is None:
            raise HubError(
                "permission_choice_unavailable",
                f"El agente «{self.name}» no ofrece «{choice[:64]}» en esta petición",
            )
        del self.permissions[request_id]
        answer.set_result(
            RequestPermissionResponse(
                outcome=AllowedOutcome(outcome="selected", option_id=option.option_id)
            )
        )
        return PermissionReceipt(applied=True)

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

    # The SDK's `Client`: what the agent may ask of the hub.

    async def session_update(self, session_id: str, update: Any, **_meta: Any) -> None:
        # Outside a turn (a load's replay included) an update belongs to no turn.
        if self.turn is None or session_id != self.session_id:
            return
        if not isinstance(update, AgentThoughtChunk):
            await self.end_thought(session_id)
        if isinstance(update, AgentMessageChunk):
            if isinstance(update.content, TextContentBlock):
                self.text.append(update.content.text)
                await self.connection.emit(MessageChunk(session_id, update.content.text))
            else:
                ignored = "El agente envió contenido que no es texto; se ignoró"
                await self.connection.emit(UnreadableUpdate(session_id, ignored))
        elif isinstance(update, ToolCallStart):
            name = _tool_name(update.title, update.field_meta)
            arguments = update.raw_input if isinstance(update.raw_input, dict) else None
            self.tools[update.tool_call_id] = (name, arguments)
            phase = TOOL_PHASES.get(update.status or "pending", "started")
            await self.connection.emit(
                ActivityItem(
                    session_id,
                    update.tool_call_id,
                    "tool",
                    "started" if phase == "progress" else phase,
                    name,
                    arguments,
                    cause=_tool_cause(update) if phase == "failed" else None,
                )
            )
        elif isinstance(update, ToolCallProgress):
            name, arguments = self.tools.get(update.tool_call_id, (None, None))
            if name is None:
                name = _tool_name(update.title or "", update.field_meta)
            phase = TOOL_PHASES.get(update.status or "in_progress", "progress")
            await self.connection.emit(
                ActivityItem(
                    session_id,
                    update.tool_call_id,
                    "tool",
                    phase,
                    name,
                    arguments,
                    cause=_tool_cause(update) if phase == "failed" else None,
                )
            )
        elif isinstance(update, AgentPlanUpdate):
            steps = tuple(PlanStep(entry.content, entry.status) for entry in update.entries)
            if steps and all(step.status == "completed" for step in steps):
                phase = "completed"
            else:
                phase = "progress" if self.planned else "started"
            self.planned = True
            await self.connection.emit(ActivityItem(session_id, "plan", "plan", phase, steps=steps))
        elif isinstance(update, AgentThoughtChunk) and isinstance(update.content, TextContentBlock):
            if self.thought_id is None:
                self.thought_id = f"reasoning-{uuid4()}"
                await self.connection.emit(
                    ActivityItem(session_id, self.thought_id, "reasoning", "started")
                )
            self.thought.append(update.content.text)

    async def end_thought(self, session_id: str) -> None:
        """A streamed thought ends when anything else arrives: its whole text
        is the reasoning item's summary, folded in the chat."""
        if self.thought_id is None:
            return
        item_id, text = self.thought_id, "".join(self.thought)
        self.thought, self.thought_id = [], None
        await self.connection.emit(
            ActivityItem(session_id, item_id, "reasoning", "completed", text=text)
        )

    async def request_permission(
        self, session_id: str, tool_call: Any, options: list[PermissionOption], **_meta: Any
    ) -> RequestPermissionResponse:
        if session_id != self.session_id or self.turn is None:
            return _cancelled()
        await self.end_thought(session_id)
        request_id = str(uuid4())
        answer = asyncio.get_running_loop().create_future()
        self.permissions[request_id] = (answer, options)
        choices = tuple(
            choice for choice, kinds in CHOICES.items() if _option(options, kinds) is not None
        )
        description = tool_call.title or f"El agente «{self.name}» pide permiso"
        await self.connection.emit(
            PermissionRequested(session_id, PermissionRequest(request_id, description, choices))
        )
        return await answer

    async def read_text_file(self, **_params: Any) -> None:
        raise _not_offered("fs/read_text_file", "fs")

    async def write_text_file(self, **_params: Any) -> None:
        raise _not_offered("fs/write_text_file", "fs")

    async def create_terminal(self, **_params: Any) -> None:
        raise _not_offered("terminal/create", "terminal")

    async def terminal_output(self, **_params: Any) -> None:
        raise _not_offered("terminal/output", "terminal")

    async def release_terminal(self, **_params: Any) -> None:
        raise _not_offered("terminal/release", "terminal")

    async def wait_for_terminal_exit(self, **_params: Any) -> None:
        raise _not_offered("terminal/wait_for_exit", "terminal")

    async def kill_terminal(self, **_params: Any) -> None:
        raise _not_offered("terminal/kill", "terminal")

    async def create_elicitation(self, **_params: Any) -> None:
        raise _not_offered("elicitation/create", "elicitation")

    async def ext_method(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        raise _not_offered(f"_{method}", "extensiones")

    async def ext_notification(self, method: str, params: dict[str, Any]) -> None:
        return None


class AcpConnection:
    """One device's sessions with the agent, each in a process of its own.

    No connection is shared, so none is lost: a process that ends takes only
    its session, whose id leaves `live_sessions`; the next turn reloads it.
    """

    def __init__(self, backend: AcpBackend, on_event: EventSink):
        self.backend, self.on_event = backend, on_event
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self._sessions: dict[str, _AgentProcess] = {}
        self._open = False

    @property
    def connected(self) -> bool:
        return self._open

    async def open(self) -> None:
        self.epoch = uuid4().hex
        self._open = True

    async def close(self) -> None:
        self._open = False
        mine = [process for process in self.backend.processes if process.connection is self]
        for process in [*self._sessions.values(), *mine]:
            await process.stop()

    async def emit(self, event) -> None:
        await self.on_event(event)

    def forget(self, process: _AgentProcess) -> None:
        if process.session_id is not None and self._sessions.get(process.session_id) is process:
            del self._sessions[process.session_id]
            self.live_sessions.discard(process.session_id)

    async def create_session(self) -> OpenedSession:
        process = await self.backend.start(self)
        try:
            session_id = await process.new_session()
        except BaseException:
            await process.stop()
            raise
        self._hold(process)
        return OpenedSession(session_id, session_id)

    async def resume_session(self, stored_id: str) -> ResumedSession:
        if (held := self._sessions.get(stored_id)) is not None:
            await held.stop()
        process = await self.backend.start(self)
        try:
            await process.load_session(stored_id)
        except BaseException:
            await process.stop()
            raise
        self._hold(process)
        return ResumedSession(stored_id, stored_id, None, False)

    def _hold(self, process: _AgentProcess) -> None:
        self._sessions[process.session_id] = process
        self.live_sessions.add(process.session_id)

    def _live(self, session_id: str) -> _AgentProcess:
        process = self._sessions.get(session_id)
        if process is None:
            raise HubError(
                "session_runtime_not_found",
                f"El agente «{self.backend.agent_id}» ya no tiene esa sesión activa; "
                "el mensaje no se aceptó y puedes volver a enviarlo",
            )
        return process

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        self._live(session_id).start_turn("".join(block.text for block in blocks))

    async def cancel(self, session_id: str) -> None:
        await self._live(session_id).cancel()

    async def respond_permission(
        self, session_id: str, request_id: str, *, choice: str
    ) -> PermissionReceipt:
        return self._live(session_id).answer_permission(request_id, choice)

    async def respond_clarification(
        self, session_id: str, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt:
        name = self.backend.agent_id
        raise HubError(
            "clarification_unsupported",
            f"El agente «{name}» no pide aclaraciones: ACP v1 no las tiene",
        )

    async def turn_stopped(self, session_id: str) -> bool:
        # A session without a process runs nothing; one with a process, only its task.
        process = self._sessions.get(session_id)
        return process is None or process.turn is None


# The Mac's choices (contracts/hub-v1.md) and the ACP option kinds each
# answers with, the first the agent offers. `always` is the agent's own
# remembered permission for that tool (Claude Code keeps it in its session);
# `deny` refuses this once, never for good.
CHOICES: dict[str, tuple[str, ...]] = {
    "once": ("allow_once",),
    "always": ("allow_always",),
    "deny": ("reject_once", "reject_always"),
}
# ACP's tool call status as a `chat.item` phase.
TOOL_PHASES = {
    "pending": "started",
    "in_progress": "progress",
    "completed": "completed",
    "failed": "failed",
}


def _tool_name(title: str, meta: dict | None) -> str:
    """The tool a call runs. claude-agent-acp names it in `_meta.claudeCode.toolName`;
    otherwise the title, which the adapters set to the tool's name for MCP tools."""
    claude = meta.get("claudeCode") if isinstance(meta, dict) else None
    named = claude.get("toolName") if isinstance(claude, dict) else None
    return named if isinstance(named, str) and named else title


def _tool_cause(update: Any) -> str | None:
    """A failed call's text content, else its raw output, as the agent said it."""
    texts = [
        item.content.text
        for item in update.content or []
        if getattr(item, "type", None) == "content"
        and isinstance(getattr(item, "content", None), TextContentBlock)
    ]
    if texts:
        return " ".join(texts)
    output = update.raw_output
    if isinstance(output, str):
        return output
    return json.dumps(output, ensure_ascii=False) if output is not None else None


def _option(options: list[PermissionOption], kinds: tuple[str, ...]) -> PermissionOption | None:
    for kind in kinds:
        for option in options:
            if option.kind == kind:
                return option
    return None


def _cancelled() -> RequestPermissionResponse:
    return RequestPermissionResponse(outcome=DeniedOutcome(outcome="cancelled"))


def _not_offered(method: str, capability: str) -> acp.RequestError:
    report("acp_client_method_refused", method, exc_info=False)
    return acp.RequestError(
        NOT_OFFERED,
        f"Orbit no ofrece {method}: el hub no declara la capacidad «{capability}»; "
        "usa las herramientas de Orbit",
        {"method": method},
    )


def _reason(exc: acp.RequestError) -> str:
    text = str(exc)
    if len(text) <= MAX_AGENT_REASON_CHARS:
        return text
    return text[: MAX_AGENT_REASON_CHARS - 1] + "…"


def _signal_group(pid: int, signal_number: int) -> None:
    with suppress(ProcessLookupError, PermissionError):
        os.killpg(pid, signal_number)
