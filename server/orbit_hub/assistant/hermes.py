"""Hermes as the hub's assistant and voice. Device credentials never reach Hermes.

The chat goes over Hermes's gateway WebSocket (`/api/ws`, JSON-RPC); this
module turns its methods and events into the typed interface of `backend`.
Dictation and read-aloud go over the same Hermes's HTTP audio endpoints.
Each agent of kind `hermes` is one Hermes profile on the hub's one Hermes
(`hermes_url`, `hermes_credential_file`).
"""

from __future__ import annotations

import asyncio
import base64
import json
import re
from collections.abc import Sequence
from contextlib import suppress
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any
from urllib.parse import urlencode
from uuid import uuid4

import httpx
from websockets.asyncio.client import connect

from ..log import clear_op, logger, report
from ..protocol import HubError
from ..secret_file import read_secret
from ..transcription_failure import from_provider
from .backend import (
    ActivityItem,
    Capabilities,
    ClarificationReceipt,
    ClarificationRequest,
    ClarificationRequested,
    ConnectionLost,
    ContentBlock,
    EventSink,
    HistoryLoaded,
    MessageChunk,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    PlanStep,
    RequestExpired,
    ResumedSession,
    StatusNotice,
    TurnError,
    UnreadableReceipt,
    UnreadableUpdate,
)
from .speech import MAX_SPEECH_CLIP_BYTES, SpeechClip

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

# How Hermes begins the error of a resume whose stored transcript it could not
# read; the rest of its message is Hermes's detail and is never relayed.
RESUME_HISTORY_ERROR_PREFIX = "Could not reopen that session (its transcript could not be read)."
# Hermes errors that end a turn before the agent ran, without a message.complete.
TERMINAL_ERRORS = {'Context injection refused.', 'Turn cancelled before the agent was ready', 'Session no longer running before the agent was ready'}  # fmt: skip
MAX_SPEECH_RESPONSE_BYTES = 6 * 1024 * 1024
# Existing 180 s provider and 10 s connection budgets, shared with the
# refusal's millisecond numbers. They bound one dictation call, not meeting
# ASR, queue wait, or an automatic retry.
TRANSCRIPTION_TIMEOUT_SECONDS = 180
TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS = 10


def credential(settings: Settings) -> str:
    value = read_secret(settings.hermes_credential_file, 64 * 1024).decode().strip()
    if not value or len(value) > 4096:
        raise HubError("configuration", "Credencial Hermes ausente o no válida")
    return value


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


@dataclass(frozen=True)
class HermesAgent:
    """What an agent of kind `hermes` configures: the profile that answers."""

    profile: str


def parse_agent(keys: dict[str, Any]) -> HermesAgent:
    """An `[[agents]]` entry's own keys for kind `hermes` (see `agents.KINDS`)."""
    unknown = sorted(set(keys) - {"profile"})
    if unknown:
        raise ValueError(f'kind "hermes" does not take {", ".join(unknown)}')
    if "profile" not in keys:
        raise ValueError('kind "hermes" needs profile')
    profile = keys["profile"]
    if not isinstance(profile, str) or not 1 <= len(profile) <= 64 or "/" in profile:
        raise ValueError(
            f"profile {json.dumps(profile, ensure_ascii=False)} must be 1 to 64 characters "
            "without /"
        )
    return HermesAgent(profile)


def agent_backend(
    settings: Settings, agent: AgentConfiguration, instructions=None
) -> HermesBackend:
    """The profile answers with its own SOUL.md and skills: the hub's
    `instructions` are not Hermes's (they are edited through its panel)."""
    return HermesBackend(settings, agent.settings.profile)


def agent_speech(settings: Settings, agent: AgentConfiguration) -> HermesSpeech:
    """Dictation and read-aloud by the agent's own profile."""
    return HermesSpeech(settings, agent.settings.profile)


