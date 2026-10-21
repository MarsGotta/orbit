"""Ownership and conversation orchestration; upstream events become public frames."""

import asyncio
import json
import math
from collections.abc import Callable
from concurrent.futures import Executor

from .interactions import TERMINAL_ERRORS, decode_response, pending_data
from .log import current_op, in_store, logger, report
from .meeting_context import MeetingTurn, meeting_context, read_meeting
from .protocol import HubError, emit
from .safety import target_key
from .storage import Store
from .turns import INTERRUPT_DEADLINE_SECONDS, TurnState, advance, can_start_turn

TRANSCRIBED_TURN_TTL_SECONDS = 120
# Bound a missing hydration event while the session runtime ID is reserved.
# Hermes waits up to 300 s for resume_history_ready (server.py:_await_resume_history),
# so this matches its own bound, not a model-turn or transcript-size ceiling.
RESUME_HISTORY_TIMEOUT_SECONDS = 300.0
RESUME_HISTORY_ERROR_PREFIX = "Could not reopen that session (its transcript could not be read)."


class TurnInputTracker:
    """Short-lived evidence that a device's next turn came from hub STT."""

    def __init__(self, now: Callable[[], float]):
        self.now = now
        self.transcriptions: dict[str, tuple[float, str]] = {}

    def mark_transcribed(self, device: str, text: str) -> None:
        current = self.now()
        self.transcriptions = {
            owner: evidence
            for owner, evidence in self.transcriptions.items()
            if evidence[0] >= current
        }
        self.transcriptions[device] = (current + TRANSCRIBED_TURN_TTL_SECONDS, text)

    def consume(self, device: str, text: str) -> str:
        evidence = self.transcriptions.pop(device, None)
        if evidence is not None and evidence[0] >= self.now() and evidence[1] == text:
            return "nota de voz transcrita"
        return "texto"


