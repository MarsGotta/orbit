"""The private route an assistant uses to ask a Mac something, and get an answer.

This is the only way in from outside. Everything it accepts is signed with a
dedicated key over the exact body. The hub validates its envelope and size;
capability parameters are opaque here. The Mac validates them before acting.

Three decisions worth naming:

- The request id is minted here, never accepted from the caller. A caller that
  chose its own ids could reuse one while its question is still open, and a late
  answer to the old question would then resolve the new one.
- A device that does not exist and a device that is not connected are answered
  identically. The caller holds a key, which does not entitle it to learn which
  identities the hub knows.
- The wait is bounded and the failure is named. A caller that cannot tell
  "refused" from "nobody answered" will retry a refusal forever.
"""

import asyncio
import base64
import uuid
from dataclasses import dataclass
from functools import partial
from time import monotonic
from typing import Literal

from pydantic import Field, ValidationError

from .assistant.access import CAPABILITY_GROUP, disabled_message
from .device_bridge import (
    ACTION_REPLY_TIMEOUT_SECONDS,
    REQUEST_TIMEOUT_SECONDS,
    BridgeRefused,
)
from .errors import PrivateError as BridgeError
from .errors import render
from .log import report
from .private_auth import Authenticator, BridgeConfiguration, PrivateModel, read_key
from .private_route import json_response, private_route
from .protocol import DeviceSummary, emit
from .turns import TurnState

PROTOCOL = "orbit_device_bridge_v1"
RESULT_PROTOCOL = "orbit_device_bridge_result_v1"
PATH = "/internal/v1/device/request"
ACTION_RESULT_PATH = "/internal/v1/device/action-result"
DEVICES_PATH = "/internal/v1/devices"
# What the agent orbit-mcp serves may use: its access groups.
ACCESS_PATH = "/internal/v1/agent/access"
# An agent's id as `[[agents]]` fixes it (`assistant.agents.AGENT_ID`).
AGENT_ID_PATTERN = r"^[a-z0-9-]{1,32}$"
HEADER_PREFIX = "x-orbit-bridge-"
# The translator's canonical action plus a measured 235-byte envelope using a
# 128-character ASCII fixture device id. The field limit counts characters,
# not bytes; paired ids are UUIDs. This bounds the private HTTP allocation,
# not the action itself or the action-result lookup.
MAX_ACTION_BYTES = 16_384
ACTION_ENVELOPE_BYTES = 235
REQUEST_BYTES = MAX_ACTION_BYTES + ACTION_ENVELOPE_BYTES
# Preserve the old gate for requests that do not carry an action.
NON_ACTION_REQUEST_BYTES = 8192
# Two at once, not more: the Mac serves one request at a time by design, so a
# deeper queue here would only buy longer waits and a fuller memory.
CONCURRENT_REQUESTS = 2


class BridgeRequest(PrivateModel):
    protocol: Literal[PROTOCOL]
    device_id: str = Field(min_length=1, max_length=128)
    capability: str = Field(min_length=1, max_length=64)
    parameters: dict = Field(default_factory=dict)
    # The agent whose orbit-mcp asks (`ORBIT_MCP_AGENT`); none is the default agent.
    agent_id: str | None = Field(default=None, pattern=AGENT_ID_PATTERN)


class ActionResultRequest(PrivateModel):
    protocol: Literal[PROTOCOL]
    device_id: str = Field(min_length=1, max_length=128)
    operation_id: str = Field(min_length=36, max_length=36)
    agent_id: str | None = Field(default=None, pattern=AGENT_ID_PATTERN)


class AccessRequest(PrivateModel):
    protocol: Literal[PROTOCOL]
    agent_id: str | None = Field(default=None, pattern=AGENT_ID_PATTERN)


# What an action that has been handed to the Mac and has not finished
# yet is called. It used to be `awaiting_approval`, which said the opposite of
# what was happening — the action was on its way, nobody was being asked — and
# the assistant repeated the label to the user as if their page were still closed.
IN_FLIGHT = "in_progress"

