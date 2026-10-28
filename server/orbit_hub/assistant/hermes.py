"""Hermes as the hub's assistant and voice. Device credentials never reach Hermes.

The chat goes over Hermes's gateway WebSocket (`/api/ws`, JSON-RPC); this
module calls its methods and names their refusals, and `hermes_events` turns
its events into the typed interface of `backend`. Dictation and read-aloud go
over the same Hermes's HTTP audio endpoints (`hermes_speech`).
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

from websockets.asyncio.client import connect

from ..log import clear_op, report
from ..protocol import HubError
from .backend import (
    Capabilities,
    ClarificationReceipt,
    ConnectionLost,
    ContentBlock,
    EventSink,
    ImageBlock,
    OpenedSession,
    PermissionReceipt,
    ResumedSession,
    TextBlock,
    UnreadableReceipt,
)
from .hermes_credential import credential
from .hermes_events import RESUME_HISTORY_ERROR_PREFIX, HermesEvents
from .hermes_speech import HermesSpeech

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

# What Hermes's `image.attach_bytes` refusals mean (tui_gateway/
# prompt_attachments.py and methods_prompt.py, read 2026-10-22).
HERMES_IMAGE_REFUSALS = {
    4016: "does not support that image type",
    4017: "could not decode it",
    4018: "exceeds its 25 MiB per-image ceiling",
}


# The gateway methods the hub calls; any other is refused before it is sent.
RPC_METHODS = {
    "session.create",
    "session.resume",
    "session.status",
    "prompt.submit",
    # A turn's images, queued on the session before its prompt.submit,
    # and taken back when one of them is refused (tui_gateway's
    # methods_prompt.py; the gateway stages and returns the path).
    "image.attach_bytes",
    "image.detach",
    "session.interrupt",
    "approval.respond",
    "clarify.respond",
}
# Existing RPC receipt budget; it bounds one acknowledgement, not a model turn.
RPC_TIMEOUT_SECONDS = 30


def _rpc_failure(method: str, error: dict) -> HubError:
    """What a Hermes RPC error means, by method and code."""
    code = error.get("code")
    if method == "image.attach_bytes" and code in {4016, 4017, 4018}:
        # 4016 an extension it does not take, 4017 bytes it cannot
        # decode, 4018 over its 25 MiB ceiling.
        return HubError(
            "attachment_refused",
            f"Hermes did not accept a message image (code {code})",
            numbers={"upstream_code": code},
        )
    if method in {"prompt.submit", "image.attach_bytes"} and code == 4001:
        return HubError(
            "session_runtime_not_found",
            "Hermes could not find the active session (code 4001); the message was not accepted and can be resent after resuming",
        )
    if method == "session.resume":
        refusal = _resume_failure(code, error.get("message"))
        if refusal is not None:
            return refusal
    return HubError("upstream_rejected", f"Hermes refused the operation (code {code})")


def _resume_failure(code: object, reason: object) -> HubError | None:
    """Why Hermes refused a `session.resume`, when its code and message say."""
    if code == -32601:
        return HubError(
            "session_resume_unsupported",
            "This Hermes version cannot resume stored sessions (code -32601)",
        )
    if code == 4007 and reason == "session not found":
        return HubError(
            "session_resume_not_found",
            "Hermes no longer retains the stored conversation (code 4007); local history is still available",
        )
    if code == 4007 and reason == "session no longer live; retry resume":
        return HubError(
            "session_resume_race",
            "Hermes closed the session while resuming (code 4007); try again",
        )
    if code == 4130:
        numbers = (
            re.search(r"\((\d+) messages; limit (\d+)\)", reason)
            if isinstance(reason, str)
            else None
        )
        if numbers is not None:
            return HubError(
                "session_resume_too_large",
                f"Hermes cannot resume {numbers[1]} messages; limit {numbers[2]} (code 4130)",
            )
        return HubError(
            "session_resume_too_large",
            "Hermes refused to resume due to size (code 4130); no count or limit was supplied",
        )
    if code == 4009 and reason == "session disconnect interrupt settling":
        return HubError(
            "session_resume_busy",
            "Hermes is still closing the previous turn (code 4009); wait",
        )
    if code == 5000 and isinstance(reason, str):
        if reason.startswith("Session storage is unavailable:"):
            return HubError(
                "session_resume_storage_unavailable",
                "Hermes cannot open its session store (code 5000)",
            )
        if reason.startswith(RESUME_HISTORY_ERROR_PREFIX):
            return HubError(
                "session_resume_history_unreadable",
                "Hermes could not read stored history (code 5000); the message was not sent",
            )
    return None


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
            raise HubError("configuration", "Invalid device name")
        self.backend, self.on_event = backend, on_event
        self.device, self.device_name = device, device_name
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self._ws = None
        self._reader = None
        self._pending: dict[str, asyncio.Future] = {}
        self._ready = asyncio.Event()
        self._closing = False
        self._events = HermesEvents(on_event, device)
        # One `session.resume` at a time on this connection: Hermes's resume
        # path has races of its own (4007, 4009) and was only ever driven one
        # at a time. Only resumes wait for it: turns, controls and the other
        # backends never do.
        self._resuming = asyncio.Lock()

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
            raise HubError("upstream_unavailable", "Could not connect to Hermes") from exc

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
                            "Hermes connection lost; the operation will not be repeated",
                        )
                    )
            self._pending.clear()
            if not self._closing:
                await self.on_event(ConnectionLost())

    async def receive(self, params: dict) -> None:
        """One Hermes event, as the typed event it means; the rest is dropped."""
        await self._events.receive(params)

    async def rpc(self, method: str, params: dict) -> dict:
        if not self.connected:
            raise HubError("upstream_unavailable", "Hermes is not connected")
        if method not in RPC_METHODS:
            raise HubError("method_forbidden", "Hermes method violates the contract")
        request_id = str(uuid4())
        future = asyncio.get_running_loop().create_future()
        self._pending[request_id] = future
        try:
            await self._ws.send(
                json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
            )
            frame = await asyncio.wait_for(future, RPC_TIMEOUT_SECONDS)
            if "error" in frame:
                error = frame["error"] if isinstance(frame["error"], dict) else {}
                if method in {"prompt.submit", "image.attach_bytes"} and error.get("code") == 4001:
                    self.live_sessions.discard(params.get("session_id"))
                raise _rpc_failure(method, error)
            if not isinstance(result := frame.get("result"), dict):
                raise HubError("upstream_protocol", "Invalid Hermes response")
            return result
        except TimeoutError as exc:
            if method == "session.resume":
                raise HubError(
                    "session_resume_timeout",
                    f"Hermes did not confirm resuming; deadline {RPC_TIMEOUT_SECONDS} seconds; the message was not sent",
                ) from exc
            raise HubError(
                "outcome_unknown", "Hermes did not confirm the operation; it will not be repeated"
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
            raise HubError("upstream_protocol", "Hermes omitted the durable identity")
        self.live_sessions.add(result["session_id"])
        return OpenedSession(result["session_id"], result["stored_session_id"])

    async def resume_session(self, stored_id: str) -> ResumedSession:
        params = {
            "profile": self.backend.profile,
            "session_id": stored_id,
            "source": "orbit",
            "close_on_disconnect": False,
            # Bound the WS reply while Hermes hydrates model history.
            # omit_messages=True would load only the tip segment.
            "defer_history": True,
            "omit_messages": False,
        }
        try:
            async with self._resuming:
                result = await self.rpc("session.resume", params)
        except HubError as exc:
            if exc.code == "outcome_unknown":
                raise HubError(
                    "session_resume_connection_lost",
                    "Hermes was lost while resuming; the message was not sent",
                ) from exc
            raise
        runtime_id = result.get("session_id")
        if not isinstance(runtime_id, str) or not runtime_id:
            raise HubError("upstream_protocol", "Hermes omitted the resumed session")
        resumed = result.get("resumed") or result.get("stored_session_id") or stored_id
        if not isinstance(resumed, str) or not resumed:
            raise HubError("upstream_protocol", "Hermes omitted the stored identity")
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
        # Hermes takes a turn as one text, and its images queued on the
        # session beforehand: `prompt.submit` takes them with the turn.
        text = "".join(block.text for block in blocks if isinstance(block, TextBlock))
        staged: list[str] = []
        try:
            for block in blocks:
                if isinstance(block, ImageBlock):
                    staged.append(await self.attach_image(session_id, block))
        except HubError:
            # One refused image: none of the turn's images stays queued for
            # the next turn, and the prompt is not sent.
            for path in staged:
                with suppress(HubError):
                    await self.rpc("image.detach", {"session_id": session_id, "path": path})
            raise
        await self.rpc("prompt.submit", {"session_id": session_id, "text": text})

    async def attach_image(self, session_id: str, block: ImageBlock) -> str:
        """Queue one image on the session; the path Hermes staged it at."""
        extension = block.mime.removeprefix("image/").replace("jpeg", "jpg")
        encoded = base64.b64encode(block.data).decode()
        try:
            result = await self.rpc(
                "image.attach_bytes",
                {
                    "session_id": session_id,
                    "content_base64": f"data:{block.mime};base64,{encoded}",
                    # The extension, not the name: Hermes takes a name's suffix
                    # as the type, and «informe.pdf · página 3» would be refused.
                    "ext": extension,
                },
            )
        except HubError as exc:
            if exc.code != "attachment_refused":
                raise
            code = (exc.numbers or {}).get("upstream_code")
            cause = HERMES_IMAGE_REFUSALS.get(code, "refused it")
            raise HubError(
                "attachment_refused",
                f"Hermes did not accept image «{block.name}» (code {code}): {cause}; the message was not sent",
                numbers={"name": block.name, "upstream_code": code},
            ) from exc
        path = result.get("path")
        if result.get("attached") is not True or not isinstance(path, str) or not path:
            raise HubError(
                "attachment_refused",
                f"Hermes did not confirm image «{block.name}»; the message was not sent",
                numbers={"name": block.name},
            )
        return path

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
                "upstream_protocol",
                "Hermes did not confirm an individual approval; it will not be repeated",
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
                "upstream_protocol",
                "Hermes did not confirm the clarification; it will not be repeated",
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

    # Images go through `image.attach_bytes` before the prompt.
    capabilities = Capabilities(images=True, resume=True, history=True, voice=True)

    def __init__(self, settings: Settings, profile: str):
        self.settings, self.profile = settings, profile

    def ws_url(self, path: str) -> str:
        base = self.settings.hermes_url.rstrip("/")
        base = ("wss" if base.startswith("https:") else "ws") + base[base.index(":") :]
        return base + path + "?" + urlencode({"token": credential(self.settings)})

    def connection(self, device: str, device_name: str, on_event: EventSink) -> HermesConnection:
        return HermesConnection(self, on_event, device, device_name)
