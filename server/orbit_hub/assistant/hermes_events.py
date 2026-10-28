"""What Hermes's gateway events mean to the hub: each one as the typed event
of `backend` it stands for; the rest is dropped."""

from __future__ import annotations

import json
from uuid import uuid4

from ..log import report
from .backend import (
    ActivityItem,
    ClarificationRequest,
    ClarificationRequested,
    EventSink,
    HistoryLoaded,
    MessageChunk,
    MessageComplete,
    PermissionRequest,
    PermissionRequested,
    PlanStep,
    RequestExpired,
    StatusNotice,
    TurnError,
    UnreadableUpdate,
)
from .deletion import command_deletion

# How Hermes begins the error of a resume whose stored transcript it could not
# read; the rest of its message is Hermes's detail and is never relayed.
RESUME_HISTORY_ERROR_PREFIX = "Could not reopen that session (its transcript could not be read)."
# Hermes errors that end a turn before the agent ran, without a message.complete.
TERMINAL_ERRORS = {'Context injection refused.', 'Turn cancelled before the agent was ready', 'Session no longer running before the agent was ready'}  # fmt: skip

# Hermes's subagent outcomes that mean the delegated task did not finish.
SUBAGENT_FAILED = {"failed", "error", "timeout", "interrupted", "cancelled"}
PLAN_STATUSES = {"pending", "in_progress", "completed", "cancelled"}


def _plan(todos: object) -> tuple[PlanStep, ...] | None:
    """Hermes's task list (`todo.updated`, the `todo` tool's result) as plan steps."""
    if not isinstance(todos, list):
        return None
    steps = []
    for todo in todos:
        if isinstance(todo, dict) and isinstance(content := todo.get("content"), str):
            status = todo.get("status")
            steps.append(PlanStep(content, status if status in PLAN_STATUSES else "pending"))
    return tuple(steps)


def _todos_of(result: object) -> object:
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except ValueError:
            return None
    return result.get("todos") if isinstance(result, dict) else None


def _choices(payload: dict) -> tuple[str, ...] | None:
    choices = payload.get("choices")
    if not isinstance(choices, list):
        return None
    return tuple(choice for choice in choices if isinstance(choice, str))


def _error_cause(message: object) -> str:
    """Why Hermes ended a turn with an error, as far as its message says."""
    if isinstance(message, str) and message.startswith(RESUME_HISTORY_ERROR_PREFIX):
        return "history_unreadable"
    if isinstance(message, str) and message in TERMINAL_ERRORS:
        return "terminal"
    return "unconfirmed"


