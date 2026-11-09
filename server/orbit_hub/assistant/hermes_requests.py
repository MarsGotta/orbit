"""Hermes's questions to the person, asked as requests from server to client.

Hermes asks over JSON-RPC requests (`approval`, `clarify`, `sudo`…, ids
`srq-…`), and only to a client that declared on its connection that it
answers them (`client.capabilities`, sent by `hermes`). An approval and a
clarification become the cards the Mac shows; their answers go back as
`request.answer`, or one question at a time as `clarify.lock`, whose receipts
say whether Hermes still waited. Everything else is refused at once with its
cause, so no agent waits out its deadline for a card nobody sees.

Every request gets one answer from the hub, except those Hermes withdraws
itself (`request.cancel`) and those left open when the connection drops:
nothing is answered or repeated on another connection. The `open_requests` a
`session.resume` returns are not shown again: after a drop the turn that asked
is `outcome_unknown`, and its question under the next turn would say something
false; Hermes withdraws it at its own deadline.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass

from ..log import report
from ..protocol import HubError
from .backend import (
    ClarificationReceipt,
    ClarificationRequest,
    ClarificationRequested,
    EventSink,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    RequestExpired,
    StatusNotice,
    UnreadableReceipt,
)
from .deletion import command_deletion
from .hermes_refusals import METHOD_NOT_FOUND

# The requests only a Hermes window that shows the session answers
# (tui_gateway/server_requests.py). Any other client declines them with
# NOT_SHOWN_CODE, which leaves them to that window; when every client
# declined, Hermes withdraws the request at once. Sent whether or not Hermes
# says `declines_not_shown`: a Hermes that does not know the code takes it as
# any error and withdraws the request just the same.
WINDOW_REQUESTS = frozenset({"preview.act", "preview.read", "terminal.read", "tour", "window.read"})
NOT_SHOWN_CODE = 4404
# JSON-RPC's codes: no handler for the method (METHOD_NOT_FOUND, what Hermes
# documents for a client without one), params the hub cannot read, and a
# refusal of the hub's own, whose message names its cause.
INVALID_PARAMS = -32602
REFUSED = -32000
# Open requests on one connection. It protects the hub's memory and the Mac's
# cards from a Hermes that asks without end. Why 64: a real Hermes holds one
# open question per running agent thread (a turn and its subagents), and a
# device's conversations with one agent rarely run more than a few at once.
# It does not limit the questions of a turn over time, nor other connections.
MAX_OPEN_SERVER_REQUESTS = 64
# The longest withdrawal reason of Hermes (`request.cancel`) the warning
# carries. It protects the warning the Mac receives from a reason Hermes does
# not bound. Why 64: Hermes's reasons are single snake_case words (`timeout`,
# `session_closed`, an approval's `notify_failed`) of under 20 characters, so
# 64 holds each whole. It does not limit anything else of the request.
MAX_REASON_CHARS = 64
# How many unanswered question ids a clarification receipt may name, and how
# long each may be, before the hub reads the receipt as unreadable. They keep
# a receipt that cannot be Hermes's out of the snapshot and the Mac's answer:
# Hermes asks 1 to 5 questions at once (tools/clarify_tool.py, MAX_QUESTIONS)
# with ids `q0` to `q4`, far inside both. They do not limit what Hermes asks.
MAX_REMAINING_QUESTIONS = 32
MAX_QUESTION_ID_CHARS = 128
# The question of a clarification of several, as the Mac has shown it so far.
SEVERAL_QUESTIONS = "Answer the Hermes questions"


@dataclass(frozen=True)
class OpenRequest:
    session_id: str
    method: str
    # A clarification's question ids, asked as `questions` and answered by
    # `clarify.lock`. None: an approval, or the single `question` of an older
    # Hermes, answered whole by `request.answer`.
    qids: tuple[str, ...] | None


def _strings(value: object) -> tuple[str, ...] | None:
    if not isinstance(value, list):
        return None
    return tuple(item for item in value if isinstance(item, str))


def _approval(session_id: str, request_id: str, params: dict) -> PermissionRequested:
    command = params.get("command")
    description = str(
        params.get("description") or params.get("command") or "Hermes requests authorization"
    )
    # No choices named: `once` and `deny`.
    choices = _strings(params.get("choices")) or None
    deletes = command_deletion(command) if isinstance(command, str) else None
    request = PermissionRequest(request_id, description, choices, deletes)
    return PermissionRequested(session_id, request)


def _readable(question: object) -> bool:
    return (
        isinstance(question, dict)
        and isinstance(question.get("qid"), str)
        and bool(question["qid"])
        and isinstance(question.get("question"), str)
    )


def _clarification(
    session_id: str, request_id: str, params: dict
) -> tuple[ClarificationRequested, tuple[str, ...] | None] | None:
    """The card and the question ids; None when a question cannot be read
    (one left unread could never be answered, and Hermes would wait for it)."""
    questions = params.get("questions")
    if questions is None:
        # An older Hermes's single question: `{question, choices?, multi_select?}`.
        question = params.get("question")
        if not isinstance(question, str) or not question.strip():
            return None
        request = ClarificationRequest(request_id, question, None, _strings(params.get("choices")))
        return ClarificationRequested(session_id, request), None
    if not isinstance(questions, list) or not questions or not all(map(_readable, questions)):
        return None
    if len(questions) == 1:
        [only] = questions
        choices = _strings(only.get("choices"))
        request = ClarificationRequest(request_id, only["question"], None, choices)
    else:
        request = ClarificationRequest(request_id, SEVERAL_QUESTIONS, tuple(questions), None)
    return ClarificationRequested(session_id, request), tuple(item["qid"] for item in questions)


def remaining_question_ids(remaining: object) -> tuple[str, ...] | None:
    """The question ids a clarification receipt says are still unanswered
    (`clarify.lock`, and `clarify.respond` of a Hermes older than its
    requests); None when it cannot be read as such."""
    if (
        not isinstance(remaining, list)
        or len(remaining) > MAX_REMAINING_QUESTIONS
        or not all(
            isinstance(item, str) and 1 <= len(item) <= MAX_QUESTION_ID_CHARS for item in remaining
        )
    ):
        return None
    return tuple(remaining)


def _status(result: dict, what: str) -> str:
    status = result.get("status")
    if status not in {"ok", "expired"}:
        raise UnreadableReceipt(
            "upstream_protocol", f"Hermes did not confirm the {what}; it will not be repeated"
        )
    return status


class HermesRequests:
    """The requests Hermes holds open on one connection, and their answers."""

    def __init__(
        self,
        on_event: EventSink,
        write: Callable[[dict], Awaitable[None]],
        rpc: Callable[[str, dict], Awaitable[dict]],
        device: str,
    ):
        self.on_event, self._write, self._rpc = on_event, write, rpc
        self.device = device
        self.open: dict[str, OpenRequest] = {}

    def holds(self, request_id: str) -> bool:
        return request_id in self.open

    def forget(self) -> None:
        """The connection dropped: nothing open on it is answered any more."""
        self.open.clear()

    async def receive(self, frame: dict, live_sessions: set[str]) -> None:
        """One request of Hermes: shown to the person, or refused at once."""
        request_id, method = frame.get("id"), frame["method"]
        if not isinstance(request_id, str):
            # Hermes names its requests `srq-…`. Without a text id there is
            # nothing the Mac can name in its answer, and a notification asks
            # nothing: dropped, as the events' unreadable ids are.
            report(
                event="hermes_field_invalid",
                code=f"request.id:{type(request_id).__name__}",
                device=self.device,
                exc_info=False,
            )
            return
        if method in WINDOW_REQUESTS:
            await self._refuse(
                request_id,
                NOT_SHOWN_CODE,
                f"Orbit shows no Hermes window; {method} is for a window that shows the session",
            )
            return
        params = frame.get("params")
        session_id = params.get("session_id") if isinstance(params, dict) else None
        if not isinstance(session_id, str) or session_id not in live_sessions:
            await self._refuse(
                request_id, REFUSED, f"Orbit holds no such session on this connection ({method})"
            )
            return
        if method not in {"approval", "clarify"}:
            await self._refuse(request_id, METHOD_NOT_FOUND, f"Orbit has no handler for {method}")
            await self._warn(session_id, f"Hermes asked for {method}, which Orbit cannot answer")
            return
        if len(self.open) >= MAX_OPEN_SERVER_REQUESTS:
            ceiling = f"{len(self.open)} open; ceiling {MAX_OPEN_SERVER_REQUESTS}"
            await self._refuse(
                request_id, REFUSED, f"Orbit holds too many Hermes requests ({ceiling})"
            )
            await self._warn(session_id, f"Hermes asked one question too many ({ceiling})")
            return
        await self._show(request_id, method, session_id, params)

    async def _show(self, request_id, method: str, session_id: str, params: dict) -> None:
        if method == "approval":
            event, qids = _approval(session_id, request_id, params), None
        elif (asked := _clarification(session_id, request_id, params)) is not None:
            event, qids = asked
        else:
            await self._refuse(
                request_id, INVALID_PARAMS, "The clarification has a question Orbit cannot read"
            )
            return
        # Open before it is shown: the Mac may answer while it is published.
        self.open[request_id] = OpenRequest(session_id, method, qids)
        if await self.on_event(event) is not True:
            # A revoked device, or a session no conversation shows.
            self.open.pop(request_id, None)
            await self._refuse(
                request_id,
                REFUSED,
                f"This device did not show the question ({method}): a revoked device, "
                "or no conversation of it shows that session",
            )

    async def cancel(self, payload: object) -> None:
        """Hermes withdrew a request: its card leaves with Hermes's reason."""
        request_id = payload.get("id") if isinstance(payload, dict) else None
        opened = self.open.pop(request_id, None) if isinstance(request_id, str) else None
        if opened is None:
            # Already answered, or not this connection's.
            return
        reason = payload.get("reason")
        reason = reason[:MAX_REASON_CHARS] if isinstance(reason, str) and reason else "no reason"
        await self.on_event(RequestExpired(opened.session_id, request_id))
        await self._warn(
            opened.session_id,
            f"Hermes withdrew its {opened.method} question ({reason}); it can no longer be answered",
        )

    async def answer_permission(self, request_id: str, choice: str) -> PermissionReceipt:
        result = await self._rpc("request.answer", {"id": request_id, "result": {"choice": choice}})
        # Answered: Hermes no longer waits for it, whatever the receipt says.
        self.open.pop(request_id, None)
        return PermissionReceipt(applied=_status(result, "approval") == "ok")

    async def answer_clarification(
        self, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt:
        qids = self.open[request_id].qids
        if qids is None:
            result = await self._rpc(
                "request.answer", {"id": request_id, "result": {"answer": answer}}
            )
            self.open.pop(request_id, None)
            return ClarificationReceipt(_status(result, "clarification"), ())
        if not question_id:
            if len(qids) != 1:
                raise HubError(
                    "clarify_question_required",
                    f"Hermes asked {len(qids)} questions at once; answer each naming its question_id",
                    numbers={"questions": len(qids)},
                )
            [question_id] = qids
        params = {"request_id": request_id, "question_id": question_id, "answer": answer}
        result = await self._rpc("clarify.lock", params)
        remaining = remaining_question_ids(result.get("remaining"))
        if result.get("status") == "ok" and remaining is not None:
            if not remaining:
                # The last question closed the request on Hermes's side.
                self.open.pop(request_id, None)
            return ClarificationReceipt("ok", remaining)
        self.open.pop(request_id, None)
        if result.get("status") == "expired":
            return ClarificationReceipt("expired", ())
        raise UnreadableReceipt(
            "upstream_protocol", "Hermes did not confirm the clarification; it will not be repeated"
        )

    async def _refuse(self, request_id, code: int, message: str) -> None:
        await self._write(
            {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}
        )

    async def _warn(self, session_id: str, text: str) -> None:
        await self.on_event(StatusNotice(session_id, "warning", text))