# Completed operations remain readable after an upgrade. Older journals can
# hold outcomes from older Macs; new Mac receipts are handled only by _action_outcome.
ACTION_OUTCOMES = frozenset({IN_FLIGHT, "performed", "unknown", "failed", "expired", "rejected"})


@dataclass(frozen=True, repr=False)
class BridgeReply:
    status: int
    payload: dict


class BridgeService:
    """Carries bounded questions to a Mac and journals human-paced actions."""

    def __init__(
        self,
        configuration,
        runtimes,
        bridge,
        waiters,
        timestamp,
        captures,
        store,
        access=None,
    ):
        # `assistant.access.AgentAccess`: refuses a request of a group its
        # agent has off. None checks nothing.
        self.access = access
        self.configuration = (
            BridgeConfiguration.model_validate(configuration) if configuration is not None else None
        )
        self.runtimes, self.bridge, self.waiters = runtimes, bridge, waiters
        self.timestamp, self.monotonic = timestamp, monotonic
        self.captures, self.store = captures, store
        self.requests = asyncio.Semaphore(CONCURRENT_REQUESTS)
        self.wait_for_reply = asyncio.wait_for
        self.auth = None
        self.action_result_auth = None
        self.devices_auth = None
        self.access_auth = None
        self._action_tasks: set[asyncio.Task] = set()
        self._active_action_operations: set[str] = set()
        self._closed = False

    def start(self):
        if self._closed:
            raise BridgeError(503, "unavailable")
        if self.auth is not None or self.configuration is None:
            return
        key = read_key(self.configuration.credential_file, BridgeError)

        # One key, one authenticator per route: a signature names its path.
        def authenticator(path):
            return Authenticator(
                self.configuration.key_id,
                key,
                protocol=PROTOCOL,
                path=path,
                header_prefix=HEADER_PREFIX,
                failure=BridgeError,
            )

        self.auth = authenticator(PATH)
        self.action_result_auth = authenticator(ACTION_RESULT_PATH)
        self.devices_auth = authenticator(DEVICES_PATH)
        self.access_auth = authenticator(ACCESS_PATH)

    def close(self):
        self._closed = True
        self.auth = None
        self.action_result_auth = None
        self.devices_auth = None
        self.access_auth = None
        for task in self._action_tasks:
            task.cancel()
        self._action_tasks.clear()
        self._active_action_operations.clear()

    def check(self):
        if self._closed or self.auth is None:
            raise BridgeError(503, "unavailable")

    def devices(self, headers) -> list[dict]:
        self.check()
        self.devices_auth.admit(headers, b"", int(self.timestamp()), self.monotonic())
        result = []
        for row in self.store.active_devices():
            runtime = self.runtimes.get(row["id"])
            connected = bool(runtime is not None and not runtime.revoked and runtime.listeners)
            summary = DeviceSummary(
                **row,
                connected=connected,
                assistant_turn_active=connected
                and any(
                    runtime.turn_state(session_id)
                    in {TurnState.SUBMITTING, TurnState.STREAMING, TurnState.INTERRUPTING}
                    for session_id in runtime.turn_ids
                ),
            )
            result.append(summary.model_dump())
        return result

    async def ask(self, headers, body) -> BridgeReply:
        self.check()
        self.auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        try:
            request = BridgeRequest.model_validate_json(body)
        except ValidationError as error:
            first = error.errors(include_input=False, include_context=False)[0]
            field = ".".join(str(part) for part in first["loc"])
            raise BridgeError(
                400,
                "invalid_request",
                detail={
                    "field": field,
                    "expected": "objeto"
                    if first["type"] == "dict_type"
                    else "campo válido del sobre",
                },
            ) from None
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if request.capability != "perform_action" and len(body) > NON_ACTION_REQUEST_BYTES:
            raise BridgeError(
                400, "invalid_request", {"bytes": len(body), "max_bytes": NON_ACTION_REQUEST_BYTES}
            )
        group = CAPABILITY_GROUP.get(request.capability)
        if self.access is not None and group is not None:
            self.access.require(request.agent_id, group)
        runtime = self.runtimes.get(request.device_id)
        # Unknown and disconnected are answered the same on purpose: holding the
        # key does not entitle a caller to enumerate this hub's devices.
        if runtime is None or runtime.revoked or not runtime.listeners:
            raise BridgeError(404, "device_unavailable")
        request_id = str(uuid.uuid4())
        loop = asyncio.get_running_loop()
        waiter: asyncio.Future = loop.create_future()
        try:
            self.bridge.open(request_id, request.device_id, request.capability, self.monotonic())
        except BridgeRefused as refusal:
            raise BridgeError(429, refusal.code, refusal.numbers) from None
        self.waiters[request_id] = waiter
        if request.capability == "perform_action":
            return await self._start_action(request, request_id, runtime, waiter)
        started_waiting = self.monotonic()
        try:
            await runtime.publish(
                emit(
                    "device.request",
                    {
                        "request_id": request_id,
                        "capability": request.capability,
                        "parameters": request.parameters,
                    },
                )
            )
            answer = await asyncio.wait_for(waiter, REQUEST_TIMEOUT_SECONDS)
            # A real reply is stronger activity evidence than an open socket.
            # Keep the durable inventory current for both answers and refusals.
            self.store.mark_device_seen(request.device_id, self.timestamp() / 1000)
        except TimeoutError:
            # Named as its own outcome rather than folded into a refusal: a
            # caller that cannot tell "the Mac said no" from "the Mac said
            # nothing" will retry a refusal forever.
            raise BridgeError(
                504,
                "no_answer",
                {
                    "deadline_ms": int(REQUEST_TIMEOUT_SECONDS * 1000),
                    "waited_ms": max(0, round((self.monotonic() - started_waiting) * 1000)),
                },
            ) from None
        finally:
            self.waiters.pop(request_id, None)
            self.bridge.resolve(request_id, request.device_id)
        refusal = answer.get("refusal")
        version = {"device_version": answer.get("device_version")}
        if refusal is not None:
            return BridgeReply(
                403,
                {"protocol": RESULT_PROTOCOL, "state": "refused", "refusal": refusal, **version},
            )
        result = answer.get("result", {})
        capture_id = result.get("capture_id") if isinstance(result, dict) else None
        if capture_id is not None:
            body = (
                self.captures.take(request.device_id, capture_id, request_id)
                if isinstance(capture_id, str)
                else None
            )
            if body is None:
                return BridgeReply(
                    403,
                    {
                        "protocol": RESULT_PROTOCOL,
                        "state": "refused",
                        "notice": "Orbit no pudo entregar la captura (capture_missing).",
                        "code": "capture_missing",
                    },
                )
            result = dict(result)
            del result["capture_id"]
            result["data_base64"] = base64.b64encode(body).decode("ascii")
        return BridgeReply(
            200,
            {
                "protocol": RESULT_PROTOCOL,
                "state": "answered",
                "result": result,
                **version,
            },
        )

    async def _start_action(self, request, request_id, runtime, waiter) -> BridgeReply:
        payload = request.model_dump(mode="json")
        self.store.reserve_operation(request.device_id, request_id, payload)
        try:
            await runtime.publish(
                emit(
                    "device.request",
                    {
                        "request_id": request_id,
                        "capability": request.capability,
                        "parameters": request.parameters,
                    },
                )
            )
        except Exception:  # noqa: BLE001 - persist a fixed outcome, never exception text
            report(
                event="device_action_publish_failed",
                code="connection_failed",
                device=request.device_id,
            )
            self.waiters.pop(request_id, None)
            self.bridge.resolve(request_id, request.device_id)
            self.store.finish_operation(
                request.device_id,
                request_id,
                {
                    "outcome": "failed",
                    "code": "connection_failed",
                    "notice": "El Mac no recibió la petición de acción.",
                    "next_step": "Comprueba la conexión de Orbit antes de preparar otra acción.",
                },
            )
        else:
            task = asyncio.create_task(self._finish_action(request.device_id, request_id, waiter))
            self._action_tasks.add(task)
            task.add_done_callback(self._action_tasks.discard)
            self._active_action_operations.add(request_id)
            task.add_done_callback(lambda _task: self._active_action_operations.discard(request_id))
        return BridgeReply(
            202,
            {
                "protocol": RESULT_PROTOCOL,
                "state": "pending",
                "outcome": IN_FLIGHT,
                "operation_id": request_id,
            },
        )

    async def _finish_action(self, device_id: str, operation_id: str, waiter) -> None:
        try:
            answer = await self.wait_for_reply(waiter, ACTION_REPLY_TIMEOUT_SECONDS)
            self.store.mark_device_seen(device_id, self.timestamp() / 1000)
            self.store.finish_operation(
                device_id,
                operation_id,
                {**_action_outcome(answer), "device_version": answer.get("device_version")},
            )
        except asyncio.CancelledError:
            raise
        except TimeoutError:
            self.bridge.expire(self.monotonic(), operation_id)
            expected = "respuesta de perform_action"
            deadline_ms = int(ACTION_REPLY_TIMEOUT_SECONDS * 1000)
            notice = f"El Mac no confirmó la acción en {deadline_ms} ms; el resultado es incierto."
            self.store.finish_operation(
                device_id,
                operation_id,
                {
                    "outcome": "unknown",
                    "code": "device_reply_timeout",
                    "expected": expected,
                    "deadline_ms": deadline_ms,
                    "refusal": render(
                        "device_reply_timeout",
                        notice,
                        numbers={"expected": expected, "deadline_ms": deadline_ms},
                    )["refusal"],
                    "notice": notice,
                    "next_step": "No repitas la acción; revisa su efecto en el Mac.",
                },
            )
        except Exception:  # noqa: BLE001 - operation stays unknown rather than being retried
            report(
                event="device_action_receipt_failed",
                code="receipt_lost",
                device=device_id,
            )
            self.store.finish_operation(
                device_id,
                operation_id,
                {
                    "outcome": "unknown",
                    "code": "receipt_lost",
                    "notice": "La respuesta se perdió después de enviar la acción.",
                    "next_step": "No repitas la acción; revisa su efecto en el Mac.",
                },
            )
        finally:
            self.waiters.pop(operation_id, None)
            self.bridge.resolve(operation_id, device_id)

    def action_result(self, headers, body) -> dict:
        self.check()
        self.action_result_auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        try:
            request = ActionResultRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if self.access is not None:
            self.access.require(request.agent_id, "control")
        operation = self.store.operation(request.device_id, request.operation_id)
        if operation is None:
            raise BridgeError(404, "operation_not_found")
        if (
            operation["status"] == "pending"
            and request.operation_id not in self._active_action_operations
        ):
            response = {
                "outcome": "unknown",
                "code": "receipt_lost",
                "notice": "La respuesta se perdió después de enviar la acción.",
                "next_step": "No repitas la acción; revisa su efecto en el Mac.",
            }
            self.store.finish_operation(request.device_id, request.operation_id, response)
            operation = {"status": "completed", "response": response}
        elif operation["status"] == "pending":
            return {
                "protocol": RESULT_PROTOCOL,
                "state": "answered",
                "operation_id": request.operation_id,
                "outcome": IN_FLIGHT,
            }
        response = operation.get("response")
        if not isinstance(response, dict) or response.get("outcome") not in ACTION_OUTCOMES:
            raise BridgeError(500, "invalid_operation")
        return {
            "protocol": RESULT_PROTOCOL,
            "state": "answered",
            "operation_id": request.operation_id,
            **response,
        }

    def agent_access(self, headers, body) -> dict:
        """The groups of the agent orbit-mcp serves, so it lists only theirs."""
        self.check()
        self.access_auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        try:
            request = AccessRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if self.access is None:
            raise BridgeError(503, "unavailable")
        agent_id, groups = self.access.groups(request.agent_id)
        return {
            "protocol": RESULT_PROTOCOL,
            "state": "answered",
            "agent_id": agent_id,
            "groups": groups,
        }


