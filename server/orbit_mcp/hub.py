"""Signed, bounded HTTP client for the private Orbit hub routes."""

from __future__ import annotations

import json
import time
from uuid import uuid4

import httpx
from pydantic import ValidationError

from orbit_hub.bridge_route import ACTION_RESULT_PATH, DEVICES_PATH
from orbit_hub.bridge_route import IN_FLIGHT as IN_FLIGHT_OUTCOME
from orbit_hub.bridge_route import PATH as BRIDGE_PATH
from orbit_hub.bridge_route import PROTOCOL as BRIDGE_PROTOCOL
from orbit_hub.log import current_op, get_logger, new_op
from orbit_hub.meetings_bridge import LIST_PATH, NOTE_PATH, TRANSCRIPT_PATH
from orbit_hub.meetings_bridge import PROTOCOL as MEETINGS_PROTOCOL
from orbit_hub.private_auth import canonical, sign_request

from . import calendar as agenda
from .bridge import (
    MAX_BRIDGE_DETAIL_CHARS,
    MAX_TOOL_NAME_BYTES,
    PARAMETER_MODELS,
    RESULT_MODELS,
    BridgeRefusal,
    ScreenshotContentResult,
    validation_detail,
)
from .config import Config, Credentials

# The hub waiter ends at 30 s; five more seconds let its named refusal return.
# This is the translator's HTTP wait, not a retry of an input action.
BRIDGE_TIMEOUT_SECONDS = 35.0
# The device listing and an action's outcome are answered by the hub itself,
# without a round trip to the Mac, so they get the shorter wait.
ROUTE_TIMEOUT_SECONDS = 10.0
MEETINGS_TIMEOUT_SECONDS = 10.0
DEVICES_CACHE_SECONDS = 10.0
# The most pages one catalogue may take. A Mac's catalogue is bounded to
# 256 KiB and a page carries up to 96 KiB, so three would do; 64 leaves room
# for a Mac whose pages are small without ever letting the walk run forever.
CATALOGUE_PAGES = 64

# An action the hub has handed to the Mac is in flight, and that is the only
# word for it. The tolerance for the approval word an older hub used is gone,
# because the hub and this translator ship and restart together from one
# version folder; any other word is an invalid response and is never repeated
# to the assistant. The user granted Orbit the use of their computer in macOS;
# nothing downstream of that is waiting on them.
IN_FLIGHT_OUTCOMES = frozenset({IN_FLIGHT_OUTCOME})
IN_FLIGHT_NOTICE = "La acción ya está en curso en el Mac."
IN_FLIGHT_NEXT_STEP = "Consulta su desenlace con orbit_action_result y este operation_id."
# These are journal reads, including receipts saved before 0.6, not states a
# current Mac may emit. Removing old values would make durable receipts unreadable.
SETTLED_OUTCOMES = frozenset({"performed", "unknown", "failed", "expired", "rejected"})

# How much of the Mac's own sentence travels back to whoever asked.
# Long enough for any refusal the Mac actually writes — the longest one today is
# under 200 characters — and short enough that a Mac answering with something
# else cannot use a refusal as a channel.
MAC_NOTICE_CHARS = MAX_BRIDGE_DETAIL_CHARS

logger = get_logger("orbit_hub.orbit_mcp")


def _in_flight(operation_id: str | None = None) -> dict:
    """What an action still under way is called, and where to read its end.

    Never the word approval: saying so made the assistant tell the user a page
    had been left pending their approval when it was already open on their screen.
    """
    answer: dict = {
        "outcome": IN_FLIGHT_OUTCOME,
        "notice": IN_FLIGHT_NOTICE,
        "next_step": IN_FLIGHT_NEXT_STEP,
    }
    if operation_id is not None:
        answer["operation_id"] = operation_id
    return answer


class HubClientError(RuntimeError):
    def __init__(self, code: str, status: int | None = None, detail: dict | None = None):
        self.code = code
        self.status = status
        # Numbers the hub measured for this one refusal, kept apart from the
        # fixed message so the tool decides what is safe to say out of them.
        # The same channel carries `mac_notice`: the one
        # sentence the Mac wrote about this refusal, already bounded.
        self.detail = detail if isinstance(detail, dict) else {}
        super().__init__(code)


