"""What an agent's connection says, turned into the frames the Mac reads.

One handler per kind of event. An event names its agent's live session; the
runtime's bindings map it to the Orbit conversation and its turn. Upstream
events are observed facts: a terminal one closes the turn whatever sub-state
the hub had recorded.
"""

import asyncio
from dataclasses import replace

from ..assistant.activity import public_item
from ..assistant.backend import (
    ActivityItem,
    AssistantConnection,
    AssistantEvent,
    ClarificationRequested,
    ConnectionLost,
    HistoryLoaded,
    MessageChunk,
    MessageComplete,
    PermissionRequest,
    PermissionRequested,
    RequestExpired,
    StatusNotice,
    TurnError,
    TurnFailed,
    UnreadableUpdate,
)
from ..assistant.deletion import one_at_a_time
from ..errors import render
from ..interactions import pending_data
from ..log import report
from ..protocol import HubError, emit
from ..turns import TurnState

_IN_FLIGHT = {TurnState.SUBMITTING, TurnState.STREAMING, TurnState.INTERRUPTING}


class TurnEvents:
    def __init__(self, runtime):
        self.runtime = runtime

    async def on_event(self, agent_id: str, event: AssistantEvent) -> None:
        """One event from this device's connection to the agent `agent_id`."""
        runtime = self.runtime
        if isinstance(event, ConnectionLost):
            await self.connection_lost(agent_id)
            return
        if not runtime.store.devices.is_active(runtime.device):
            return
        key = (agent_id, event.session_id)
        if self._resume_progress(key, event):
            return
        session_id = runtime.bindings.get(key)
        if session_id is None:
            return
        if isinstance(event, UnreadableUpdate):
            # Never a turn event: an unreadable update must not end or retire anything.
            await self.status(session_id, runtime.turn_ids.get(session_id), "warning", event.text)
            return
        turn_id = runtime.turn_ids.get(session_id)
        if turn_id is None:
            return
        if isinstance(event, TurnFailed):
            await self.fail_turn(agent_id, session_id, turn_id, event)
        elif isinstance(event, TurnError):
            await self._turn_error(agent_id, key, session_id, turn_id, event)
        elif isinstance(event, MessageChunk):
            data = {"session_id": session_id, "text": event.text}
            await runtime.publish(emit("chat.delta", data, turn_id))
        elif isinstance(event, MessageComplete):
            await self.complete(agent_id, key, session_id, turn_id, event)
        elif isinstance(event, PermissionRequested | ClarificationRequested):
            await self.requested(session_id, turn_id, event)
        elif isinstance(event, RequestExpired):
            await self.expired(session_id, turn_id, event)
        elif isinstance(event, StatusNotice):
            await self.status(session_id, turn_id, event.kind, event.text)
        elif isinstance(event, ActivityItem):
            # Live only: neither stored nor in the snapshot. A Mac that
            # reconnects mid-turn sees the steps that follow. Their count
            # and what they used stay with the answer (`runtime.work`).
            item = public_item(event)
            if (work := runtime.work.get(session_id)) is not None:
                work.record(item)
            await runtime.publish(emit("chat.item", {"session_id": session_id, **item}, turn_id))

    def _resume_progress(self, key, event) -> bool:
        """A reopen's history event, which no turn event is; True when handled."""
        runtime = self.runtime
        if isinstance(event, HistoryLoaded):
            runtime.resumes.record_progress(key, "complete" if event.loaded else "failed")
            return True
        if not (isinstance(event, TurnError) and event.cause == "history_unreadable"):
            return False
        if runtime.resumes.record_progress(key, "failed"):
            return True
        connection = runtime.connections.get(key[0])
        return (
            key in runtime.bindings
            and connection is not None
            and key[1] not in connection.live_sessions
        )

    async def connection_lost(self, agent_id: str) -> None:
        runtime = self.runtime
        runtime.resumes.connection_lost(agent_id)
        # Only this agent's sessions lost their connection.
        lost = {session for (agent, _), session in runtime.bindings.items() if agent == agent_id}
        for session_id in lost:
            if runtime.turn_state(session_id) not in _IN_FLIGHT:
                continue
            runtime.store.chat.set_status(session_id, "outcome_unknown")
            data = {
                "session_id": session_id,
                "code": "outcome_unknown",
                "message": f"Se perdió la conexión con {runtime.agent_name(agent_id)}; "
                "consulta el historial antes de continuar",
            }
            await runtime.publish(emit("error", data, runtime.turn_ids.get(session_id)))
        runtime.forget_requests(lost)
        runtime.resumed_message_counts = {
            key: count
            for key, count in runtime.resumed_message_counts.items()
            if key[0] != agent_id
        }
        runtime.interrupt_deadlines = {
            session: deadline
            for session, deadline in runtime.interrupt_deadlines.items()
            if session not in lost
        }
        await runtime.changed()

    async def status(self, session_id: str, turn_id: str | None, kind: str, text: str) -> None:
        data = {"session_id": session_id, "kind": kind, "text": text}
        await self.runtime.publish(emit("chat.status", data, turn_id))

    async def fail_turn(
        self, agent_id: str, session_id: str, turn_id: str, failure: TurnFailed | None = None
    ) -> None:
        """End the turn as failed: for the cause the backend named in
        `failure`, or, without one, the backend's terminal error."""
        runtime = self.runtime
        if runtime.turn_ids.get(session_id) != turn_id:
            return
        # A terminal upstream event is an observed fact, not a decision: it
        # closes the turn whatever sub-state the hub had recorded.
        runtime.store.chat.set_status(session_id, "failed")
        runtime.interrupt_deadlines.pop(session_id, None)
        runtime.forget_requests({session_id})
        if failure is None:
            data = {
                "session_id": session_id,
                "code": "upstream_turn_failed",
                "message": f"{runtime.agent_name(agent_id, starts=True)} terminó el turno "
                "sin una respuesta final; no se repetirá",
            }
        else:
            data = render(
                failure.code,
                failure.message,
                payload={"session_id": session_id},
                numbers=failure.numbers,
            )
        await runtime.publish(emit("error", data, turn_id))
        await runtime.changed()

    async def _turn_error(self, agent_id, key, session_id, turn_id, event: TurnError) -> None:
        runtime = self.runtime
        if event.cause == "terminal":
            await self.fail_turn(agent_id, session_id, turn_id)
        elif (session_id, turn_id) not in runtime.reconciliations:
            connection = runtime.connections.get(agent_id)
            runtime.reconciliations[(session_id, turn_id)] = asyncio.create_task(
                self.reconcile_error(connection, agent_id, key[1], session_id, turn_id)
            )

    async def reconcile_error(
        self,
        connection: AssistantConnection,
        agent_id: str,
        runtime_id: str,
        session_id: str,
        turn_id: str,
    ) -> None:
        runtime = self.runtime
        try:
            stopped = await connection.turn_stopped(runtime_id)
            if runtime.turn_ids.get(session_id) != turn_id:
                return
            if runtime.store.chat.session(runtime.device, session_id)["status"] != "streaming":
                return
            if stopped:
                await self.fail_turn(agent_id, session_id, turn_id)
            else:
                await self.status(
                    session_id,
                    turn_id,
                    "warning",
                    f"{runtime.agent_name(agent_id, starts=True)} notificó un error; el turno "
                    "no está confirmado como finalizado",
                )
        except HubError:
            if runtime.turn_ids.get(session_id) == turn_id:
                await self.status(
                    session_id,
                    turn_id,
                    "warning",
                    f"No se pudo comprobar el turno con {runtime.agent_name(agent_id)}; puedes "
                    "interrumpir o abrir una sesión nueva",
                )
        except Exception:  # noqa: BLE001 - a background task's failure is reported, never lost
            report(
                "reconcile_error_failed",
                "internal_error",
                device=runtime.device,
                session=session_id,
            )
        finally:
            runtime.reconciliations.pop((session_id, turn_id), None)

    async def complete(self, agent_id, key, session_id, turn_id, event: MessageComplete) -> None:
        runtime = self.runtime
        text, status = event.text, event.status
        is_resumed = key in runtime.resumed_message_counts
        resumed_count = runtime.resumed_message_counts.pop(key, None)
        resume_failed = event.history_unreadable and is_resumed
        # Failed output remains a live partial, never an authoritative history message.
        persisted = True
        warning_code: str | None = None
        work = runtime.work.pop(session_id, None)
        if status != "error" and text:
            now_ms = round(runtime.now() * 1000)
            try:
                runtime.store.chat.append_message(
                    session_id,
                    "assistant",
                    text,
                    now_ms=now_ms,
                    work=work.summary(now_ms) if work is not None else None,
                )
            except HubError as exc:
                # Persistence failure: the turn still ends and the session
                # leaves streaming. Other sessions and the upstream
                # connection remain untouched.
                persisted = False
                warning_code = exc.code
        # A terminal upstream event is an observed fact, not a decision:
        # it closes the turn whatever sub-state the hub had recorded.
        runtime.store.chat.set_status(session_id, "failed" if status == "error" else "completed")
        runtime.interrupt_deadlines.pop(session_id, None)
        runtime.forget_requests({session_id})
        completed: dict = {
            "session_id": session_id,
            "text": "" if resume_failed else text,
            "status": status,
            "persisted": persisted,
        }
        if warning_code is not None:
            completed["warning"] = warning_code
        await runtime.publish(emit("chat.completed", completed, turn_id))
        if resume_failed:
            await self._resume_unreadable(agent_id, key, session_id, turn_id, resumed_count)
        await runtime.changed()

    async def _resume_unreadable(self, agent_id, key, session_id, turn_id, count) -> None:
        runtime = self.runtime
        if (connection := runtime.connections.get(agent_id)) is not None:
            connection.live_sessions.discard(key[1])
        count_text = (
            f"({count} mensajes en el último tramo)"
            if count is not None
            else "durante la reanudación"
        )
        data = {
            "session_id": session_id,
            "code": "session_resume_history_unreadable",
            "message": f"{runtime.agent_name(agent_id, starts=True)} no pudo leer el historial "
            f"guardado {count_text}; este turno falló y no se repetirá. Puedes enviar un mensaje "
            "nuevo para volver a reanudar",
        }
        await runtime.publish(emit("error", data, turn_id))

    async def requested(self, session_id, turn_id, event) -> None:
        runtime = self.runtime
        kind, name = (
            ("approval", "approval.required")
            if isinstance(event, PermissionRequested)
            else ("clarify", "clarify.required")
        )
        request = event.request
        if isinstance(request, PermissionRequest) and request.deletes is not None:
            # One command that deletes at a time: no `session` or
            # `always`, here where the answer is checked.
            request = replace(request, choices=one_at_a_time(request.choices))
        request_id = runtime.new_request_key(session_id, request.request_id)
        runtime.pending[request_id] = (session_id, kind, request)
        data = {"session_id": session_id, **pending_data(kind, request, request_id)}
        await runtime.publish(emit(name, data, turn_id))
        await runtime.changed()

    async def expired(self, session_id, turn_id, event: RequestExpired) -> None:
        runtime = self.runtime
        request_id = runtime.request_key(session_id, event.request_id)
        if request_id is None:
            # Not pending here: told as the backend named it, as before.
            request_id = event.request_id
        else:
            runtime.pending.pop(request_id, None)
        data = {"session_id": session_id, "request_id": request_id}
        await runtime.publish(emit("interaction.expired", data, turn_id))
        await runtime.changed()
