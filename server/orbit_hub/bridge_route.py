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

from .assistant.access import CAPABILITY_GROUP
from .assistant.skills import MAX_NAME_CHARS
from .bridge_outcomes import (
    RESULT_PROTOCOL,
    action_outcome,
    failure,
    publish_failed,
    receipt_lost,
    rendered_action,
    reply_timeout,
)
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
PATH = "/internal/v1/device/request"
ACTION_RESULT_PATH = "/internal/v1/device/action-result"
DEVICES_PATH = "/internal/v1/devices"
# What the agent orbit-mcp serves may use: its access groups.
ACCESS_PATH = "/internal/v1/agent/access"
# One enabled skill of that agent, for orbit-mcp's `orbit_skill`.
SKILL_PATH = "/internal/v1/agent/skill"
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


class SkillRequest(PrivateModel):
    protocol: Literal[PROTOCOL]
    agent_id: str | None = Field(default=None, pattern=AGENT_ID_PATTERN)
    # The standard's bound on a name; any other text is simply not enabled.
    name: str = Field(min_length=1, max_length=MAX_NAME_CHARS)


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
        self.skill_auth = None
        self._action_tasks: set[asyncio.Task] = set()
        self._active_action_operations: set[str] = set()
        self._closed = False

    def start(self):
        if self._closed:
            raise BridgeError(503, "unavailable")
        if self.auth is not None or (self.configuration is None and self.access is None):
            return
        key = (
            read_key(self.configuration.credential_file, BridgeError)
            if self.configuration
            else None
        )

        # One authenticator per route: a signature names its path.
        def authenticator(path):
            return Authenticator(
                self.configuration.key_id if self.configuration else None,
                key,
                protocol=PROTOCOL,
                path=path,
                header_prefix=HEADER_PREFIX,
                failure=BridgeError,
                credentials=self.access.bridge_credential if self.access else None,
            )

        self.auth = authenticator(PATH)
        self.action_result_auth = authenticator(ACTION_RESULT_PATH)
        self.devices_auth = authenticator(DEVICES_PATH)
        self.access_auth = authenticator(ACCESS_PATH)
        self.skill_auth = authenticator(SKILL_PATH)

    def close(self):
        self._closed = True
        self.auth = None
        self.action_result_auth = None
        self.devices_auth = None
        self.access_auth = None
        self.skill_auth = None
        for task in self._action_tasks:
            task.cancel()
        self._action_tasks.clear()
        self._active_action_operations.clear()

    def check(self):
        if (
            self._closed
            or self.auth is None
            or (self.configuration is None and not (self.access and self.access.bridge_available()))
        ):
            raise BridgeError(503, "unavailable")

    def devices(self, headers) -> list[dict]:
        self.check()
        self.devices_auth.admit(headers, b"", int(self.timestamp()), self.monotonic())
        result = []
        for row in self.store.devices.active():
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
        identity = self.auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        request = self._request(body)
        if identity is not None:
            request = request.model_copy(update={"agent_id": identity})
        group = CAPABILITY_GROUP.get(request.capability)
        if self.access is not None and group is not None:
            self.access.require(request.agent_id, group)
        runtime = self.runtimes.get(request.device_id)
        # Unknown and disconnected are answered the same on purpose: holding the
        # key does not entitle a caller to enumerate this hub's devices.
        if runtime is None or runtime.revoked or not runtime.listeners:
            raise BridgeError(404, "device_unavailable")
        request_id = str(uuid.uuid4())
        waiter: asyncio.Future = asyncio.get_running_loop().create_future()
        try:
            self.bridge.open(request_id, request.device_id, request.capability, self.monotonic())
        except BridgeRefused as refusal:
            raise BridgeError(429, refusal.code, refusal.numbers) from None
        self.waiters[request_id] = waiter
        if request.capability == "perform_action":
            return await self._start_action(request, request_id, runtime, waiter)
        answer = await self._question(request, request_id, runtime, waiter)
        return self._answered(request, request_id, answer)

    def _request(self, body) -> BridgeRequest:
        """The request's envelope, refused with the field that is wrong."""
        try:
            request = BridgeRequest.model_validate_json(body)
        except ValidationError as error:
            first = error.errors(include_input=False, include_context=False)[0]
            field = ".".join(str(part) for part in first["loc"])
            expected = "objeto" if first["type"] == "dict_type" else "valid envelope field"
            raise BridgeError(
                400, "invalid_request", detail={"field": field, "expected": expected}
            ) from None
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if request.capability != "perform_action" and len(body) > NON_ACTION_REQUEST_BYTES:
            raise BridgeError(
                400, "invalid_request", {"bytes": len(body), "max_bytes": NON_ACTION_REQUEST_BYTES}
            )
        return request

    def _device_frame(self, request, request_id):
        # The authenticated id has already replaced the caller's claim. Take
        # the name from the hub registry, including the operator's default.
        agent = self.access.agent(request.agent_id) if self.access is not None else None
        return emit(
            "device.request",
            {
                "request_id": request_id,
                "capability": request.capability,
                "parameters": request.parameters,
                **({"agent": {"id": agent.id, "name": agent.name}} if agent else {}),
            },
        )

    async def _question(self, request, request_id, runtime, waiter) -> dict:
        """Put the question to the Mac and wait, bounded, for its answer."""
        started_waiting = self.monotonic()
        try:
            await runtime.publish(self._device_frame(request, request_id))
            answer = await asyncio.wait_for(waiter, REQUEST_TIMEOUT_SECONDS)
            # A real reply is stronger activity evidence than an open socket.
            # Keep the durable inventory current for both answers and refusals.
            self.store.devices.mark_seen(request.device_id, self.timestamp() / 1000)
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
        return answer

    def _answered(self, request, request_id, answer: dict) -> BridgeReply:
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
                        "notice": "Orbit could not deliver the capture (capture_missing).",
                        "code": "capture_missing",
                    },
                )
            result = dict(result)
            del result["capture_id"]
            result["data_base64"] = base64.b64encode(body).decode("ascii")
        answered = {"protocol": RESULT_PROTOCOL, "state": "answered", "result": result}
        return BridgeReply(200, {**answered, **version})

    async def _start_action(self, request, request_id, runtime, waiter) -> BridgeReply:
        payload = request.model_dump(mode="json")
        self.store.operations.reserve(request.device_id, request_id, payload)
        try:
            await runtime.publish(self._device_frame(request, request_id))
        except Exception:  # noqa: BLE001 - persist a fixed outcome, never exception text
            report(
                event="device_action_publish_failed",
                code="connection_failed",
                device=request.device_id,
            )
            self.waiters.pop(request_id, None)
            self.bridge.resolve(request_id, request.device_id)
            self.store.operations.finish(request.device_id, request_id, publish_failed())
        else:
            task = asyncio.create_task(self._finish_action(request.device_id, request_id, waiter))
            self._action_tasks.add(task)
            task.add_done_callback(self._action_tasks.discard)
            self._active_action_operations.add(request_id)
            task.add_done_callback(lambda _task: self._active_action_operations.discard(request_id))
            # Wait for the journal writer, not just the reply: both entry points
            # must expose the same durable result. asyncio.wait leaves the task
            # running at timeout or caller cancellation; it never repeats input.
            await asyncio.wait({task}, timeout=self.configuration.action_inline_wait_seconds)
        operation = self.store.operations.get(request.device_id, request_id)
        if operation is not None and operation["status"] == "completed":
            return BridgeReply(
                200,
                {
                    "protocol": RESULT_PROTOCOL,
                    "state": "answered",
                    "operation_id": request_id,
                    **operation["response"],
                },
            )
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
            self.store.devices.mark_seen(device_id, self.timestamp() / 1000)
            self.store.operations.finish(
                device_id,
                operation_id,
                {**action_outcome(answer), "device_version": answer.get("device_version")},
            )
        except asyncio.CancelledError:
            raise
        except TimeoutError:
            self.bridge.expire(self.monotonic(), operation_id)
            deadline_ms = int(ACTION_REPLY_TIMEOUT_SECONDS * 1000)
            self.store.operations.finish(device_id, operation_id, reply_timeout(deadline_ms))
        except Exception:  # noqa: BLE001 - operation stays unknown rather than being retried
            report(event="device_action_receipt_failed", code="receipt_lost", device=device_id)
            self.store.operations.finish(device_id, operation_id, receipt_lost())
        finally:
            self.waiters.pop(operation_id, None)
            self.bridge.resolve(operation_id, device_id)

    def action_result(self, headers, body) -> dict:
        self.check()
        identity = self.action_result_auth.admit(
            headers, body, int(self.timestamp()), self.monotonic()
        )
        try:
            request = ActionResultRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if identity is not None:
            request = request.model_copy(update={"agent_id": identity})
        if self.access is not None:
            self.access.require(request.agent_id, "control")
        operation = self.store.operations.get(request.device_id, request.operation_id)
        if operation is None:
            raise BridgeError(404, "operation_not_found")
        if (
            operation["status"] == "pending"
            and request.operation_id not in self._active_action_operations
        ):
            response = receipt_lost()
            self.store.operations.finish(request.device_id, request.operation_id, response)
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
        identity = self.access_auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        try:
            request = AccessRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if identity is not None:
            request = request.model_copy(update={"agent_id": identity})
        if self.access is None:
            raise BridgeError(503, "unavailable")
        agent = self.access.agent(request.agent_id)
        _agent_id, groups = self.access.groups(agent.id)
        return {
            "protocol": RESULT_PROTOCOL,
            "state": "answered",
            "agent_id": agent.id,
            "groups": groups,
            # False for a Hermes agent: orbit-mcp then offers no `orbit_skill`.
            "skills_in_hub": agent.skills_in_hub,
        }

    def agent_skill(self, headers, body) -> dict:
        """One enabled skill's instructions for the agent orbit-mcp serves."""
        self.check()
        identity = self.skill_auth.admit(headers, body, int(self.timestamp()), self.monotonic())
        try:
            request = SkillRequest.model_validate_json(body)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise BridgeError(400, "invalid_request") from None
        if identity is not None:
            request = request.model_copy(update={"agent_id": identity})
        if self.access is None:
            raise BridgeError(503, "unavailable")
        agent_id, skill = self.access.skill(request.agent_id, request.name)
        return {
            "protocol": RESULT_PROTOCOL,
            "state": "answered",
            "agent_id": agent_id,
            "name": skill.name,
            "instructions": skill.body,
        }


def mount(app, service, transport_allowed):
    async def request_device(headers, body):
        reply = await service.ask(headers, body)
        payload = reply.payload
        if payload.get("state") == "answered" and "operation_id" in payload:
            payload = rendered_action(payload)
        if reply.status >= 400:
            payload = render(
                payload.get("code"),
                payload.get("notice", "The Mac returned a typed refusal."),
                payload=payload,
            )
        return json_response(payload, reply.status)

    def action_result(headers, body):
        return rendered_action(service.action_result(headers, body))

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
    route(SKILL_PATH, service.agent_skill, NON_ACTION_REQUEST_BYTES, methods=["POST"])


__all__ = ["BridgeConfiguration", "BridgeError", "BridgeService", "mount"]