def _walk_failure(exc: agenda.WalkError) -> HubClientError:
    """A page that could not be joined, with the translator's own sentence on why.

    The sentence is built from numbers in ``calendar.py``, never from agenda
    content, and travels in the same ``walk_notice`` channel as the restart count.
    """
    return HubClientError(exc.code, detail={"walk_notice": exc.notice} if exc.notice else None)


def _failure(payload: dict, status: int) -> HubClientError:
    code = payload.get("code")
    refusal = payload.get("refusal")
    if (
        not isinstance(code, str)
        and isinstance(refusal, dict)
        and isinstance(refusal.get("code"), str)
    ):
        code = refusal["code"]
    elif not isinstance(code, str):
        code = "hub_response_without_code"
    detail = {}
    if isinstance(refusal, dict):
        detail["refusal"] = refusal
    version = payload.get("device_version")
    if isinstance(version, str):
        detail["device_version"] = version
    return HubClientError(code, status, detail)


def validate_parameters(capability, parameters):
    try:
        PARAMETER_MODELS[capability].model_validate(parameters)
    except KeyError:
        raise HubClientError("unknown_capability") from None
    except ValidationError as error:
        details = validation_detail(error, "")
        if (
            capability in {"screenshot", "selected_text"}
            and error.errors()[0]["type"] == "extra_forbidden"
        ):
            details["expected"] = (
                "application, title y full_screen"
                if capability == "screenshot"
                else "application y title"
            )
        if capability == "perform_action":
            action = parameters.get("action") if isinstance(parameters, dict) else parameters
            details["received_keys"] = (
                json.dumps(sorted(action), ensure_ascii=False) if isinstance(action, dict) else "[]"
            )
            details["received_type"] = type(action).__name__
            if (
                details["field"] == "action.tool"
                and isinstance(action, dict)
                and isinstance(action.get("tool"), str)
            ):
                details.update(
                    bytes=len(action["tool"].encode("utf-8")), max_bytes=MAX_TOOL_NAME_BYTES
                )
        raise HubClientError(
            "invalid_parameters",
            detail={
                "refusal": {"code": "invalid_parameters", **details, "retry": {"kind": "after_fix"}}
            },
        ) from None
    return parameters


def validate_mac_payload(capability, payload):
    """Validate at the consumer, retaining every byte of the relayed JSON values."""
    prefix = (
        "refusal"
        if payload.get("state") == "refused" or ("refusal" in payload and "result" not in payload)
        else "result"
    )
    try:
        if prefix == "refusal":
            BridgeRefusal.model_validate(payload["refusal"])
        else:
            model = (
                ScreenshotContentResult if capability == "screenshot" else RESULT_MODELS[capability]
            )
            model.model_validate(payload.get("result"))
    except ValidationError as error:
        details = validation_detail(error, prefix)
        details["device_version"] = payload.get("device_version") or "no comunicada"
        raise HubClientError(
            "device_reply_invalid",
            detail={
                "device_version": details["device_version"],
                "refusal": {
                    "code": "device_reply_invalid",
                    **details,
                    "retry": {
                        "kind": "check_outcome" if capability == "perform_action" else "after_fix"
                    },
                },
            },
        ) from None