def _action_outcome(answer: dict) -> dict:
    refusal = answer.get("refusal")
    if refusal is not None:
        if refusal.get("retry", {}).get("kind") == "check_outcome":
            return {
                "outcome": "unknown",
                "code": refusal["code"],
                "refusal": refusal,
                "notice": "El Mac no pudo confirmar el resultado de la acción.",
                "next_step": "No repitas la acción; consulta su resultado y revisa su efecto en el Mac.",
            }
        return {
            "outcome": "failed",
            "code": refusal["code"],
            "refusal": refusal,
            "notice": "El Mac no completó la acción.",
            "next_step": "Consulta la causa antes de preparar otra acción.",
        }
    result = answer.get("result")
    receipt = result.get("receipt") if isinstance(result, dict) else None
    if not isinstance(receipt, dict):
        return {
            "outcome": "unknown",
            "result": result,
            "code": "invalid_action_receipt",
            "notice": "El Mac devolvió un recibo de acción no válido.",
            "next_step": "No repitas la acción; revisa su efecto en el Mac.",
        }
    state = receipt.get("state")
    code = receipt.get("code")
    if state == "performed":
        return {"outcome": "performed", "receipt": receipt, "result": result}
    return {
        "outcome": "failed",
        "code": code if isinstance(code, str) else "action_failed",
        "notice": "El Mac no pudo ejecutar la acción.",
        "next_step": "Revisa Orbit y prepara otra acción sólo si no llegó a enviarse.",
        "receipt": receipt,
        "result": result,
    }


