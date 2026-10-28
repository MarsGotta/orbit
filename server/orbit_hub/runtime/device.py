"""One paired device's conversations with the hub's agents.

`DeviceRuntime` is what the socket, the device bridge and the routes hold for
a device: its connections to each agent, the live sessions they carry, the
turns in flight and their pending questions, and the frames its Macs read.
The work is the pieces': each conversation's locks (`conversation`), a
turn's send (`send`), a conversation's controls (`controls`), the agent's
events (`events`) and the reopening of a conversation (`resume`).
"""

import asyncio
import json
from collections.abc import Callable
from concurrent.futures import Executor
from functools import partial
from uuid import uuid4

from ..assistant.agents import Agents
from ..assistant.backend import (
    AssistantConnection,
    AssistantEvent,
    ClarificationRequest,
    PermissionRequest,
    TurnFailed,
)
from ..interactions import pending_data
from ..log import current_op, in_store, logger, report
from ..protocol import HubError, emit
from ..safety import target_key
from ..storage import Store
from ..turns import TurnState, advance
from .controls import Controls
from .conversation import Conversations
from .events import TurnEvents
from .inputs import TurnInputTracker
from .observation import observe
from .resume import LiveKey, Resumes
from .send import TurnSender
from .work import TurnWork

Pending = tuple[str, str, PermissionRequest | ClarificationRequest]