class HermesConnection:
    """One device's gateway WebSocket, and what its events mean to the hub."""

    def __init__(
        self, backend: HermesBackend | None, on_event: EventSink, device: str, device_name: str
    ):
        if not 1 <= len(device_name) <= 80 or any(ord(char) < 32 for char in device_name):
            raise HubError("configuration", "Nombre de dispositivo no válido")
        self.backend, self.on_event = backend, on_event
        self.device, self.device_name = device, device_name
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self._ws = None
        self._reader = None
        self._pending: dict[str, asyncio.Future] = {}
        self._ready = asyncio.Event()
        self._closing = False
        # Sessions whose turn has published a plan: its next update is progress.
        self._planned: set[str] = set()

    @property
    def connected(self) -> bool:
        return self._reader is not None and not self._reader.done()

    async def open(self) -> None:
        try:
            # One Hermes message may be 4 MiB, and 32 of them may wait unread
            # before the socket stops reading: at most 128 MiB held per
            # device's connection. They protect the hub's memory from Hermes;
            # the sizes of real Hermes messages were not measured here. They do
            # not limit a reply, which streams as many messages, nor a resume.
            self._ws = await connect(
                self.backend.ws_url("/api/ws"),
                max_size=4 * 1024 * 1024,
                max_queue=32,
                open_timeout=10,
                close_timeout=2,
            )
            self._reader = asyncio.create_task(self._read())
            await asyncio.wait_for(self._ready.wait(), 15)
            if not self.epoch:
                raise ValueError("Missing gateway.ready replay_epoch")
        except Exception as exc:
            report(event="hermes_connect_failed", code="upstream_unavailable")
            await self.close()
            raise HubError("upstream_unavailable", "No se pudo conectar con Hermes") from exc

    async def _read(self) -> None:
        # The connection outlives the request that opened it.
        clear_op()
        try:
            async for raw in self._ws:
                if not isinstance(raw, str):
                    continue
                frame = json.loads(raw)
                if not isinstance(frame, dict):
                    continue
                if frame.get("method") == "event":
                    params = frame.get("params", {})
                    if params.get("type") == "gateway.ready":
                        self.epoch = str(params.get("payload", {}).get("replay_epoch") or "")
                        self._ready.set()
                    else:
                        await self.receive(params)
                elif (future := self._pending.pop(str(frame.get("id")), None)) is not None:
                    if not future.done():
                        future.set_result(frame)
        except Exception:  # noqa: BLE001 - failure becomes a typed disconnect event in finally
            # Exceptions may contain auth query strings; no raw upstream logging.
            report(event="hermes_reader_failed", code="upstream_lost")
            self.epoch = ""
        finally:
            self._ready.set()
            self.live_sessions.clear()
            for future in self._pending.values():
                if not future.done():
                    future.set_exception(
                        HubError(
                            "outcome_unknown",
                            "Conexión Hermes perdida; no se repetirá la operación",
                        )
                    )
            self._pending.clear()
            if not self._closing:
                await self.on_event(ConnectionLost())

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
                    session_id, "Hermes envió un payload que no es un objeto JSON; se ignoró"
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
            return
        if kind == "error":
            message = payload.get("message")
            if isinstance(message, str) and message.startswith(RESUME_HISTORY_ERROR_PREFIX):
                cause = "history_unreadable"
            elif isinstance(message, str) and message in TERMINAL_ERRORS:
                cause = "terminal"
            else:
                cause = "unconfirmed"
            await self.on_event(TurnError(session_id, cause))
        elif kind == "message.delta":
            if isinstance(text := payload.get("text"), str):
                await self.on_event(MessageChunk(session_id, text))
        elif kind == "message.complete":
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
        elif kind == "approval.request":
            if isinstance(request_id := payload.get("request_id"), str):
                description = str(
                    payload.get("description")
                    or payload.get("command")
                    or "Hermes solicita autorización"
                )
                await self.on_event(
                    PermissionRequested(
                        session_id, PermissionRequest(request_id, description, _choices(payload))
                    )
                )
        elif kind == "clarify.request":
            if isinstance(request_id := payload.get("request_id"), str):
                question = str(payload.get("question") or "Responde a las preguntas de Hermes")
                await self.on_event(
                    ClarificationRequested(
                        session_id,
                        ClarificationRequest(
                            request_id, question, self._questions(kind, payload), _choices(payload)
                        ),
                    )
                )
        elif kind in {"clarify.expire", "approval.expire"}:
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
        elif kind in {"tool.start", "tool.complete"}:
            await self._tool(session_id, kind, payload)
        elif kind == "todo.updated":
            await self._plan_update(session_id, _plan(payload.get("todos")))
        elif kind == "reasoning.available":
            # The reasoning's summary, when the profile shows reasoning. Its
            # deltas (`reasoning.delta`) are the thought itself and
            # `thinking.delta` a spinner's text: neither is shown.
            if isinstance(text := payload.get("text"), str) and text.strip():
                await self.on_event(
                    ActivityItem(
                        session_id, f"reasoning-{uuid4()}", "reasoning", "completed", text=text
                    )
                )
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

    async def rpc(self, method: str, params: dict) -> dict:
        if not self.connected:
            raise HubError("upstream_unavailable", "Hermes no está conectado")
        if method not in {
            "session.create",
            "session.resume",
            "session.status",
            "prompt.submit",
            "session.interrupt",
            "approval.respond",
            "clarify.respond",
        }:
            raise HubError("method_forbidden", "Método Hermes fuera de contrato")
        request_id = str(uuid4())
        future = asyncio.get_running_loop().create_future()
        self._pending[request_id] = future
        # Existing RPC receipt budget; it bounds one acknowledgement, not a model turn.
        rpc_timeout_seconds = 30
        try:
            await self._ws.send(
                json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
            )
            frame = await asyncio.wait_for(future, rpc_timeout_seconds)
            if "error" in frame:
                error = frame["error"] if isinstance(frame["error"], dict) else {}
                code = error.get("code")
                if method == "prompt.submit" and code == 4001:
                    self.live_sessions.discard(params.get("session_id"))
                    raise HubError(
                        "session_runtime_not_found",
                        "Hermes no encontró la sesión activa (código 4001); "
                        "el mensaje no se aceptó y puedes volver a enviarlo tras reanudar",
                    )
                if method == "session.resume":
                    reason = error.get("message")
                    if code == -32601:
                        raise HubError(
                            "session_resume_unsupported",
                            "Esta versión de Hermes no permite reanudar sesiones guardadas "
                            "(código -32601)",
                        )
                    if code == 4007 and reason == "session not found":
                        raise HubError(
                            "session_resume_not_found",
                            "Hermes no conserva la conversación guardada (código 4007); "
                            "el historial local sigue disponible",
                        )
                    if code == 4007 and reason == "session no longer live; retry resume":
                        raise HubError(
                            "session_resume_race",
                            "Hermes cerró la sesión durante la reanudación (código 4007); "
                            "vuelve a intentarlo",
                        )
                    if code == 4130:
                        numbers = (
                            re.search(r"\((\d+) messages; limit (\d+)\)", reason)
                            if isinstance(reason, str)
                            else None
                        )
                        if numbers is not None:
                            raise HubError(
                                "session_resume_too_large",
                                f"Hermes no puede reanudar {numbers[1]} mensajes; "
                                f"límite {numbers[2]} (código 4130)",
                            )
                        raise HubError(
                            "session_resume_too_large",
                            "Hermes rechazó reanudar por tamaño (código 4130); "
                            "no entregó cuenta y límite",
                        )
                    if code == 4009 and reason == "session disconnect interrupt settling":
                        raise HubError(
                            "session_resume_busy",
                            "Hermes sigue cerrando el turno anterior (código 4009); espera",
                        )
                    if (
                        code == 5000
                        and isinstance(reason, str)
                        and reason.startswith("Session storage is unavailable:")
                    ):
                        raise HubError(
                            "session_resume_storage_unavailable",
                            "Hermes no puede abrir su almacén de sesiones (código 5000)",
                        )
                    if (
                        code == 5000
                        and isinstance(reason, str)
                        and reason.startswith(RESUME_HISTORY_ERROR_PREFIX)
                    ):
                        raise HubError(
                            "session_resume_history_unreadable",
                            "Hermes no pudo leer el historial guardado (código 5000); "
                            "no se envió el mensaje",
                        )
                raise HubError("upstream_rejected", f"Hermes rechazó la operación (código {code})")
            if not isinstance(result := frame.get("result"), dict):
                raise HubError("upstream_protocol", "Respuesta Hermes no válida")
            return result
        except TimeoutError as exc:
            if method == "session.resume":
                raise HubError(
                    "session_resume_timeout",
                    f"Hermes no confirmó la reanudación; plazo de {rpc_timeout_seconds} segundos; "
                    "no se envió el mensaje",
                ) from exc
            raise HubError(
                "outcome_unknown", "Hermes no confirmó la operación; no se repetirá"
            ) from exc
        finally:
            self._pending.pop(request_id, None)

    async def create_session(self) -> OpenedSession:
        result = await self.rpc(
            "session.create",
            {
                "profile": self.backend.profile,
                "close_on_disconnect": False,
                "title": f"Orbit · {self.device_name}",
                "source": "orbit",
                "follow_profile_config": True,
            },
        )
        if not result.get("session_id") or not result.get("stored_session_id"):
            raise HubError("upstream_protocol", "Hermes omitió la identidad durable")
        self.live_sessions.add(result["session_id"])
        return OpenedSession(result["session_id"], result["stored_session_id"])

    async def resume_session(self, stored_id: str) -> ResumedSession:
        try:
            result = await self.rpc(
                "session.resume",
                {
                    "profile": self.backend.profile,
                    "session_id": stored_id,
                    "source": "orbit",
                    "close_on_disconnect": False,
                    # Bound the WS reply while Hermes hydrates model history.
                    # omit_messages=True would load only the tip segment.
                    "defer_history": True,
                    "omit_messages": False,
                },
            )
        except HubError as exc:
            if exc.code == "outcome_unknown":
                raise HubError(
                    "session_resume_connection_lost",
                    "Se perdió Hermes durante la reanudación; no se envió el mensaje",
                ) from exc
            raise
        runtime_id = result.get("session_id")
        if not isinstance(runtime_id, str) or not runtime_id:
            raise HubError("upstream_protocol", "Hermes omitió la sesión reanudada")
        resumed = result.get("resumed") or result.get("stored_session_id") or stored_id
        if not isinstance(resumed, str) or not resumed:
            raise HubError("upstream_protocol", "Hermes omitió la identidad guardada")
        self.live_sessions.add(runtime_id)
        count = result.get("message_count")
        hydrating = result.get("hydrating")
        return ResumedSession(
            runtime_id,
            resumed,
            count if type(count) is int and count > 0 else None,
            hydrating if type(hydrating) is bool else None,
        )

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        # Hermes takes a turn as one text; the hub sends it one text block.
        text = "".join(block.text for block in blocks)
        await self.rpc("prompt.submit", {"session_id": session_id, "text": text})

    async def cancel(self, session_id: str) -> None:
        await self.rpc("session.interrupt", {"session_id": session_id})

    async def respond_permission(
        self, session_id: str, request_id: str, *, choice: str
    ) -> PermissionReceipt:
        # Hermes's own vocabulary (`once`, `session`, `always`, `deny`): the
        # runtime admits only a choice the request offered.
        result = await self.rpc(
            "approval.respond",
            {"session_id": session_id, "request_id": request_id, "choice": choice},
        )
        resolved = result.get("resolved")
        if type(resolved) is not int or resolved not in {0, 1}:
            raise UnreadableReceipt(
                "upstream_protocol", "Hermes no confirmó una aprobación individual; no se repetirá"
            )
        return PermissionReceipt(applied=resolved == 1)

    async def respond_clarification(
        self, session_id: str, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt:
        params = {"session_id": session_id, "request_id": request_id, "answer": answer}
        if question_id:
            params["question_id"] = question_id
        result = await self.rpc("clarify.respond", params)
        status, remaining = result.get("status"), result.get("remaining", [])
        if (
            status not in {"ok", "expired"}
            or not isinstance(remaining, list)
            or len(remaining) > 32
            or not all(isinstance(item, str) and 1 <= len(item) <= 128 for item in remaining)
        ):
            raise UnreadableReceipt(
                "upstream_protocol", "Hermes no confirmó la aclaración; no se repetirá"
            )
        return ClarificationReceipt(status, tuple(remaining))

    async def turn_stopped(self, session_id: str) -> bool:
        result = await self.rpc("session.status", {"session_id": session_id})
        output = result.get("output")
        lines = output.splitlines() if isinstance(output, str) else []
        return lines.count("Agent Running: No") == 1 and "Agent Running: Yes" not in lines

    async def close(self) -> None:
        self._closing = True
        try:
            if self._ws is not None:
                with suppress(TimeoutError):
                    await asyncio.wait_for(self._ws.close(), 2.5)
        finally:
            if self._reader is not None and self._reader is not asyncio.current_task():
                self._reader.cancel()
                await asyncio.gather(self._reader, return_exceptions=True)


class HermesBackend:
    """The assistant is the Hermes profile `profile`."""

    # Images: Orbit's turns carry none (see `Capabilities.images`).
    capabilities = Capabilities(images=False, resume=True, history=True, voice=True)

    def __init__(self, settings: Settings, profile: str):
        self.settings, self.profile = settings, profile

    def ws_url(self, path: str) -> str:
        base = self.settings.hermes_url.rstrip("/")
        base = ("wss" if base.startswith("https:") else "ws") + base[base.index(":") :]
        return base + path + "?" + urlencode({"token": credential(self.settings)})

    def connection(self, device: str, device_name: str, on_event: EventSink) -> HermesConnection:
        return HermesConnection(self, on_event, device, device_name)


class HermesSpeech:
    """Dictation and read-aloud by the Hermes profile `profile`."""

    def __init__(self, settings: Settings, profile: str):
        self.settings, self.profile = settings, profile

    async def transcribe(self, audio: bytes) -> str:
        async with httpx.AsyncClient(
            timeout=httpx.Timeout(
                TRANSCRIPTION_TIMEOUT_SECONDS, connect=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS
            ),
            follow_redirects=False,
        ) as client:
            try:
                response = await client.post(
                    self.settings.hermes_url.rstrip("/") + "/api/audio/transcribe",
                    params={"profile": self.profile},
                    headers={"X-Hermes-Session-Token": credential(self.settings)},
                    json={
                        "data_url": "data:audio/wav;base64," + base64.b64encode(audio).decode(),
                        "mime_type": "audio/wav",
                    },
                )
                response.raise_for_status()
                payload = response.json()
                if payload.get("ok") is not True or not isinstance(payload.get("transcript"), str):
                    raise ValueError("Invalid transcription response")
                return payload["transcript"]
            except Exception as exc:
                report(event="hermes_transcription_failed", code="transcription_failed")
                raise from_provider(
                    exc,
                    timeout_ms=TRANSCRIPTION_TIMEOUT_SECONDS * 1000,
                    connect_timeout_ms=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS * 1000,
                ) from exc

    async def synthesize(self, text: str) -> SpeechClip:
        try:
            # 10 s to connect to Hermes and 180 s for each read, write or pooled
            # connection, as for dictation: it protects the request from a
            # Hermes that stops answering. It is not a total: a slow stream is
            # bounded by its 6 MiB, not by time. The Mac gives up on this route
            # after 90 s (HUB_HTTP_TIMEOUT in orbit-daemon's hub.rs), so past
            # that the hub waits for nobody; 180 s was not measured for speech.
            async with (
                httpx.AsyncClient(
                    timeout=httpx.Timeout(180, connect=10), follow_redirects=False
                ) as client,
                client.stream(
                    "POST",
                    self.settings.hermes_url.rstrip("/") + "/api/audio/speak",
                    headers={"X-Hermes-Session-Token": credential(self.settings)},
                    json={"text": text, "profile": self.profile},
                ) as response,
            ):
                response.raise_for_status()
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(chunk) > MAX_SPEECH_RESPONSE_BYTES - len(body):
                        # Hermes's whole answer (JSON around the base64 clip),
                        # cut while it streams: `bytes` is what had arrived.
                        raise HubError(
                            "speech_too_large",
                            "La respuesta de voz de Hermes supera el límite de 6 MiB",
                            numbers={
                                "bytes": len(body) + len(chunk),
                                "max_bytes": MAX_SPEECH_RESPONSE_BYTES,
                            },
                        )
                    body.extend(chunk)
            payload = json.loads(body)
            mime_type = payload.get("mime_type")
            provider = payload.get("provider")
            data_url = payload.get("data_url")
            if (
                payload.get("ok") is not True
                or not isinstance(mime_type, str)
                or not 1 <= len(mime_type) <= 128
                or any(ord(character) < 33 or ord(character) > 126 for character in mime_type)
                or not isinstance(provider, str)
                or not 1 <= len(provider) <= 128
                or any(ord(character) < 32 or ord(character) > 126 for character in provider)
                or not isinstance(data_url, str)
            ):
                raise ValueError("Invalid speech response")
            prefix, separator, encoded = data_url.partition(",")
            if separator != "," or prefix != f"data:{mime_type};base64":
                raise ValueError("Invalid speech data URL")
            maximum_encoded = 4 * ((MAX_SPEECH_CLIP_BYTES + 2) // 3)
            if len(encoded) > maximum_encoded:
                # The clip's size from its base64 length, before decoding it.
                raise HubError(
                    "speech_too_large",
                    "El clip de voz supera el límite de 4 MiB",
                    numbers={
                        "bytes": len(encoded) // 4 * 3 - (len(encoded) - len(encoded.rstrip("="))),
                        "max_bytes": MAX_SPEECH_CLIP_BYTES,
                    },
                )
            audio = base64.b64decode(encoded, validate=True)
            if not audio:
                raise ValueError("Empty speech clip")
            if len(audio) > MAX_SPEECH_CLIP_BYTES:
                raise HubError(
                    "speech_too_large",
                    "El clip de voz supera el límite de 4 MiB",
                    numbers={"bytes": len(audio), "max_bytes": MAX_SPEECH_CLIP_BYTES},
                )
            logger.info(
                "hermes_speech_clip",
                extra={"event": "hermes_speech_clip", "code": f"{provider}|{mime_type}"},
            )
            return SpeechClip(mime_type, provider, audio)
        except HubError:
            raise
        except Exception as exc:
            report(event="hermes_speech_clip_failed", code="speech_unavailable")
            raise HubError(
                "speech_unavailable", "Hermes no entregó el clip de voz configurado"
            ) from exc
