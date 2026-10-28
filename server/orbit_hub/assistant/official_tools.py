"""Official vendor runners behind the hub's existing assistant contract."""

import asyncio
import os
import sys
from collections.abc import Sequence
from pathlib import Path
from uuid import uuid4

from ..errors import HubError
from ..log import clear_op, report
from .agent_config import no_instructions
from .backend import (
    Capabilities,
    ContentBlock,
    MessageComplete,
    OpenedSession,
    ResumedSession,
    TurnFailed,
)
from .sign_in import private_folder
from .tool_availability import ToolUnavailable
from .tool_interactions import ToolInteractions
from .tool_settings import HOME_FOLDERS, environment
from .workspaces import WorkspacePolicy

# One personal hub needs only a few concurrent conversations. Four vendor
# processes bound host RAM/CPU, not stored sessions; idle runners are evicted.
MAX_AGENT_PROCESSES = 4
# One vendor JSON frame may include several 700 KB Orbit screenshots. 8 MiB
# bounds each allocation without limiting the length of a turn or transcript.
MAX_AGENT_MESSAGE_BYTES = 8 * 1024 * 1024
# Generous unmeasured budgets for process/MCP startup and transcript replay;
# neither applies to a model turn, which can stream indefinitely.
START_TIMEOUT_SECONDS = 30
OPEN_TIMEOUT_SECONDS = 120


def build_backend(hub, agent, instructions=no_instructions):
    path = Path(agent.settings.command[0])
    cause, cause_code = None, "adapter_unavailable"
    if not path.exists():
        cause, cause_code = "does not exist", "adapter_path_missing"
    elif not path.is_file():
        cause, cause_code = "is not a file", "adapter_not_file"
    elif not os.access(path, os.X_OK):
        cause, cause_code = "does not have execute permission", "adapter_permission"
    if cause:
        raise ToolUnavailable(path, cause, agent.id, agent.kind, cause_code)
    return OfficialBackend(hub, agent, instructions)


class OfficialBackend:
    capabilities = Capabilities(images=True, resume=True, history=True, voice=False)

    def __init__(self, hub, agent, instructions):
        self.hub, self.agent = hub, agent
        self.agent_id, self.agent_name = agent.id, agent.name
        self.settings, self.instructions = agent.settings, instructions
        self.workspaces = WorkspacePolicy(hub, agent.id)
        self.folder = hub.data_dir / "agents" / agent.id
        self.config_dir = self.folder / HOME_FOLDERS[agent.kind]
        self.processes = []
        self._admission = asyncio.Lock()
        self.observe = lambda fact, **values: None

    def rename(self, name: str) -> None:
        self.agent_name = name

    def connection(self, device, device_name, on_event):
        return OfficialConnection(self, on_event)

    def environment(self):
        return {**environment(self.settings, self.config_dir), "TMPDIR": "/tmp"}

    def mcp_server(self):
        return {
            "command": sys.executable,
            "args": ["-m", "orbit_mcp.cli"],
            "env": {
                "ORBIT_MCP_CONFIG": str(self.folder / "bridge" / "mcp.toml"),
                "ORBIT_MCP_AGENT": self.agent_id,
            },
        }

    async def start(self, connection, stored_id=None):
        from .claude_sdk import ClaudeProcess
        from .codex_server import CodexProcess

        async with self._admission:
            if len(self.processes) >= MAX_AGENT_PROCESSES:
                idle = next((p for p in self.processes if p.held and p.turn is None), None)
                if idle is None:
                    raise HubError(
                        "agent_process_limit",
                        f"{self.agent_name} has {len(self.processes)} busy processes; maximum {MAX_AGENT_PROCESSES}",
                        numbers={"busy": len(self.processes), "limit": MAX_AGENT_PROCESSES},
                    )
                await idle.stop()
            for path in (self.folder.parent, self.folder, self.config_dir):
                private_folder(path)
            from .jail import launch
            from .local_mcp import prepare_agent

            await asyncio.to_thread(
                prepare_agent, self.settings.orbit_mcp_config, self.folder, self.agent_id
            )
            prefix = await launch(
                self.hub, self.agent, self.workspaces.current(), self.workspaces.configs
            )
            runner = ClaudeProcess if self.settings.agent == "claude-code" else CodexProcess
            process = runner(self, connection, stored_id)
            process.jail_prefix = prefix
            self.processes.append(process)
        try:
            await process.open()
            if process.stopped:
                raise HubError(
                    "agent_stopped", "The connection closed while the agent was starting"
                )
        except BaseException:
            await process.stop()
            raise
        return process