class DeviceRuntime:
    def __init__(
        self,
        device: str,
        store: Store,
        agents: Agents,
        now: Callable[[], float],
        *,
        store_executor: Executor | None = None,
        input_tracker: TurnInputTracker | None = None,
    ):
        self.device, self.store, self.agents, self.now = device, store, agents, now
        self.device_name = self._device_name()
        self.input_tracker = input_tracker or TurnInputTracker(now)
        self.store_executor = store_executor
        # One connection per agent this device has talked to, by agent id, and
        # the opening of each: two turns for one agent wait for the same one.
        self.connections: dict[str, AssistantConnection] = {}
        self._opening: dict[str, asyncio.Lock] = {}
        # Live sessions to the Orbit sessions they carry.
        self.bindings: dict[LiveKey, str] = {}
        self.resumed_message_counts: dict[LiveKey, int | None] = {}
        self.turn_ids: dict[str, str] = {}
        # What each conversation's turn in flight did, for its answer (`work`).
        self.work: dict[str, TurnWork] = {}
        # The `chat.send` ids whose message this hub stored, until the socket
        # that sent them has answered (`ws_dispatch`): a send refused before
        # this point kept nothing and gives its id back.
        self._committed: set[str] = set()
        self.interrupt_deadlines: dict[str, float] = {}
        # Approvals and clarifications waiting for the Mac, by the request id
        # the Mac sees: (Orbit session, kind, the backend's request). Unique on
        # this device; see `request_key`.
        self.pending: dict[str, Pending] = {}
        self.listeners: set[asyncio.Queue] = set()
        self.revoked = False
        self.closed = False
        self.revision = 0
        self.reconciliations: dict[tuple[str, str], asyncio.Task] = {}
        self._snapshot_cache = None
        self.conversations = Conversations()
        self.resumes = Resumes(self)
        self.events = TurnEvents(self)
        self.controls = Controls(self)
        self.sender = TurnSender(self)

    def agent_name(self, agent_id: str | None, *, starts: bool = False) -> str:
        """The agent as the user knows it, for a message; «el asistente» when
        the runtime does not know which agent, or it left the configuration.
        `starts`: the name opens the sentence."""
        agent = self.agents.by_id.get(agent_id)
        if agent is not None:
            return agent.name
        return "The assistant" if starts else "the assistant"

    def _device_name(self) -> str:
        for row in self.store.devices.active():
            if row["id"] == self.device:
                return row["name"].replace("\r", " ").replace("\n", " ")
        raise HubError("revoked", "Device revoked")

    # The device's Macs ------------------------------------------------------

    def subscribe(self) -> asyncio.Queue:
        # 256 frames waiting for one connection: it protects the hub's memory
        # from a client that stops reading. A full queue drops that connection
        # with `slow_client` and the client reads its history again (`offer`).
        # Every connection of a device queues the same frame objects, so the
        # four connections ws.py admits do not hold four copies. Why 256 was not
        # measured: no burst of a real Hermes answer has been counted. It does
        # not limit a turn's length, the stored history or one frame's size.
        queue: asyncio.Queue = asyncio.Queue(maxsize=256)
        self.listeners.add(queue)
        if self.bindings:
            queue.put_nowait(emit("state.snapshot", self.snapshot()))
        return queue

    def snapshot(self) -> dict:
        """The device's live turns and their pending questions, in one frame.

        Bounded to the last 64 turns, 128 pending questions per turn and about
        100 KiB of JSON, with `omitted` set when anything was left out: the
        frame goes to every connection on each change, and the Mac's ceiling for
        a hub frame is 1 MiB (MAX_HUB_INCOMING_FRAME_BYTES). It does not limit
        what is pending: a question left out stays pending in the hub.
        Measured on 2026-10-21 with synthetic turns (64 × 128 approvals of
        2,048 characters): the largest frame sent was 105,747 bytes, and
        building it took a median 1.6 s on the event loop, because every
        question after the 100 KiB mark is still encoded once to be refused.
        The bound counts `json.dumps` with ASCII escapes, up to three times
        the bytes sent for characters outside the BMP. Why 64 and 128 was not
        measured; one turn rarely has more than one question pending.
        """
        if self._snapshot_cache is not None:
            return self._snapshot_cache
        sessions, omitted = [], len(self.turn_ids) > 64
        for session_id, turn_id in list(self.turn_ids.items())[-64:]:
            binding = self.store.chat.session(self.device, session_id)
            pending = []
            for request_id, (owner, kind, request) in self.pending.items():
                if owner != session_id:
                    continue
                if len(pending) == 128:
                    omitted = True
                    continue
                pending.append(pending_data(kind, request, request_id))
                if len(json.dumps([sessions, pending]).encode()) > 100 * 1024:
                    pending.pop()
                    omitted = True
            sessions.append(
                {
                    "session_id": session_id,
                    "turn_id": turn_id,
                    "status": binding["status"],
                    "pending": pending,
                    **(
                        {"retry": self.events.retries[session_id]}
                        if session_id in self.events.retries
                        else {}
                    ),
                }
            )
        self._snapshot_cache = {"revision": self.revision, "sessions": sessions, "omitted": omitted}
        return self._snapshot_cache

    def touched(self) -> None:
        """The turns or their questions changed: the next snapshot is new."""
        self.revision, self._snapshot_cache = self.revision + 1, None

    async def changed(self) -> None:
        self.touched()
        await self.publish(emit("state.snapshot", self.snapshot()))

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
                            "message": "Client fell behind; recover history",
                        },
                    )
                )
                queue.put_nowait(None)

    # Turns and their questions ------------------------------------------------

    def request_key(self, session_id: str, request_id: str) -> str | None:
        """The id the Mac knows a backend's request by, if it is pending.

        A session is one agent's, so the session and the backend's id name
        the request: two agents may both number theirs from 1."""
        for key, (owner, _kind, request) in self.pending.items():
            if owner == session_id and request.request_id == request_id:
                return key
        return None

    def new_request_key(self, session_id: str, request_id: str) -> str:
        """The id the Mac will know a new request by: the one it already has
        when the backend asks again, else the backend's own while no other
        pending request uses it, else one the hub makes. The Mac answers with
        the session and this id, which `pending` maps back to the backend's."""
        known = self.request_key(session_id, request_id)
        if known is not None:
            return known
        if request_id not in self.pending:
            return request_id
        return str(uuid4())

    def forget_requests(self, sessions: set[str]) -> None:
        """The pending questions of these conversations' turns, which ended."""
        self.pending = {
            key: value for key, value in self.pending.items() if value[0] not in sessions
        }

    def turn_state(self, session_id: str) -> TurnState:
        """The persisted session status is exactly one machine state."""
        return TurnState(self.store.chat.session(self.device, session_id)["status"])

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
            self.store.chat.set_status(session_id, advance(current, target).value)
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
            raise HubError("stale_turn", "The control does not belong to the current turn")
        if method == "chat.interrupt" and self.turn_state(params.session_id) in {
            TurnState.IDLE,
            TurnState.COMPLETED,
            TurnState.FAILED,
        }:
            raise HubError("stale_turn", "The turn is no longer active")
        if method == "approval.respond":
            pending = self.pending.get(params.request_id)
            if params.approved or pending is None or pending[:2] != (params.session_id, "approval"):
                raise HubError(
                    "interaction_not_found",
                    "Denial belongs to another session, is unknown or has expired",
                )
        request = getattr(params, "request_id", "")
        return target_key(method, params.session_id, turn_id, request), turn_id

    async def fail_turn(
        self, agent_id: str, session_id: str, turn_id: str, failure: TurnFailed | None = None
    ) -> None:
        await self.events.fail_turn(agent_id, session_id, turn_id, failure)

    # The agents' connections -----------------------------------------------------

    async def connect(self, agent_id: str) -> AssistantConnection:
        """This device's open connection to the agent, opening it if needed.
        Turns for one agent wait for its one opening; another agent's never do."""
        connection = self.connections.get(agent_id)
        if connection is not None and connection.connected:
            return connection
        async with self._opening.setdefault(agent_id, asyncio.Lock()):
            connection = self.connections.get(agent_id)
            if connection is None or not connection.connected:
                connection = await self._open(agent_id)
        return connection

    async def _open(self, agent_id: str) -> AssistantConnection:
        agent = self.agents.get(agent_id)
        connection = agent.backend.connection(
            self.device, self.device_name, partial(self.on_event, agent_id)
        )
        self.connections[agent_id] = connection
        try:
            await agent.integration.start(
                agent_id, agent.name, connection.open, round(self.now() * 1000)
            )
        except HubError as exc:
            if exc.code == "agent_start_timeout":
                # Cut half-way: what it started is stopped, never left running.
                await self._close(connection)
            raise
        return connection

    async def on_event(self, agent_id: str, event: AssistantEvent) -> None:
        """One event from this device's connection to the agent `agent_id`."""
        await self.events.on_event(agent_id, event)

    def require_live(self, session_id: str) -> tuple[dict, AssistantConnection]:
        """The session and its agent's connection, which holds it live."""
        binding = self.store.chat.session(self.device, session_id)
        connection = self.connections.get(self.agents.owner(binding))
        if (
            connection is None
            or not connection.connected
            or binding["runtime_id"] not in connection.live_sessions
        ):
            raise HubError(
                "session_disconnected",
                "The session is not connected to the assistant; sending another message will attempt to resume if the previous turn is closed",
            )
        return binding, connection

    async def resume_if_disconnected(
        self, session_id: str, agent_id: str, connection
    ) -> tuple[dict, bool | None]:
        return await self.resumes.reopen(session_id, agent_id, connection)

    @property
    def resume_waiter(self):
        """A turn waiting for its history, (its live session, its future), if any."""
        return self.resumes.waiter

    async def run_store(self, function: Callable[[], object]):
        """Dispatch a Store call onto the shared single-thread executor when available."""
        if self.store_executor is None:
            return function()
        return await in_store(self.store_executor, function)

    # Requests from the Mac ------------------------------------------------------

    def committed(self, request_id: str) -> bool:
        """Whether the message of this `chat.send` was stored."""
        return request_id in self._committed

    def commit(self, request_id: str) -> None:
        self._committed.add(request_id)

    def forget_commit(self, request_id: str) -> None:
        self._committed.discard(request_id)

    def log_chat_rejection(self, code, numbers=None):
        # A file's name is the user's words: the refusal carries it to the
        # Mac, never to the log.
        if numbers and "name" in numbers:
            numbers = {key: value for key, value in numbers.items() if key != "name"}
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
        if method == "chat.observe":
            return "state.snapshot", observe(self, params.session_id)
        if method == "ping":
            return "pong", {}
        if method == "chat.send":
            try:
                return await self.sender.send(params, request_id)
            except HubError as error:
                self.log_chat_rejection(error.code, error.numbers)
                raise
        return await self.controls.handle(method, params)

    # The end ---------------------------------------------------------------

    async def revoke(self) -> None:
        """Stop this device's live sessions even when it has no connected viewers.

        Interruption is best effort and cannot undo an action already executed upstream.
        This path is not used for a normal client disconnect.
        """
        if self.revoked:
            return
        self.revoked = True
        self.resumes.end("revoked")
        self.pending.clear()
        await self.publish(emit("revoked", {}))
        live = [
            (connection, runtime_id)
            for connection in self.connections.values()
            if connection.connected
            for runtime_id in tuple(connection.live_sessions)
        ]
        try:
            async with asyncio.timeout(2):
                await asyncio.gather(
                    *(connection.cancel(runtime_id) for connection, runtime_id in live),
                    return_exceptions=True,
                )
        except TimeoutError:
            pass
        await self.close()

    async def close(self) -> None:
        self.closed = True
        self.resumes.end("closed")
        for task in list(self.reconciliations.values()):
            task.cancel()
        await asyncio.gather(*list(self.reconciliations.values()), return_exceptions=True)
        for connection in list(self.connections.values()):
            await self._close(connection)

    async def _close(self, connection: AssistantConnection) -> None:
        try:
            async with asyncio.timeout(3):
                await connection.close()
        except Exception:  # noqa: BLE001 - redact close failure and preserve global revocation
            report(event="runtime_close_failed", code="close_failed", device=self.device)