class HermesEvents:
    """One device connection's reading of Hermes's events."""

    def __init__(self, on_event: EventSink, device: str):
        self.on_event, self.device = on_event, device
        # Sessions whose turn has published a plan: its next update is progress.
        self._planned: set[str] = set()

    async def receive(self, params: dict) -> None:
        """One Hermes event, as the typed event it means; the rest is dropped."""
        kind = params.get("type")
        session_id = params.get("session_id")
        if not isinstance(session_id, str):
            return
        payload = params.get("payload")
        if payload is not None and type(payload) is not dict:
            # A non-object upstream payload is a transport-level warning,
            # never a turn event. It must not propagate: a protocol mistake
            # cannot be allowed to falsely retire every live session.
            await self.on_event(
                UnreadableUpdate(
                    session_id, "Hermes sent a payload that is not a JSON object; ignored"
                )
            )
            return
        payload = payload or {}
        if kind == "session.resume_progress":
            if payload.get("phase") == "history" and payload.get("status") in {
                "complete",
                "failed",
            }:
                await self.on_event(HistoryLoaded(session_id, payload["status"] == "complete"))
        elif kind == "error":
            await self.on_event(TurnError(session_id, _error_cause(payload.get("message"))))
        elif kind == "message.delta":
            if isinstance(text := payload.get("text"), str):
                await self.on_event(MessageChunk(session_id, text))
        elif kind == "message.complete":
            await self._message_complete(session_id, payload)
        elif kind == "approval.request":
            await self._approval_request(session_id, payload)
        elif kind == "clarify.request":
            await self._clarify_request(session_id, kind, payload)
        elif kind in {"clarify.expire", "approval.expire"}:
            await self._request_expired(session_id, kind, payload)
        elif kind in {"tool.start", "tool.complete"}:
            await self._tool(session_id, kind, payload)
        elif kind == "todo.updated":
            await self._plan_update(session_id, _plan(payload.get("todos")))
        elif kind == "reasoning.available":
            await self._reasoning(session_id, payload)
        elif isinstance(kind, str) and kind.startswith("subagent."):
            await self._subagent(session_id, kind, payload)
        elif kind == "status.update":
            await self.on_event(
                StatusNotice(
                    session_id,
                    str(payload.get("kind") or "status"),
                    str(payload.get("text") or ""),
                )
            )

    async def _message_complete(self, session_id: str, payload: dict) -> None:
        self._planned.discard(session_id)
        text = payload.get("text")
        status = str(payload.get("status") or "completed")
        history_error = payload.get("error") or payload.get("text")
        await self.on_event(
            MessageComplete(
                session_id,
                text if isinstance(text, str) else "",
                status,
                history_unreadable=(
                    status == "error"
                    and isinstance(surface := payload.get("error_surface"), dict)
                    and surface.get("code") == "agent_init_failed"
                    and isinstance(history_error, str)
                    and history_error.startswith(RESUME_HISTORY_ERROR_PREFIX)
                ),
            )
        )

    async def _approval_request(self, session_id: str, payload: dict) -> None:
        if not isinstance(request_id := payload.get("request_id"), str):
            return
        description = str(
            payload.get("description") or payload.get("command") or "Hermes requests authorization"
        )
        command = payload.get("command")
        await self.on_event(
            PermissionRequested(
                session_id,
                PermissionRequest(
                    request_id,
                    description,
                    _choices(payload),
                    command_deletion(command) if isinstance(command, str) else None,
                ),
            )
        )

    async def _clarify_request(self, session_id: str, kind: str, payload: dict) -> None:
        if not isinstance(request_id := payload.get("request_id"), str):
            return
        question = str(payload.get("question") or "Answer the Hermes questions")
        await self.on_event(
            ClarificationRequested(
                session_id,
                ClarificationRequest(
                    request_id, question, self._questions(kind, payload), _choices(payload)
                ),
            )
        )

    async def _request_expired(self, session_id: str, kind: str, payload: dict) -> None:
        request_id = payload.get("request_id")
        if not isinstance(request_id, str):
            report(
                event="hermes_field_invalid",
                code=f"{kind}.request_id:{type(request_id).__name__}",
                device=self.device,
                exc_info=False,
            )
            return
        await self.on_event(RequestExpired(session_id, request_id))

    async def _reasoning(self, session_id: str, payload: dict) -> None:
        # The reasoning's summary, when the profile shows reasoning. Its
        # deltas (`reasoning.delta`) are the thought itself and
        # `thinking.delta` a spinner's text: neither is shown.
        if isinstance(text := payload.get("text"), str) and text.strip():
            await self.on_event(
                ActivityItem(
                    session_id, f"reasoning-{uuid4()}", "reasoning", "completed", text=text
                )
            )

    async def _tool(self, session_id: str, kind: str, payload: dict) -> None:
        call_id = payload.get("tool_id") or payload.get("tool_call_id")
        name = payload.get("name") or payload.get("tool_name")
        if not isinstance(call_id, str) or not call_id or not isinstance(name, str):
            report(
                event="hermes_field_invalid",
                code=f"{kind}.tool_id:{type(call_id).__name__}",
                device=self.device,
                exc_info=False,
            )
            return
        arguments = payload.get("args") if isinstance(payload.get("args"), dict) else None
        if name == "todo":
            # Hermes's task list is the turn's plan, not a step of its own
            # (and «Usó todo» would read as «used everything»).
            if kind == "tool.complete" and payload.get("is_error") is not True:
                # A turn that is not a local viewer's gets no `todo.updated`:
                # the todo tool's own result carries the list.
                await self._plan_update(session_id, _plan(_todos_of(payload.get("result"))))
            return
        if kind == "tool.start":
            phase, cause = "started", None
        elif payload.get("is_error") is True:
            phase, cause = "failed", payload.get("result")
            cause = cause if isinstance(cause, str) else None
        else:
            phase, cause = "completed", None
        await self.on_event(
            ActivityItem(session_id, call_id, "tool", phase, name, arguments, cause=cause)
        )

    async def _plan_update(self, session_id: str, steps: tuple[PlanStep, ...] | None) -> None:
        if not steps:
            return
        if all(step.status in {"completed", "cancelled"} for step in steps):
            phase = "completed"
        else:
            phase = "progress" if session_id in self._planned else "started"
        self._planned.add(session_id)
        await self.on_event(ActivityItem(session_id, "plan", "plan", phase, steps=steps))

    async def _subagent(self, session_id: str, kind: str, payload: dict) -> None:
        phases = {
            "subagent.spawn_requested": "started",
            "subagent.start": "started",
            "subagent.progress": "progress",
            "subagent.tool": "progress",
            "subagent.complete": "completed",
        }
        if (phase := phases.get(kind)) is None:
            return
        identity = payload.get("subagent_id")
        if not isinstance(identity, str) or not identity:
            index = payload.get("task_index")
            identity = f"subagent-{index if type(index) is int else 0}"
        cause = None
        status = payload.get("status")
        if phase == "completed" and status in SUBAGENT_FAILED:
            phase = "failed"
            summary = payload.get("summary")
            cause = summary if isinstance(summary, str) and summary.strip() else status
        goal = payload.get("goal")
        await self.on_event(
            ActivityItem(
                session_id,
                identity,
                "subagent",
                phase,
                text=goal if isinstance(goal, str) else "",
                cause=cause,
            )
        )

    def _questions(self, event: str, payload: dict) -> tuple[dict, ...] | None:
        questions = payload.get("questions")
        if questions is None:
            return None
        if not isinstance(questions, list):
            report(
                event="hermes_field_invalid",
                code=f"{event}.questions:{type(questions).__name__}",
                device=self.device,
                exc_info=False,
            )
            return ()
        valid = []
        for index, question in enumerate(questions):
            if not isinstance(question, dict):
                field, value = "item", question
            elif not isinstance(question.get("qid"), str):
                field, value = "qid", question.get("qid")
            elif not isinstance(question.get("question"), str):
                field, value = "question", question.get("question")
            else:
                valid.append(question)
                continue
            report(
                event="hermes_field_invalid",
                code=f"{event}.questions[{index}].{field}:{type(value).__name__}",
                device=self.device,
                exc_info=False,
            )
        return tuple(valid)