def failure(exc):
    status, code, numbers, detail = exc.status, exc.code, exc.numbers, exc.detail
    message = {
        "unauthorized": "Acceso no autorizado",
        "invalid_request": "Solicitud fuera de contrato",
        "no_answer": "El Mac no respondió dentro del plazo",
        "device_unavailable": "El Mac no está conectado",
        "agent_not_found": "Este hub no tiene ese agente",
    }.get(code, "No se pudo completar la petición")
    if code == "agent_access_disabled" and numbers is not None:
        message = disabled_message(str(numbers.get("agent_id")), str(numbers.get("group")))
    if detail is not None:
        message = f"El parámetro {detail['field']} debe ser {detail['expected']}."
    payload = render(
        code,
        message,
        payload={"protocol": RESULT_PROTOCOL, "state": "unavailable"},
        numbers=numbers,
    )
    if detail is not None:
        payload["refusal"].update(detail)
    return json_response(payload, status)


def mount(app, service, transport_allowed):
    async def request_device(headers, body):
        reply = await service.ask(headers, body)
        payload = reply.payload
        if reply.status >= 400:
            payload = render(
                payload.get("code"),
                payload.get("notice", "El Mac devolvió una negativa tipada."),
                payload=payload,
            )
        return json_response(payload, reply.status)

    def action_result(headers, body):
        payload = service.action_result(headers, body)
        if payload.get("outcome") in {"failed", "unknown", "expired", "rejected"}:
            receipt = payload.get("receipt")
            code = payload.get("code")
            if not isinstance(code, str) and isinstance(receipt, dict):
                code = receipt.get("code")
            if not isinstance(code, str):
                code = None
                payload.pop("code", None)
            fallback = {
                "failed": "La acción falló",
                "unknown": "No se confirmó el resultado; comprueba su efecto en el Mac",
                "expired": "La operación caducó",
                "rejected": "La acción fue rechazada",
            }[payload["outcome"]]
            notice = payload.get("notice")
            message = notice if isinstance(notice, str) and notice else fallback
            payload = render(code, message, payload=payload)
        return payload

    route = partial(private_route, app=app, transport_allowed=transport_allowed, failure=failure)
    route(
        PATH,
        request_device,
        REQUEST_BYTES,
        requests=service.requests,
        methods=["POST", "GET", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"],
    )
    route(
        DEVICES_PATH,
        lambda headers, body: {"devices": service.devices(headers)},
        0,
        methods=["GET"],
    )
    route(ACTION_RESULT_PATH, action_result, NON_ACTION_REQUEST_BYTES, methods=["POST"])
    route(ACCESS_PATH, service.agent_access, NON_ACTION_REQUEST_BYTES, methods=["POST"])


__all__ = ["BridgeConfiguration", "BridgeError", "BridgeService", "mount"]