class OfficialConnection:
    def __init__(self, backend, emit):
        self.backend, self.emit = backend, emit
        self.epoch = ""
        self.connected = False
        self.live_sessions: set[str] = set()
        self.sessions = {}

    async def open(self):
        self.epoch, self.connected = uuid4().hex, True

    async def close(self):
        self.connected = False
        for process in list(self.backend.processes):
            if process.connection is self:
                await process.stop()

    async def create_session(self):
        process = await self.backend.start(self)
        self._hold(process)
        return OpenedSession(process.session_id, process.session_id)

    async def resume_session(self, stored_id):
        if held := self.sessions.get(stored_id):
            await held.stop()
        process = await self.backend.start(self, stored_id)
        self._hold(process)
        return ResumedSession(process.session_id, process.session_id, None, False)

    def _hold(self, process):
        process.held = True
        self.sessions[process.session_id] = process
        self.live_sessions.add(process.session_id)

    def forget(self, process):
        if self.sessions.get(process.session_id) is process:
            self.sessions.pop(process.session_id)
            self.live_sessions.discard(process.session_id)

    def live(self, session_id):
        if (process := self.sessions.get(session_id)) is None:
            raise HubError("session_runtime_not_found", "The agent session is no longer active")
        return process

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]):
        self.live(session_id).start_turn(blocks)

    async def cancel(self, session_id):
        await self.live(session_id).cancel()

    async def respond_permission(self, session_id, request_id, *, choice):
        return self.live(session_id).interactions.answer_permission(request_id, choice)

    async def respond_clarification(self, session_id, request_id, answer, question_id):
        return self.live(session_id).interactions.answer_question(request_id, answer, question_id)

    async def turn_stopped(self, session_id):
        process = self.sessions.get(session_id)
        return process is None or process.turn is None


class VendorProcess:
    def __init__(self, backend, connection, stored_id):
        self.backend, self.connection = backend, connection
        self.workspace = backend.workspaces.current()
        self.session_id = stored_id or str(uuid4())
        self.resuming = stored_id is not None
        self.interactions = ToolInteractions(self.session_id, connection.emit)
        self.turn = None
        self.text = []
        self.limit_resets_ms = None
        self.stopped = False
        # An initializer owns a slot and cannot be evicted as an idle
        # conversation before its caller has finished opening the session.
        self.held = False

    def refusal(self, code, cause, numbers=None):
        return HubError(code, f"Agent «{self.backend.agent_name}» {cause}", numbers=numbers)

    def start_turn(self, blocks):
        if self.turn is not None:
            raise self.refusal("turn_in_progress", "already has a turn in progress")
        self.text, self.limit_resets_ms = [], None
        self.turn = asyncio.create_task(self.run_turn(blocks))

    async def run_turn(self, blocks):
        clear_op()
        try:
            status = await self.execute(blocks)
        except HubError as exc:
            report("agent_turn_failed", exc.code, numbers=exc.numbers, exc_info=False)
            terminal = TurnFailed(self.session_id, exc.code, exc.message, exc.numbers)
        except Exception:  # noqa: BLE001 - vendor input must end the request without leaking diagnostics
            report("agent_turn_failed", "agent_protocol", exc_info=False)
            error = self.refusal("agent_protocol", "returned something the hub cannot read")
            terminal = TurnFailed(self.session_id, error.code, error.message)
        else:
            if status == "completed":
                self.backend.observe("answered")
            terminal = MessageComplete(self.session_id, "".join(self.text), status)
        finally:
            await self.interactions.expire()
            self.turn = None
        await self.connection.emit(terminal)

    def limit_failure(self):
        self.backend.observe("limit", until_ms=self.limit_resets_ms)
        numbers = (
            {"resets_at_ms": self.limit_resets_ms} if self.limit_resets_ms is not None else None
        )
        return self.refusal("agent_limit_reached", "reached your plan limit", numbers)

    def auth_failure(self):
        self.backend.observe("auth_required")
        return self.refusal(
            "agent_auth_required", "needs you to sign in through its settings in Orbit"
        )

    async def stop(self):
        if self.stopped:
            return
        self.stopped = True
        self.connection.forget(self)
        if self in self.backend.processes:
            self.backend.processes.remove(self)
        await self.interactions.expire()
        if self.turn is not None and self.turn is not asyncio.current_task():
            self.turn.cancel()
            await asyncio.gather(self.turn, return_exceptions=True)