class DeviceRuntime:
    def __init__(
        self,
        device: str,
        store: Store,
        adapter,
        now: Callable[[], float],
        *,
        store_executor: Executor | None = None,
        input_tracker: TurnInputTracker | None = None,
    ):
        self.device, self.store, self.adapter, self.now = device, store, adapter, now
        self.device_name = self._device_name()
        self.input_tracker = input_tracker or TurnInputTracker(now)
        self.connection = None
        self.bindings: dict[str, str] = {}
        self.resumed_message_counts: dict[str, int | None] = {}
        self.resuming_ack = False
        self.early_resume_progress: tuple[str, str] | None = None
        self.resume_waiter: tuple[str, asyncio.Future[str]] | None = None
        self.resume_pending: set[str] = set()
        self.resume_deadline: float | None = None
        self.turn_ids: dict[str, str] = {}
        self.interrupt_deadlines: dict[str, float] = {}
        self.pending: dict[str, tuple[str, str, dict]] = {}
        self.listeners: set[asyncio.Queue] = set()
        self.lock = asyncio.Lock()
        self.revoked = False
        self.closed = False
        self.revision = 0
        self.reconciliations = {}
        self.store_executor = store_executor
        self._snapshot_cache = None

    def _device_name(self) -> str:
        for row in self.store.active_devices():
            if row["id"] == self.device:
                return row["name"].replace("\r", " ").replace("\n", " ")
        raise HubError("revoked", "Dispositivo revocado")

    def subscribe(self) -> asyncio.Queue:
        queue: asyncio.Queue = asyncio.Queue(maxsize=256)
        self.listeners.add(queue)
        if self.bindings:
            queue.put_nowait(emit("state.snapshot", self.snapshot()))
        return queue

    def _normalize_questions(self, event: str, payload: dict) -> dict:
        questions = payload.get("questions")
        if questions is None:
            return payload
        if not isinstance(questions, list):
            report(
                event="hermes_field_invalid",
                code=f"{event}.questions:{type(questions).__name__}",
                device=self.device,
                exc_info=False,
            )
            return {**payload, "questions": []}
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
        return {**payload, "questions": valid}

    async def _run_store(self, loop: asyncio.AbstractEventLoop, function: Callable[[], object]):
        """Dispatch a Store call onto the shared single-thread executor when available."""
        if self.store_executor is None:
            return function()
        return await in_store(self.store_executor, function)

    def snapshot(self) -> dict:
        if self._snapshot_cache is not None:
            return self._snapshot_cache
        sessions, omitted = [], len(self.turn_ids) > 64
        for session_id, turn_id in list(self.turn_ids.items())[-64:]:
            binding = self.store.session(self.device, session_id)
            pending = []
            for request_id, (owner, kind, payload) in self.pending.items():
                if owner != session_id:
                    continue
                if len(pending) == 128:
                    omitted = True
                    continue
                value = pending_data(request_id, kind, payload)
                pending.append(value)
                if len(json.dumps([sessions, pending]).encode()) > 100 * 1024:
                    pending.pop()
                    omitted = True
            sessions.append(
                {
                    "session_id": session_id,
                    "turn_id": turn_id,
                    "status": binding["status"],
                    "pending": pending,
                }
            )
        self._snapshot_cache = {
            "revision": self.revision,
            "sessions": sessions,
            "omitted": omitted,
        }
        return self._snapshot_cache

    async def changed(self) -> None:
        self.revision, self._snapshot_cache = self.revision + 1, None
        await self.publish(emit("state.snapshot", self.snapshot()))

    def turn_state(self, session_id: str) -> TurnState:
        """The persisted session status is exactly one machine state."""
        return TurnState(self.store.session(self.device, session_id)["status"])

    def apply_state(self, session_id: str, target: TurnState) -> None:
        """Apply one table-validated transition.

        A stale event proposing a transition the table forbids is skipped, not
        forced: the persisted state is more authoritative than the observation
        that arrived too late.
        """
        current = self.turn_state(session_id)
        if current is target:
            return
        try:
            self.store.status(session_id, advance(current, target).value)
        except HubError:
            return
        if current is TurnState.INTERRUPTING:
            self.interrupt_deadlines.pop(session_id, None)

    def expire_stale_interrupt(self, session_id: str) -> None:
        """An interrupting turn that outlived its deadline has an unknown outcome."""
        deadline = self.interrupt_deadlines.get(session_id)
        if deadline is not None and self.now() >= deadline:
            self.interrupt_deadlines.pop(session_id, None)
            self.apply_state(session_id, TurnState.OUTCOME_UNKNOWN)

    def safety_target(self, method, params) -> tuple[str, str]:
        self.require_live(params.session_id)
        turn_id = self.turn_ids.get(params.session_id)
        if not turn_id or params.turn_id != turn_id:
            raise HubError("stale_turn", "El control no pertenece al turno actual")
        if method == "chat.interrupt" and self.turn_state(params.session_id) in {
            TurnState.IDLE,
            TurnState.COMPLETED,
            TurnState.FAILED,
        }:
            raise HubError("stale_turn", "El turno ya no está activo")
        if method == "approval.respond":
            pending = self.pending.get(params.request_id)
            if params.approved or pending is None or pending[:2] != (params.session_id, "approval"):
                raise HubError("interaction_not_found", "Denegación ajena, desconocida o caducada")
        return target_key(
            method, params.session_id, turn_id, getattr(params, "request_id", "")
        ), turn_id

    async def fail_turn(self, session_id: str, turn_id: str) -> None:
        if self.turn_ids.get(session_id) != turn_id:
            return
        # A terminal upstream event is an observed fact, not a decision: it
        # closes the turn whatever sub-state the hub had recorded.
        self.store.status(session_id, "failed")
        self.interrupt_deadlines.pop(session_id, None)
        self.pending = {key: value for key, value in self.pending.items() if value[0] != session_id}
        await self.publish(
            emit(
                "error",
                {
                    "session_id": session_id,
                    "code": "upstream_turn_failed",
                    "message": "Hermes terminó el turno sin una respuesta final; no se repetirá",
                },
                turn_id,
            )
        )
        await self.changed()

    async def reconcile_error(self, runtime_id: str, session_id: str, turn_id: str) -> None:
        try:
            result = await self.connection.rpc("session.status", {"session_id": runtime_id})
            if self.turn_ids.get(session_id) != turn_id:
                return
            status = self.store.session(self.device, session_id)["status"]
            if status != "streaming":
                return
            output = result.get("output")
            lines = output.splitlines() if isinstance(output, str) else []
            if lines.count("Agent Running: No") == 1 and "Agent Running: Yes" not in lines:
                await self.fail_turn(session_id, turn_id)
            else:
                await self.publish(
                    emit(
                        "chat.status",
                        {
                            "session_id": session_id,
                            "kind": "warning",
                            "text": "Hermes notificó un error; el turno no está confirmado como finalizado",
                        },
                        turn_id,
                    )
                )
        except HubError:
            if self.turn_ids.get(session_id) == turn_id:
                await self.publish(
                    emit(
                        "chat.status",
                        {
                            "session_id": session_id,
                            "kind": "warning",
                            "text": "No se pudo reconciliar Hermes; puedes interrumpir o abrir una sesión nueva",
                        },
                        turn_id,
                    )
                )
        except Exception:  # noqa: BLE001 - a background task's failure is reported, never lost
            report(
                "reconcile_error_failed", "internal_error", device=self.device, session=session_id
            )
        finally:
            self.reconciliations.pop((session_id, turn_id), None)

    async def publish(self, frame: dict) -> None:
        self.offer(frame)

    def offer(self, frame: dict) -> None:
        """Queue one frame for every connection of this device, on the loop."""
        for queue in list(self.listeners):
            try:
                queue.put_nowait(frame)
            except asyncio.QueueFull:
                self.listeners.discard(queue)
                while not queue.empty():
                    queue.get_nowait()
                queue.put_nowait(
                    emit(
                        "error",
                        {
                            "code": "slow_client",
                            "message": "Cliente atrasado; recupera el historial",
                        },
                    )
                )
                queue.put_nowait(None)

    async def connect(self):
        if self.connection is None or not self.connection.connected:
            connection = self.adapter.connection(self.on_upstream)
            bind_device = getattr(connection, "bind_device", None)
            if bind_device is not None:
                bind_device(self.device_name)
            self.connection = connection
            await self.connection.open()
        return self.connection

    async def on_upstream(self, params: dict) -> None:
        kind = params.get("type")
        if kind == "orbit.upstream_lost":
            if self.resume_waiter is not None and not self.resume_waiter[1].done():
                self.resume_waiter[1].set_result("lost")
            self.early_resume_progress = None
            for session_id in self.bindings.values():
                if self.turn_state(session_id) not in {
                    TurnState.SUBMITTING,
                    TurnState.STREAMING,
                    TurnState.INTERRUPTING,
                }:
                    continue
                self.store.status(session_id, "outcome_unknown")
                await self.publish(
                    emit(
                        "error",
                        {
                            "session_id": session_id,
                            "code": "outcome_unknown",
                            "message": "Se perdió Hermes; consulta el historial antes de continuar",
                        },
                        self.turn_ids.get(session_id),
                    )
                )
            self.pending.clear()
            self.resumed_message_counts.clear()
            self.interrupt_deadlines.clear()
            await self.changed()
            return
        if not self.store.is_active(self.device):
            return
        runtime_id = params.get("session_id")
        payload = params.get("payload")
        if kind == "session.resume_progress" and isinstance(payload, dict):
            if payload.get("phase") == "history" and payload.get("status") in {
                "complete",
                "failed",
            }:
                self.record_resume_progress(runtime_id, payload["status"])
            return
        if (
            kind == "error"
            and isinstance(payload, dict)
            and isinstance(payload.get("message"), str)
            and payload["message"].startswith(RESUME_HISTORY_ERROR_PREFIX)
            and (
                self.record_resume_progress(runtime_id, "failed")
                or (
                    isinstance(runtime_id, str)
                    and runtime_id in self.bindings
                    and self.connection is not None
                    and runtime_id not in self.connection.live_sessions
                )
            )
        ):
            return
        session_id = self.bindings.get(params.get("session_id"))
        if session_id is None:
            return
        raw_payload = params.get("payload")
        if raw_payload is not None and type(raw_payload) is not dict:
            # A non-object upstream payload is a transport-level warning,
            # never a turn event. It must not propagate: a protocol mistake
            # cannot be allowed to falsely retire every live session via
            # orbit.upstream_lost.
            await self.publish(
                emit(
                    "chat.status",
                    {
                        "session_id": session_id,
                        "kind": "warning",
                        "text": "Hermes envió un payload que no es un objeto JSON; se ignoró",
                    },
                    self.turn_ids.get(session_id),
                )
            )
            return
        payload = raw_payload if isinstance(raw_payload, dict) else {}
        data = {"session_id": session_id}
        turn_id = self.turn_ids.get(session_id)
        if turn_id is None:
            return
        if kind == "error":
            if payload.get("message") in TERMINAL_ERRORS:
                await self.fail_turn(session_id, turn_id)
            elif (session_id, turn_id) not in self.reconciliations:
                self.reconciliations[(session_id, turn_id)] = asyncio.create_task(
                    self.reconcile_error(params["session_id"], session_id, turn_id)
                )
            return
        if kind == "message.delta":
            if isinstance(text := payload.get("text"), str):
                await self.publish(emit("chat.delta", {**data, "text": text}, turn_id))
        elif kind == "message.complete":
            text = payload.get("text")
            status = str(payload.get("status") or "completed")
            is_resumed = params["session_id"] in self.resumed_message_counts
            resumed_count = self.resumed_message_counts.pop(params["session_id"], None)
            history_error = payload.get("error") or payload.get("text")
            resume_failed = (
                status == "error"
                and is_resumed
                and isinstance(surface := payload.get("error_surface"), dict)
                and surface.get("code") == "agent_init_failed"
                and isinstance(history_error, str)
                and history_error.startswith(RESUME_HISTORY_ERROR_PREFIX)
            )
            # Failed output remains a live partial, never an authoritative history message.
            persisted = True
            warning_code: str | None = None
            if status != "error" and isinstance(text, str) and text:
                try:
                    self.store.append_message(
                        session_id, "assistant", text, now_ms=round(self.now() * 1000)
                    )
                except HubError as exc:
                    # Persistence failure: the turn still ends and the session
                    # leaves streaming. Other sessions and the upstream
                    # connection remain untouched.
                    persisted = False
                    warning_code = exc.code
            # A terminal upstream event is an observed fact, not a decision:
            # it closes the turn whatever sub-state the hub had recorded.
            self.store.status(session_id, "failed" if status == "error" else "completed")
            self.interrupt_deadlines.pop(session_id, None)
            self.pending = {
                key: value for key, value in self.pending.items() if value[0] != session_id
            }
            completed_data: dict = {
                **data,
                "text": text if isinstance(text, str) else "",
                "status": status,
                "persisted": persisted,
            }
            if warning_code is not None:
                completed_data["warning"] = warning_code
            if resume_failed:
                completed_data["text"] = ""
            await self.publish(emit("chat.completed", completed_data, turn_id))
            if resume_failed:
                self.connection.live_sessions.discard(params["session_id"])
                count_text = (
                    f"({resumed_count} mensajes en el último tramo)"
                    if resumed_count is not None
                    else "durante la reanudación"
                )
                await self.publish(
                    emit(
                        "error",
                        {
                            "session_id": session_id,
                            "code": "session_resume_history_unreadable",
                            "message": f"Hermes no pudo leer el historial guardado {count_text}; "
                            "este turno falló y no se repetirá. "
                            "Puedes enviar un mensaje nuevo para volver a reanudar",
                        },
                        turn_id,
                    )
                )
            await self.changed()
        elif kind == "approval.request":
            if isinstance(request_id := payload.get("request_id"), str):
                self.pending[request_id] = (session_id, "approval", payload)
                await self.publish(
                    emit(
                        "approval.required",
                        {**data, **pending_data(request_id, "approval", payload)},
                        turn_id,
                    )
                )
                await self.changed()
        elif kind == "clarify.request":
            if isinstance(request_id := payload.get("request_id"), str):
                payload = self._normalize_questions(kind, payload)
                self.pending[request_id] = (session_id, "clarify", payload)
                await self.publish(
                    emit(
                        "clarify.required",
                        {**data, **pending_data(request_id, "clarify", payload)},
                        turn_id,
                    )
                )
                await self.changed()
        elif kind in {"clarify.expire", "approval.expire"}:
            request_id = payload.get("request_id")
            if not isinstance(request_id, str):
                report(
                    event="hermes_field_invalid",
                    code=f"{kind}.request_id:{type(request_id).__name__}",
                    device=self.device,
                    session=session_id,
                    exc_info=False,
                )
                return
            self.pending.pop(request_id, None)
            await self.publish(
                emit(
                    "interaction.expired",
                    {**data, "request_id": request_id},
                    turn_id,
                )
            )
            await self.changed()
        elif kind == "status.update":
            await self.publish(
                emit(
                    "chat.status",
                    {
                        **data,
                        "kind": str(payload.get("kind") or "status"),
                        "text": str(payload.get("text") or ""),
                    },
                    turn_id,
                )
            )

    def require_live(self, session_id: str) -> dict:
        binding = self.store.session(self.device, session_id)
        if (
            self.connection is None
            or not self.connection.connected
            or binding["runtime_id"] not in self.connection.live_sessions
        ):
            raise HubError(
                "session_disconnected",
                "La sesión no está conectada con Hermes; se intentará reanudar la "
                "conversación al enviar otro mensaje si el turno anterior está cerrado",
            )
        return binding

    def record_resume_progress(self, runtime_id, status: str) -> bool:
        if not isinstance(runtime_id, str):
            return False
        if self.resume_waiter is not None and self.resume_waiter[0] == runtime_id:
            future = self.resume_waiter[1]
            if not future.done():
                future.set_result(status)
            return True
        if self.resuming_ack:
            # One resume RPC can be in flight under the device runtime lock.
            self.early_resume_progress = (runtime_id, status)
            return True
        return False

    async def wait_resume_history(self, binding: dict, connection, hydrating) -> None:
        runtime_id = binding["runtime_id"]
        if self.revoked or not self.store.is_active(self.device):
            connection.live_sessions.discard(runtime_id)
            self.resumed_message_counts.pop(runtime_id, None)
            raise HubError("revoked", "Dispositivo revocado; no se envió el mensaje")
        if self.closed:
            connection.live_sessions.discard(runtime_id)
            self.resumed_message_counts.pop(runtime_id, None)
            raise HubError(
                "session_resume_closed", "Se cerró la sesión de Orbit; no se envió el mensaje"
            )
        status = None
        if self.early_resume_progress is not None:
            early_id, early_status = self.early_resume_progress
            if early_id == runtime_id:
                status = early_status
        self.early_resume_progress = None
        if status is None:
            if hydrating is False:
                return
            if hydrating is not True:
                connection.live_sessions.discard(runtime_id)
                self.resumed_message_counts.pop(runtime_id, None)
                raise HubError(
                    "session_resume_progress_unavailable",
                    "Hermes no confirmó si terminó de cargar el historial; no se envió el mensaje",
                )
            future: asyncio.Future[str] = asyncio.get_running_loop().create_future()
            self.resume_waiter = (runtime_id, future)
            try:
                status = await asyncio.wait_for(future, RESUME_HISTORY_TIMEOUT_SECONDS)
            except TimeoutError as exc:
                connection.live_sessions.discard(runtime_id)
                self.resumed_message_counts.pop(runtime_id, None)
                milliseconds = int(RESUME_HISTORY_TIMEOUT_SECONDS * 1000)
                raise HubError(
                    "session_resume_history_timeout",
                    f"Hermes no confirmó la carga del historial en {milliseconds} ms; "
                    "no se envió el mensaje",
                ) from exc
            finally:
                self.resume_waiter = None
        if status == "revoked":
            connection.live_sessions.discard(runtime_id)
            self.resumed_message_counts.pop(runtime_id, None)
            raise HubError("revoked", "Dispositivo revocado; no se envió el mensaje")
        if status == "closed":
            connection.live_sessions.discard(runtime_id)
            self.resumed_message_counts.pop(runtime_id, None)
            raise HubError(
                "session_resume_closed", "Se cerró la sesión de Orbit; no se envió el mensaje"
            )
        if self.connection is not connection or not connection.connected or status == "lost":
            connection.live_sessions.discard(runtime_id)
            self.resumed_message_counts.pop(runtime_id, None)
            raise HubError(
                "session_resume_connection_lost",
                "Se perdió Hermes durante la carga del historial; no se envió el mensaje",
            )
        if status == "failed":
            connection.live_sessions.discard(runtime_id)
            count = self.resumed_message_counts.pop(runtime_id, None)
            count_text = f" ({count} mensajes en el último tramo)" if count else ""
            raise HubError(
                "session_resume_history_unreadable",
                f"Hermes no pudo leer el historial guardado{count_text}; "
                "no se envió el mensaje. Puedes enviar uno nuevo para volver a reanudar",
            )

    async def resume_if_disconnected(self, session_id: str, connection) -> tuple[dict, bool | None]:
        binding = self.store.session(self.device, session_id)
        if (
            binding["epoch"] == connection.epoch
            and binding["runtime_id"] in connection.live_sessions
        ):
            return binding, False
        self.early_resume_progress = None
        self.resuming_ack = True
        try:
            resumed = await connection.resume_session(binding["stored_id"])
        finally:
            self.resuming_ack = False
        if self.connection is not connection:
            raise HubError(
                "session_resume_connection_replaced",
                "La conexión con Hermes cambió durante la reanudación; no se envió el mensaje",
            )
        if not connection.connected:
            raise HubError(
                "session_resume_connection_lost",
                "Se perdió Hermes durante la reanudación; no se envió el mensaje",
            )
        self.store.resume_session(
            self.device,
            session_id,
            resumed["session_id"],
            resumed["stored_session_id"],
            connection.epoch,
        )
        self.bindings.pop(binding["runtime_id"], None)
        self.resumed_message_counts.pop(binding["runtime_id"], None)
        self.bindings[resumed["session_id"]] = session_id
        count = resumed.get("message_count")
        self.resumed_message_counts[resumed["session_id"]] = (
            count if type(count) is int and count > 0 else None
        )
        return self.store.session(self.device, session_id), resumed.get("hydrating")

    def log_chat_rejection(self, code, numbers=None):
        logger.info(
            "chat_send_rejected",
            extra={
                "event": "chat_send_rejected",
                "code": code,
                "op": current_op(),
                "device": self.device,
                "numbers": numbers,
            },
        )

    async def handle(self, method: str, params, request_id: str) -> tuple[str, dict]:
        if method == "ping":
            return "pong", {}
        if method == "chat.send":
            try:
                return await self._chat_send(params, request_id)
            except HubError as error:
                self.log_chat_rejection(error.code, error.numbers)
                raise
        async with self.lock:
            if not self.store.is_active(self.device):
                raise HubError("revoked", "Dispositivo revocado")
            binding = self.require_live(params.session_id)
            upstream = {"session_id": binding["runtime_id"]}
            if method == "chat.interrupt":
                self.safety_target(method, params)
                await self.connection.rpc("session.interrupt", upstream)
                state = self.turn_state(params.session_id)
                if state in {TurnState.SUBMITTING, TurnState.STREAMING}:
                    # Only Hermes's acknowledgement of the interruption request
                    # moves a live turn to interrupting; the deadline after which
                    # it becomes outcome_unknown starts here. A late submit ACK
                    # cannot move interrupting back to streaming.
                    self.store.status(
                        params.session_id,
                        advance(state, TurnState.INTERRUPTING).value,
                    )
                    self.interrupt_deadlines[params.session_id] = (
                        self.now() + INTERRUPT_DEADLINE_SECONDS
                    )
                return "chat.interrupted", {"session_id": params.session_id, "status": "requested"}
            kind = "approval" if method == "approval.respond" else "clarify"
            pending = self.pending.get(params.request_id)
            if pending is None or pending[:2] != (params.session_id, kind):
                raise HubError("interaction_not_found", "Solicitud ajena, desconocida o caducada")
            turn_id = self.turn_ids.get(params.session_id)
            connection, epoch = self.connection, self.connection.epoch
            if (
                kind == "approval"
                and params.turn_id is not None
                and params.turn_id != self.turn_ids.get(params.session_id)
            ):
                raise HubError("stale_turn", "La respuesta no pertenece al turno actual")
            upstream["request_id"] = params.request_id
            if kind == "approval":
                upstream["choice"] = "once" if params.approved else "deny"
                if upstream["choice"] not in pending[2].get("choices", ["once", "deny"]):
                    raise HubError("approval_not_allowed", "Hermes no permite esta respuesta")
            else:
                upstream["answer"] = params.answer
                if params.question_id:
                    upstream["question_id"] = params.question_id
            result = await connection.rpc(method, upstream)
            # An entry's object identity is its generation: every request/partial
            # update replaces the tuple, including reuse of an identical request ID.
            # The RPC reader remains unlocked and can expire/complete/replace it.
            current = (
                self.pending.get(params.request_id) is pending
                and turn_id is not None
                and self.turn_ids.get(params.session_id) == turn_id
                and self.connection is connection
                and connection.connected
                and connection.epoch == epoch
                and binding["runtime_id"] in connection.live_sessions
                and not self.revoked
                and self.store.is_active(self.device)
                and self.store.session(self.device, params.session_id)["status"] == "streaming"
            )
            try:
                remaining, receipt = decode_response(kind, result)
            except HubError:
                if current:
                    self.pending.pop(params.request_id, None)
                    await self.changed()
                raise
            if not current:
                # Preserve a genuinely applied historical ACK, but never use it
                # to mutate a terminal/replaced interaction. Clients must consume
                # this response without clearing pending; the snapshot is current.
                receipt = {**receipt, "stale_interaction": True, "turn_id": turn_id}
            elif not remaining:
                self.pending.pop(params.request_id, None)
            elif kind == "clarify":
                # Batch replies acknowledge individual qids. Reconnection must
                # not present an already answered question as still pending.
                updated = dict(pending[2])
                if isinstance(questions := updated.get("questions"), list):
                    updated["questions"] = [
                        question
                        for question in questions
                        if isinstance(question, dict)
                        and question.get("qid") in receipt["remaining"]
                    ]
                self.pending[params.request_id] = (params.session_id, kind, updated)
            # Boundary emits the result first, then this authoritative cached snapshot.
            self.revision, self._snapshot_cache = self.revision + 1, None
            return "interaction.expired" if receipt[
                "status"
            ] == "expired" else "interaction.resolved", {
                "session_id": params.session_id,
                "request_id": params.request_id,
                **receipt,
            }

    async def _meeting_turn(self, params) -> MeetingTurn:
        """SIM-30: the question with the meeting the hub holds; refused as
        `meeting_not_found` before anything reaches Hermes."""
        meeting = params.meeting
        lines, started, pending = await self._run_store(
            asyncio.get_running_loop(),
            lambda: read_meeting(self.store, self.device, meeting.recording_id),
        )
        elapsed = None if started is None else round(self.now() * 1000) - started
        return meeting_context(
            params.text,
            meeting.recording_id,
            meeting.title,
            meeting.minutes,
            lines,
            elapsed,
            pending,
        )

    async def _commit_chat_send(
        self,
        params,
        request_id: str,
        session_id: str,
        binding: dict,
        connection,
        meeting: MeetingTurn | None,
    ) -> str:
        """Commit a turn while holding the runtime lock."""
        if self.revoked or not self.store.is_active(self.device):
            raise HubError("revoked", "Dispositivo revocado")
        if self.closed:
            raise HubError(
                "session_resume_closed", "Se cerró la sesión de Orbit; no se envió el mensaje"
            )
        if (
            self.connection is not connection
            or not connection.connected
            or binding["epoch"] != connection.epoch
            or binding["runtime_id"] not in connection.live_sessions
        ):
            raise HubError(
                "session_resume_connection_lost",
                "Se perdió Hermes antes de enviar el mensaje; no se envió",
            )
        self.expire_stale_interrupt(session_id)
        state = self.turn_state(session_id)
        if state is TurnState.OUTCOME_UNKNOWN:
            raise HubError(
                "turn_outcome_unknown",
                "El resultado del turno anterior es incierto; abre una sesión nueva "
                "(session.new) o espera la reconciliación",
            )
        if not can_start_turn(state):
            raise HubError("session_busy", "Espera o interrumpe el turno en curso")
        tracked_input_type = self.input_tracker.consume(self.device, params.text)
        input_type = "llamada de voz en curso" if params.call_active else tracked_input_type
        sent = meeting.text if meeting is not None else params.text
        upstream_text = f"[Orbit · {self.device_name} · {input_type}]\n{sent}"
        self.turn_ids[session_id] = request_id
        # The conversation keeps what Mars sees: a meeting question without
        # the transcript that went with it.
        self.store.append_message(
            session_id,
            "user",
            meeting.visible if meeting is not None else params.text,
            now_ms=round(self.now() * 1000),
            kind="voice" if params.voice is not None else "text",
            duration_ms=params.voice.duration_ms if params.voice is not None else None,
            client_id=params.client_id,
        )
        if state is not TurnState.IDLE:
            # completed/failed first return to idle; the table has no
            # direct edge back into submitting.
            self.store.status(session_id, TurnState.IDLE.value)
        self.store.status(session_id, advance(TurnState.IDLE, TurnState.SUBMITTING).value)
        # Publish before prompt.submit: upstream may emit before its RPC acknowledgement.
        await self.publish(emit("chat.session", {"session_id": session_id}, request_id))
        return upstream_text

    async def _chat_send(self, params, request_id: str) -> tuple[str, dict]:
        """Submit one chat turn on an explicit state machine.

        The runtime lock covers admission and commitment, but not history
        hydration or prompt.submit. A reservation prevents another send to the
        same session while hydration is pending; controls can still answer.
        """
        meeting = await self._meeting_turn(params) if params.meeting is not None else None
        async with self.lock:
            if not self.store.is_active(self.device):
                raise HubError("revoked", "Dispositivo revocado")
            session_id = params.session_id
            if session_id is not None:
                self.store.session(self.device, session_id)
            if session_id in self.resume_pending:
                remaining = (
                    None
                    if self.resume_deadline is None
                    else math.ceil(
                        max(0.0, self.resume_deadline - asyncio.get_running_loop().time())
                    )
                )
                raise HubError(
                    "session_resume_in_progress",
                    "Hermes sigue cargando el historial de esta conversación; espera"
                    + (
                        ""
                        if remaining is None
                        else f": quedan hasta {remaining} s del plazo de "
                        f"{RESUME_HISTORY_TIMEOUT_SECONDS:g} s"
                    ),
                )
            connection = await self.connect()
            if self.resume_deadline is not None and session_id is not None:
                existing = self.store.session(self.device, session_id)
                if (
                    existing["epoch"] != connection.epoch
                    or existing["runtime_id"] not in connection.live_sessions
                ):
                    remaining = math.ceil(
                        max(0.0, self.resume_deadline - asyncio.get_running_loop().time())
                    )
                    raise HubError(
                        "session_resume_in_progress",
                        "Hermes ya carga otro historial en este dispositivo; "
                        f"quedan hasta {remaining} s del plazo de "
                        f"{RESUME_HISTORY_TIMEOUT_SECONDS:g} s",
                    )
            if session_id is None:
                created = await connection.create_session()
                if self.revoked or not self.store.is_active(self.device):
                    raise HubError("revoked", "Dispositivo revocado")
                session_id = self.store.create_session(
                    self.device,
                    created["session_id"],
                    created["stored_session_id"],
                    connection.epoch,
                    self.now(),
                )
                self.bindings[created["session_id"]] = session_id
            self.expire_stale_interrupt(session_id)
            state = self.turn_state(session_id)
            if state is TurnState.OUTCOME_UNKNOWN:
                # Never resend over a session whose previous turn may already
                # have reached Hermes: the old turn identity stays untouched.
                raise HubError(
                    "turn_outcome_unknown",
                    "El resultado del turno anterior es incierto; abre una sesión"
                    " nueva (session.new) o espera la reconciliación",
                )
            if not can_start_turn(state):
                raise HubError("session_busy", "Espera o interrumpe el turno en curso")
            binding, hydrating = await self.resume_if_disconnected(session_id, connection)
            early = self.early_resume_progress
            needs_wait = hydrating is True and not (
                early is not None and early[0] == binding["runtime_id"]
            )
            if needs_wait:
                self.resume_pending.add(session_id)
                self.resume_deadline = (
                    asyncio.get_running_loop().time() + RESUME_HISTORY_TIMEOUT_SECONDS
                )
            else:
                await self.wait_resume_history(binding, connection, hydrating)
                upstream_text = await self._commit_chat_send(
                    params, request_id, session_id, binding, connection, meeting
                )
        if needs_wait:
            try:
                await self.wait_resume_history(binding, connection, hydrating)
                async with self.lock:
                    upstream_text = await self._commit_chat_send(
                        params, request_id, session_id, binding, connection, meeting
                    )
            finally:
                self.resume_pending.discard(session_id)
                self.resume_deadline = None
        upstream = {"session_id": binding["runtime_id"], "text": upstream_text}
        try:
            await connection.rpc("prompt.submit", upstream)
        except HubError as exc:
            # The submit may have reached Hermes; never resend it. A terminal
            # event observed during the RPC keeps its more authoritative state.
            async with self.lock:
                if exc.code == "session_runtime_not_found":
                    connection.live_sessions.discard(binding["runtime_id"])
                    self.resumed_message_counts.pop(binding["runtime_id"], None)
                    self.store.status(session_id, TurnState.FAILED.value)
                    await self.changed()
                else:
                    self.apply_state(session_id, TurnState.OUTCOME_UNKNOWN)
            raise
        async with self.lock:
            self.apply_state(session_id, TurnState.STREAMING)
            # An interrupt that landed while prompt.submit waited already moved
            # the persisted state to interrupting; apply_state skips the
            # forbidden hop back, and this reply must not claim it either.
            status = self.turn_state(session_id).value
        return "chat.session", {"session_id": session_id, "status": status}

    async def revoke(self) -> None:
        """Stop this device's live sessions even when it has no connected viewers.

        Interruption is best effort and cannot undo an action already executed upstream.
        This path is not used for a normal client/TUI disconnect.
        """
        if self.revoked:
            return
        self.revoked = True
        if self.resume_waiter is not None and not self.resume_waiter[1].done():
            self.resume_waiter[1].set_result("revoked")
        self.pending.clear()
        await self.publish(emit("revoked", {}))
        connection = self.connection
        if connection and connection.connected:
            try:
                async with asyncio.timeout(2):
                    await asyncio.gather(
                        *(
                            connection.rpc("session.interrupt", {"session_id": runtime_id})
                            for runtime_id in tuple(connection.live_sessions)
                        ),
                        return_exceptions=True,
                    )
            except TimeoutError:
                pass
        await self.close()

    async def close(self) -> None:
        self.closed = True
        if self.resume_waiter is not None and not self.resume_waiter[1].done():
            self.resume_waiter[1].set_result("closed")
        for task in list(self.reconciliations.values()):
            task.cancel()
        await asyncio.gather(*list(self.reconciliations.values()), return_exceptions=True)
        if self.connection:
            try:
                async with asyncio.timeout(3):
                    await self.connection.close()
            except Exception:  # noqa: BLE001 - redact close failure and preserve global revocation
                report(
                    event="runtime_close_failed",
                    code="close_failed",
                    device=self.device,
                )
                return
