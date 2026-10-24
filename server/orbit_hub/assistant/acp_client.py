"""The SDK's `Client` for one ACP agent process: what the agent may ask of the hub.

The agent streams its turn as `session/update` notifications, asks the Mac's
permission with `session/request_permission`, and says which account it runs
with in `_auth/status_update`. The files, terminals and elicitations it might
ask for are not offered: the hub declares neither `fs` nor `terminal`.
"""

import asyncio
import json
import shlex
from typing import TYPE_CHECKING, Any
from uuid import uuid4

import acp
from acp.schema import (
    AgentMessageChunk,
    AgentPlanUpdate,
    AgentThoughtChunk,
    AllowedOutcome,
    DeniedOutcome,
    PermissionOption,
    RequestPermissionResponse,
    TextContentBlock,
    ToolCallProgress,
    ToolCallStart,
    UsageUpdate,
)

from ..errors import HubError
from ..log import report
from .backend import (
    ActivityItem,
    MessageChunk,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    PlanStep,
    RequestExpired,
    UnreadableUpdate,
)
from .deletion import Deletion, command_deletion

if TYPE_CHECKING:
    from .acp import AcpBackend
    from .acp_connection import AcpConnection

# JSON-RPC's "method not found", for what the hub does not offer an agent.
NOT_OFFERED = -32601

# What the adapter says of its account, kept and published to every Mac in
# `ready` and `agents.changed`. What they protect: each Mac's status topic,
# which carries every agent (orbit-core `TopicId::Status`). Why these sizes:
# an address of 128 bytes and a plan's name of 64 hold any real one
# («ChatGPT Plus», «Claude Pro»); a longer one is dropped, not cut.
MAX_ACCOUNT_EMAIL_BYTES = 128
MAX_PLAN_BYTES = 64


def _short_text(value: Any, max_bytes: int) -> str | None:
    """A label the adapter gives, kept if it is printable text within `max_bytes`."""
    if (
        isinstance(value, str)
        and value
        and value.isprintable()
        and len(value.encode("utf-8")) <= max_bytes
    ):
        return value
    return None


class AcpClient:
    """What the agent may ask of the hub, mixed into `AgentProcess`, whose
    turn state these methods read and write."""

    backend: "AcpBackend"
    connection: "AcpConnection"
    session_id: str | None
    turn: asyncio.Task | None
    text: list[str]
    tools: dict[str, tuple[str, dict | None]]
    thought: list[str]
    thought_id: str | None
    planned: bool
    permissions: dict[str, tuple[asyncio.Future, list[PermissionOption]]]
    limit_resets_ms: int | None

    @property
    def name(self) -> str:
        return self.backend.agent_name

    async def session_update(self, session_id: str, update: Any, **_meta: Any) -> None:
        # Outside a turn (a load's replay included) an update belongs to no turn.
        if self.turn is None or session_id != self.session_id:
            return
        if isinstance(update, UsageUpdate):
            self.rate_limit(update.field_meta)
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
            PermissionRequested(
                session_id,
                PermissionRequest(request_id, description, choices, _tool_deletion(tool_call)),
            )
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
        # Both adapters say which account they run with after starting, at
        # each session and at each turn: `{authStatus: {kind, label,
        # account: {email, plan}}}`. Nothing else of it is kept.
        if method != "auth/status_update" or not isinstance(params, dict):
            return
        status = params.get("authStatus")
        if not isinstance(status, dict) or not isinstance(status.get("kind"), str):
            return
        account = status.get("account") if isinstance(status.get("account"), dict) else {}
        self.backend.observe(
            "status",
            kind=status["kind"],
            email=_short_text(account.get("email"), MAX_ACCOUNT_EMAIL_BYTES),
            plan=_short_text(account.get("plan"), MAX_PLAN_BYTES)
            or _short_text(status.get("label"), MAX_PLAN_BYTES),
        )
        return

    def rate_limit(self, meta: Any) -> None:
        """Claude Code's report of its plan's limit (`_claude/rateLimit`, the
        Agent SDK's `SDKRateLimitInfo`): the share used of a window, as a
        fraction, and when it resets, in seconds. Sent when it changes, and
        only for a claude.ai subscription. Codex sends nothing of the kind."""
        info = meta.get("_claude/rateLimit") if isinstance(meta, dict) else None
        if not isinstance(info, dict):
            return
        resets = info.get("resetsAt")
        resets_ms = round(resets * 1000) if isinstance(resets, int | float) else None
        if info.get("status") == "rejected":
            self.limit_resets_ms = resets_ms
        used = info.get("utilization")
        if isinstance(used, int | float) and not isinstance(used, bool) and 0 <= used <= 1:
            window = info.get("rateLimitType")
            self.backend.observe(
                "usage",
                used_percent=round(used * 100),
                window=window if isinstance(window, str) and len(window) <= 64 else None,
                resets_at_ms=resets_ms,
            )

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


def _tool_deletion(tool_call: Any) -> Deletion | None:
    """What a tool call deletes: ACP's own `delete` kind (with a path per
    location), or a command line in its input (`command`, a string or the
    words of one, as claude-agent-acp and codex-acp send it)."""
    if getattr(tool_call, "kind", None) == "delete":
        locations = getattr(tool_call, "locations", None) or []
        return Deletion(paths=len(locations) or None)
    raw = getattr(tool_call, "raw_input", None)
    command = raw.get("command") if isinstance(raw, dict) else None
    if isinstance(command, list) and all(isinstance(word, str) for word in command):
        command = shlex.join(command)
    return command_deletion(command) if isinstance(command, str) else None


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
