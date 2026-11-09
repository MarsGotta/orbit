"""The hub's real Hermes connection and device runtime, in front of a `ContractPeer`.

What the Mac would receive is read from the runtime's own stream of frames
(`DeviceRuntime.subscribe`); what Hermes would receive, from the peer.
"""

from __future__ import annotations

import asyncio
import os
from contextlib import AsyncExitStack
from dataclasses import replace
from uuid import uuid4

from conftest import device, only
from support_hermes_requests import ContractPeer
from websockets.asyncio.server import serve

from orbit_hub.assistant.hermes import HermesBackend
from orbit_hub.protocol import ApprovalParams, ClarifyParams, SendParams
from orbit_hub.runtime import DeviceRuntime

# Waits for something the hub does next; never a measurement.
WAIT_SECONDS = 10


class HubOnPeer:
    def __init__(self, environment, tmp_path, peer: ContractPeer):
        self.settings, self.store, _ = environment
        self.tmp_path = tmp_path
        self.peer = peer
        self.owner = device(self.store)["device_id"]
        self.runtime: DeviceRuntime | None = None
        self.turns: dict[str, str] = {}
        # Every frame the Mac got, and those no wait has taken yet.
        self.seen: list[dict] = []
        self._unclaimed: list[dict] = []
        self._stack = AsyncExitStack()

    async def start(self) -> HubOnPeer:
        server = await self._stack.enter_async_context(serve(self.peer.serve, "127.0.0.1", 0))
        port = server.sockets[0].getsockname()[1]
        credential = self.tmp_path / f"synthetic-credential-{uuid4().hex[:8]}"
        descriptor = os.open(credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(descriptor, b"synthetic-token-only")
        finally:
            os.close(descriptor)
        settings = replace(
            self.settings,
            hermes_url=f"http://127.0.0.1:{port}",
            allow_insecure_localhost=True,
            hermes_credential_file=credential,
        )
        backend = HermesBackend(settings, settings.profile)
        self.runtime = DeviceRuntime(self.owner, self.store, only(backend), lambda: 1001)
        self.queue = self.runtime.subscribe()
        return self

    async def close(self) -> None:
        if self.runtime is not None:
            await self.runtime.close()
        await self._stack.aclose()

    # The Mac's side -----------------------------------------------------------

    async def send(self, text: str, session_id: str | None = None) -> str:
        """A turn the Mac sends; its conversation's id."""
        turn = str(uuid4())
        _, data = await self.runtime.handle(
            "chat.send", SendParams(text=text, session_id=session_id), turn
        )
        self.turns[data["session_id"]] = turn
        return data["session_id"]

    def live(self, session_id: str) -> str:
        """The Hermes session the conversation is bound to."""
        return self.store.chat.session(self.owner, session_id)["runtime_id"]

    async def approve(self, session_id: str, request_id: str, choice: str):
        params = ApprovalParams(
            session_id=session_id,
            request_id=request_id,
            approved=choice != "deny",
            turn_id=self.turns[session_id],
            choice=choice,
        )
        return await self.runtime.handle("approval.respond", params, str(uuid4()))

    async def clarify(self, session_id: str, request_id: str, answer: str, question_id=None):
        params = ClarifyParams(
            session_id=session_id, request_id=request_id, answer=answer, question_id=question_id
        )
        return await self.runtime.handle("clarify.respond", params, str(uuid4()))

    async def frame(self, event: str, where=lambda data: True, timeout=WAIT_SECONDS) -> dict:
        """The next frame `event` the Mac gets whose data matches `where`."""
        for index, frame in enumerate(self._unclaimed):
            if frame["event"] == event and where(frame["data"]):
                return self._unclaimed.pop(index)
        try:
            async with asyncio.timeout(timeout):
                while True:
                    frame = await self.queue.get()
                    if frame is None:
                        raise AssertionError("the hub closed the Mac's stream")
                    self.seen.append(frame)
                    if frame["event"] == event and where(frame["data"]):
                        return frame
                    self._unclaimed.append(frame)
        except TimeoutError as error:
            got = [frame["event"] for frame in self.seen]
            raise AssertionError(
                f"the Mac never got {event} within {timeout} s; it got {got}"
            ) from error

    def warnings(self, session_id: str) -> list[str]:
        """The `chat.status` warnings of the conversation the Mac got so far."""
        while not self.queue.empty():
            frame = self.queue.get_nowait()
            if frame is not None:
                self.seen.append(frame)
                self._unclaimed.append(frame)
        return [
            frame["data"]["text"]
            for frame in self.seen
            if frame["event"] == "chat.status"
            and frame["data"]["session_id"] == session_id
            and frame["data"]["kind"] == "warning"
        ]