class HubClient:
    def __init__(self, config: Config, *, client_factory=None, clock=None):
        self.config = config
        self._client_factory = client_factory or httpx.AsyncClient
        self._clock = clock or (lambda: int(time.time() * 1000))
        self._devices_cache: tuple[float, list[dict]] | None = None

    def update_config(self, config: Config) -> None:
        if (
            config.hub_url != self.config.hub_url
            or config.device_bridge != self.config.device_bridge
        ):
            self._devices_cache = None
        self.config = config

    async def devices(self, *, fresh: bool = False) -> list[dict]:
        now = time.monotonic()
        if not fresh and self._devices_cache is not None:
            cached_at, cached = self._devices_cache
            if now - cached_at < DEVICES_CACHE_SECONDS:
                return [dict(row) for row in cached]
        response = await self._request(
            "GET",
            DEVICES_PATH,
            b"",
            self.config.device_bridge,
            BRIDGE_PROTOCOL,
            ROUTE_TIMEOUT_SECONDS,
            None,
        )
        payload = self._json_payload(response)
        rows = payload.get("devices")
        if response.status_code >= 400:
            raise _failure(payload, response.status_code)
        if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
            raise HubClientError("invalid_response", response.status_code)
        cached = [dict(row) for row in rows]
        self._devices_cache = (now, cached)
        return [dict(row) for row in cached]

    async def device_request(self, device_id: str, capability: str, parameters: dict) -> dict:
        validate_parameters(capability, parameters)
        body = canonical(
            {
                "protocol": BRIDGE_PROTOCOL,
                "device_id": device_id,
                "capability": capability,
                "parameters": parameters,
            }
        )
        response = await self._post(
            BRIDGE_PATH,
            body,
            self.config.device_bridge,
            BRIDGE_PROTOCOL,
            BRIDGE_TIMEOUT_SECONDS,
            device_id,
        )
        payload = self._json_payload(response)
        if response.status_code < 400 and payload.get("state") == "pending":
            operation_id = payload.get("operation_id")
            if (
                capability != "perform_action"
                or not isinstance(operation_id, str)
                or payload.get("outcome") not in IN_FLIGHT_OUTCOMES
            ):
                raise HubClientError("invalid_response", response.status_code)
            # «Pending» means the Mac has it, not that anyone is being asked.
            return _in_flight(operation_id)
        if payload.get("state") == "refused":
            # Old Macs' refused+code replies are deliberately no longer accepted.
            if not isinstance(payload.get("refusal"), dict):
                raise HubClientError("invalid_response", response.status_code)
            validate_mac_payload(capability, payload)
        if response.status_code >= 400 or payload.get("state") != "answered":
            # The Mac explains its own refusals in a sentence and
            # this is where that sentence used to die, leaving the caller the
            # code and the fixed table entry — which is how `orbit_calendar`
            # spent its first day telling the user their parameters were wrong
            # without ever saying which one or why. Every capability's refusal
            # comes through here, so every one of them now carries its cause.
            failure = _failure(payload, response.status_code)
            failure.detail["capability"] = capability
            raise failure
        validate_mac_payload(capability, payload)
        return payload["result"]

    async def control_tools(self, device_id: str) -> list[dict]:
        """The engine's whole catalogue, walked page by page.

        The Mac answers `control_tools` one page at a time because the whole
        catalogue does not fit a hub frame. The walk lives here rather than in
        the tool so that Deimos still asks once and gets everything: the extra
        round trips are between this process and the Mac, never between Deimos
        and this process.

        Every page is validated as `ControlToolsResult` first, so its counters
        are there; a catalogue that fits one page comes without a continuation
        and the walk ends on it.
        """
        after_index = 0
        tools: list[dict] = []
        total = None
        for _ in range(CATALOGUE_PAGES):
            parameters = {"after_index": after_index} if after_index else {}
            result = await self.device_request(device_id, "control_tools", parameters)
            declared = result["total"]
            if result["after_index"] != after_index or declared < 0:
                raise HubClientError("invalid_response")
            # A catalogue that changed size mid-walk would assemble into a
            # list that never existed on the Mac. Say so rather than hand
            # Deimos a mixture of two engines.
            if total is not None and declared != total:
                raise HubClientError("catalogue_changed")
            total = declared
            tools.extend(result["tools"])
            following = result.get("next_after_index")
            if following is None:
                if len(tools) != total:
                    raise HubClientError("invalid_response")
                return tools
            if type(following) is not int or following <= after_index:
                raise HubClientError("invalid_response")
            after_index = following
        raise HubClientError("invalid_response")

    async def action_result(self, device_id: str, operation_id: str) -> dict:
        body = canonical(
            {
                "protocol": BRIDGE_PROTOCOL,
                "device_id": device_id,
                "operation_id": operation_id,
            }
        )
        response = await self._post(
            ACTION_RESULT_PATH,
            body,
            self.config.device_bridge,
            BRIDGE_PROTOCOL,
            ROUTE_TIMEOUT_SECONDS,
            device_id,
        )
        payload = self._json_payload(response)
        if response.status_code >= 400 or payload.get("state") != "answered":
            raise _failure(payload, response.status_code)
        if "result" in payload or "refusal" in payload:
            validate_mac_payload("perform_action", payload)
        outcome = payload.get("outcome")
        if outcome in IN_FLIGHT_OUTCOMES:
            # Still running. Say that, and say where to look again.
            return _in_flight()
        if outcome not in SETTLED_OUTCOMES:
            raise HubClientError("invalid_response", response.status_code)
        return {
            key: value
            for key, value in payload.items()
            if key not in {"protocol", "state", "operation_id"}
        }

    async def calendar_read(self, device_id: str, start_ms: int, end_ms: int) -> dict:
        """The user's whole agenda for ``[start_ms, end_ms)``, however many pages it takes.

        Deimos asks once. The Mac answers ``calendar_read`` a page at
        a time and says where the next one starts; the walk lives here, like the
        catalogue's, so the extra round trips are between this process and the
        Mac and never between Deimos and this process.

        When the Mac says the read it was paging was lost because its reader
        restarted (``calendar_read_restarted``) or macOS cut capture without a
        privacy stop (``capture_stopped``), the walk starts again from the
        beginning, up to ``agenda.RESTARTS`` times, and then says how many
        times it tried.
        """
        attempts = 0
        while True:
            try:
                return await self._calendar_walk(device_id, start_ms, end_ms)
            except HubClientError as exc:
                reason = None
                refusal = exc.detail.get("refusal")
                if exc.code == "read_interrupted" and isinstance(refusal, dict):
                    cause = refusal.get("cause")
                    reason = cause.get("kind") if isinstance(cause, dict) else None
                    if reason == "helper_restarted":
                        reason = "calendar_read_restarted"
                if reason not in {"calendar_read_restarted", "capture_stopped"}:
                    raise
                attempts += 1
                if attempts > agenda.RESTARTS:
                    raise HubClientError(
                        exc.code,
                        exc.status,
                        {
                            **exc.detail,
                            "walk_notice": (
                                f"Orbit empezó la lectura {attempts} veces y el Mac la perdió "
                                "las mismas veces antes de terminarla."
                            ),
                        },
                    ) from None

    async def _calendar_walk(self, device_id: str, start_ms: int, end_ms: int) -> dict:
        reading = agenda.Reading(start_ms, end_ms)
        parameters: dict | None = {"start_ms": start_ms, "end_ms": end_ms}
        while parameters is not None:
            result = await self.device_request(device_id, "calendar_read", parameters)
            try:
                parameters = agenda.next_request(reading, parameters, result)
            except agenda.WalkError as exc:
                raise _walk_failure(exc) from None
        try:
            return reading.answer()
        except agenda.WalkError as exc:
            raise _walk_failure(exc) from None

    async def meetings(
        self, device_id: str, since_ms: int, limit: int, before_ms: int | None = None
    ) -> dict:
        """One page of the device's meetings, newest first, and whether more remain."""
        request = {"device_id": device_id, "since_ms": since_ms, "limit": limit}
        if before_ms is not None:
            request["before_ms"] = before_ms
        payload = await self._meetings_call(LIST_PATH, request, device_id)
        meetings = payload.get("meetings")
        more = payload.get("more")
        if not isinstance(meetings, list) or not isinstance(more, bool):
            raise HubClientError("invalid_response")
        return {"meetings": meetings, "more": more}

    async def transcript(
        self, device_id: str, recording_id: str, format: str, after_index: int = 0
    ) -> dict:
        """The hub's page of the transcript that starts at `after_index`.

        The hub pages at 1 MiB; what reaches Deimos is cut much shorter by the
        tool, so one hub page is always enough for one answer.
        """
        request = {"device_id": device_id, "recording_id": recording_id, "format": format}
        if after_index:
            request["after_index"] = after_index
        payload = await self._meetings_call(TRANSCRIPT_PATH, request, device_id)
        text = payload.get("text")
        total = payload.get("total_chars")
        following = payload.get("next_after_index")
        groups = payload.get("incomplete_groups", [])
        if (
            not isinstance(text, str)
            or payload.get("after_index") != after_index
            or (following is not None and (type(following) is not int or following <= after_index))
            or (total is not None and type(total) is not int)
            or not isinstance(groups, list)
            or payload.get("capture", "closed") not in {"open", "closed"}
        ):
            raise HubClientError("invalid_response")
        for group in groups:
            if (
                not isinstance(group, dict)
                or not isinstance(group.get("track"), str)
                or not group["track"]
                or type(group.get("first_index")) is not int
                or type(group.get("last_index")) is not int
                or not isinstance(group.get("cause"), str)
                or not group["cause"]
                or type(group.get("attempts")) is not int
            ):
                raise HubClientError("invalid_response")
        return payload

    async def note(self, device_id: str, recording_id: str) -> dict:
        """The note as the user sees it; a note not written yet is the hub's refusal."""
        payload = await self._meetings_call(
            NOTE_PATH, {"device_id": device_id, "recording_id": recording_id}, device_id
        )
        note = payload.get("note")
        if (
            not isinstance(note, dict)
            or not isinstance(payload.get("state"), str)
            or not isinstance(payload.get("private_notes"), str)
            or not isinstance(payload.get("edited"), dict)
        ):
            raise HubClientError("invalid_response")
        return payload

    async def _meetings_call(self, path: str, request: dict, device_id: str) -> dict:
        credentials = self.config.meetings
        if credentials is None:
            raise HubClientError("meetings_not_configured")
        response = await self._post(
            path,
            canonical(request),
            credentials,
            MEETINGS_PROTOCOL,
            MEETINGS_TIMEOUT_SECONDS,
            device_id,
        )
        payload = self._json_payload(response)
        if response.status_code >= 400:
            raise _failure(payload, response.status_code)
        return payload

    async def _post(
        self,
        path: str,
        body: bytes,
        credentials: Credentials,
        protocol: str,
        timeout: float,
        device: str | None,
    ) -> httpx.Response:
        return await self._request("POST", path, body, credentials, protocol, timeout, device)

    async def _request(
        self,
        method: str,
        path: str,
        body: bytes,
        credentials: Credentials,
        protocol: str,
        timeout: float,
        device: str | None,
    ) -> httpx.Response:
        started = time.monotonic()
        code = "answered"
        try:
            issued_at_ms = self._clock()
            if type(issued_at_ms) is not int:
                raise HubClientError("client_clock")
            headers = sign_request(
                protocol,
                path,
                credentials.key_id,
                credentials.key,
                body,
                issued_at_ms=issued_at_ms,
                nonce=str(uuid4()),
            )
            headers["content-type"] = "application/json"
            headers["X-Orbit-Op"] = current_op() or new_op()
            async with self._client_factory(timeout=timeout) as client:
                url = f"{self.config.hub_url}{path}"
                if method == "GET":
                    response = await client.get(url, headers=headers)
                else:
                    response = await client.post(url, content=body, headers=headers)
            if response.status_code >= 400:
                code = f"http_{response.status_code}"
                try:
                    error_payload = response.json()
                except (ValueError, UnicodeError):
                    error_payload = None
                if isinstance(error_payload, dict):
                    response_code = error_payload.get("code")
                    if isinstance(response_code, str):
                        code = response_code
            return response
        except httpx.TimeoutException:
            code = "no_answer"
            raise HubClientError(code) from None
        except httpx.HTTPError:
            code = "hub_unavailable"
            raise HubClientError(code) from None
        except HubClientError as exc:
            code = exc.code
            raise
        finally:
            logger.info(
                "orbit_mcp_request",
                extra={
                    "event": "orbit_mcp_request",
                    "code": code,
                    "device": device,
                    "elapsed_ms": max(0, int((time.monotonic() - started) * 1000)),
                },
            )

    @staticmethod
    def _json_payload(response: httpx.Response) -> dict:
        try:
            payload = response.json()
        except (ValueError, UnicodeError):
            raise HubClientError("invalid_response", response.status_code) from None
        if not isinstance(payload, dict):
            raise HubClientError("invalid_response", response.status_code)
        return payload
