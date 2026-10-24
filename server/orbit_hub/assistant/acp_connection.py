"""One device's sessions with an ACP agent, each in a process of its own."""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING
from uuid import uuid4

from ..errors import HubError
from .backend import (
    ClarificationReceipt,
    ContentBlock,
    EventSink,
    OpenedSession,
    PermissionReceipt,
    ResumedSession,
)

if TYPE_CHECKING:
    from .acp import AcpBackend, AgentProcess


class AcpConnection:
    """One device's sessions with the agent, each in a process of its own.

    No connection is shared, so none is lost: a process that ends takes only
    its session, whose id leaves `live_sessions`; the next turn reloads it.
    """

    def __init__(self, backend: AcpBackend, on_event: EventSink):
        self.backend, self.on_event = backend, on_event
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self._sessions: dict[str, AgentProcess] = {}
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

    def forget(self, process: AgentProcess) -> None:
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

    def _hold(self, process: AgentProcess) -> None:
        self._sessions[process.session_id] = process
        self.live_sessions.add(process.session_id)

    def _live(self, session_id: str) -> AgentProcess:
        process = self._sessions.get(session_id)
        if process is None:
            raise HubError(
                "session_runtime_not_found",
                f"El agente «{self.backend.agent_name}» ya no tiene esa sesión activa; "
                "el mensaje no se aceptó y puedes volver a enviarlo",
            )
        return process

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        process = self._live(session_id)
        process.start_turn(process.turn_prompt(blocks))

    async def cancel(self, session_id: str) -> None:
        await self._live(session_id).cancel()

    async def respond_permission(
        self, session_id: str, request_id: str, *, choice: str
    ) -> PermissionReceipt:
        return self._live(session_id).answer_permission(request_id, choice)

    async def respond_clarification(
        self, session_id: str, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt:
        raise HubError(
            "clarification_unsupported",
            f"El agente «{self.backend.agent_name}» no pide aclaraciones: ACP v1 no las tiene",
        )

    async def turn_stopped(self, session_id: str) -> bool:
        # A session without a process runs nothing; one with a process, only its task.
        process = self._sessions.get(session_id)
        return process is None or process.turn is None
