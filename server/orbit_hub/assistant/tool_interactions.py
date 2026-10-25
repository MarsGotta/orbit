"""Pending vendor questions expressed as Orbit approvals and clarifications."""

import asyncio
from dataclasses import dataclass, field
from uuid import uuid4

from ..errors import HubError
from .activity import one_line
from .backend import (
    ClarificationReceipt,
    ClarificationRequest,
    ClarificationRequested,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    RequestExpired,
)

# Human callbacks may wait indefinitely for a Mac. Sixteen concurrent
# interactions bound retained futures and snapshots, not turns or questions
# answered over the lifetime of a conversation.
MAX_PENDING_INTERACTIONS = 16


@dataclass
class Question:
    future: asyncio.Future
    ids: tuple[str, ...]
    answers: dict[str, str] = field(default_factory=dict)


class ToolInteractions:
    def __init__(self, session_id: str, emit):
        self.session_id, self.emit = session_id, emit
        self.permissions: dict[str, asyncio.Future] = {}
        self.questions: dict[str, Question] = {}

    async def permission(self, description: str, deletes=None) -> str:
        self.admit()
        request_id = str(uuid4())
        future = asyncio.get_running_loop().create_future()
        self.permissions[request_id] = future
        try:
            request = PermissionRequest(
                request_id, one_line(description, 500), ("once", "deny"), deletes
            )
            await self.emit(PermissionRequested(self.session_id, request))
            return await future
        finally:
            self.permissions.pop(request_id, None)

    def answer_permission(self, request_id: str, choice: str) -> PermissionReceipt:
        future = self.permissions.get(request_id)
        if future is None or future.done():
            return PermissionReceipt(False)
        if choice not in {"once", "deny"}:
            raise HubError(
                "permission_choice_unavailable", "Esta petición ofrece una vez o denegar"
            )
        future.set_result(choice)
        return PermissionReceipt(True)

    async def clarify(self, questions: list[dict]) -> dict[str, str]:
        self.admit()
        if not questions:
            raise HubError("agent_protocol", "El agente pidió una aclaración sin preguntas")
        # Orbit's interaction frame is 128 KiB. Eight short questions leave
        # room for choices and the envelope; this bounds a batch, not a turn.
        if len(questions) > 8:
            raise HubError(
                "agent_protocol",
                f"El agente pidió {len(questions)} aclaraciones; máximo 8",
                numbers={"count": len(questions), "max": 8},
            )
        public = []
        for index, question in enumerate(questions):
            public.append(
                {
                    "qid": str(question.get("id", index)),
                    "question": one_line(question.get("question"), 500),
                    "multi_select": bool(question.get("multiSelect", False)),
                    "choices": [
                        one_line(o.get("label"), 100) for o in (question.get("options") or [])[:8]
                    ],
                }
            )
        request_id = str(uuid4())
        entry = Question(
            asyncio.get_running_loop().create_future(), tuple(q["qid"] for q in public)
        )
        if len(set(entry.ids)) != len(entry.ids) or any(
            not qid or len(qid) > 160 for qid in entry.ids
        ):
            raise HubError(
                "agent_protocol",
                "El agente envió ids de pregunta vacíos, repetidos o mayores de 160 caracteres",
            )
        self.questions[request_id] = entry
        try:
            await self.emit(
                ClarificationRequested(
                    self.session_id,
                    ClarificationRequest(request_id, public[0]["question"], tuple(public), None),
                )
            )
            return await entry.future
        finally:
            self.questions.pop(request_id, None)

    def admit(self) -> None:
        count = len(self.permissions) + len(self.questions)
        if count >= MAX_PENDING_INTERACTIONS:
            raise HubError(
                "agent_protocol",
                f"El agente tiene {count} preguntas pendientes; máximo {MAX_PENDING_INTERACTIONS}",
                numbers={"count": count, "max": MAX_PENDING_INTERACTIONS},
            )

    def answer_question(self, request_id: str, answer: str, question_id: str | None):
        entry = self.questions.get(request_id)
        if entry is None or entry.future.done():
            return ClarificationReceipt("expired", ())
        qid = question_id or (entry.ids[0] if len(entry.ids) == 1 else None)
        if qid not in entry.ids or qid in entry.answers:
            raise HubError("clarification_question_invalid", "Esa pregunta no está pendiente")
        entry.answers[qid] = answer
        remaining = tuple(q for q in entry.ids if q not in entry.answers)
        if not remaining:
            entry.future.set_result(dict(entry.answers))
        return ClarificationReceipt("ok", remaining)

    async def expire(self) -> None:
        for request_id, future in list(self.permissions.items()):
            if not future.done():
                future.set_result("deny")
                await self.emit(RequestExpired(self.session_id, request_id))
        for request_id, question in list(self.questions.items()):
            if not question.future.done():
                question.future.set_result({})
                await self.emit(RequestExpired(self.session_id, request_id))
