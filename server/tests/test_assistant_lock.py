"""A control of one conversation never waits behind another agent's slow start.

Two agents on one hub. The first answers a turn that keeps streaming; the
second takes long to open a session, as an ACP agent does while Node, the
agent's binary and its MCP servers start (up to `acp.START_TIMEOUT_SECONDS`
plus `acp.OPEN_TIMEOUT_SECONDS`), or an `openai` agent while orbit-mcp starts
(`openai.TOOLS_START_TIMEOUT_SECONDS`). Interrupting or answering the first
agent's conversation meanwhile does not wait for the second agent; a control
on the conversation being opened still waits for it, so it applies to the turn
sent before it. Every agent here is a double; names are invented.
"""

import asyncio
from uuid import uuid4

from conftest import device

from orbit_hub.assistant.agents import Agent, Agents
from orbit_hub.assistant.backend import (
    Capabilities,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    ResumedSession,
)
from orbit_hub.assistant.hermes import HermesAgent
from orbit_hub.protocol import ApprovalParams, InterruptParams, SendParams
from orbit_hub.runtime import DeviceRuntime

# Far under any start budget above; far over the hub's own work for a control.
CONTROL_BUDGET_SECONDS = 2


class StreamingConnection:
    """A turn that asks one permission and keeps streaming until cancelled."""

    epoch = "streaming-epoch"

    def __init__(self, on_event):
        self.on_event = on_event
        self.live_sessions: set[str] = set()
        self.connected = False
        self.cancelled: list[str] = []

    async def open(self):
        self.connected = True

    async def close(self):
        self.connected = False

    async def create_session(self):
        live = f"live-{uuid4()}"
        self.live_sessions.add(live)
        return OpenedSession(live, f"stored-{live}")

    async def prompt(self, session_id, blocks):
        await self.on_event(
            PermissionRequested(
                session_id, PermissionRequest("permiso-1", "Crear la nota", ("once", "deny"))
            )
        )

    async def cancel(self, session_id):
        self.cancelled.append(session_id)

    async def respond_permission(self, session_id, request_id, *, choice):
        return PermissionReceipt(applied=True)

    async def turn_stopped(self, session_id):
        return False


class SlowStartConnection(StreamingConnection):
    """A session that opens only when the test lets it."""

    def __init__(self, on_event, started, release):
        super().__init__(on_event)
        self.started, self.release = started, release

    async def create_session(self):
        self.started.set()
        await self.release.wait()
        return await super().create_session()


class Backend:
    capabilities = Capabilities(images=False, resume=True, history=True, voice=False)

    def __init__(self, factory):
        self.factory = factory

    def connection(self, device, device_name, on_event):
        return self.factory(on_event)


def two_agents(started, release):
    return Agents(
        [
            Agent("vega", "Vega", "hermes", HermesAgent("vega"), Backend(StreamingConnection)),
            Agent(
                "orion",
                "Orión",
                "acp",
                None,
                Backend(lambda on_event: SlowStartConnection(on_event, started, release)),
            ),
        ],
        "vega",
    )


async def test_an_interrupt_does_not_wait_for_another_agent_to_open_a_session(environment):
    _, store, _ = environment
    started, release = asyncio.Event(), asyncio.Event()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, two_agents(started, release), lambda: 1001)
    turn = str(uuid4())
    opening = None
    try:
        _, data = await runtime.handle("chat.send", SendParams(text="Escribe la nota"), turn)
        session = data["session_id"]
        # A new conversation with the slow agent, from another window.
        opening = asyncio.create_task(
            runtime.handle("chat.send", SendParams(text="Hola", agent_id="orion"), str(uuid4()))
        )
        await asyncio.wait_for(started.wait(), CONTROL_BUDGET_SECONDS)
        name, _ = await asyncio.wait_for(
            runtime.handle(
                "chat.interrupt", InterruptParams(session_id=session, turn_id=turn), str(uuid4())
            ),
            CONTROL_BUDGET_SECONDS,
        )
        assert name == "chat.interrupted"
    finally:
        release.set()
        if opening is not None:
            await asyncio.gather(opening, return_exceptions=True)
        await runtime.close()


async def test_an_approval_does_not_wait_for_another_agent_to_open_a_session(environment):
    _, store, _ = environment
    started, release = asyncio.Event(), asyncio.Event()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, two_agents(started, release), lambda: 1001)
    turn = str(uuid4())
    opening = None
    try:
        _, data = await runtime.handle("chat.send", SendParams(text="Escribe la nota"), turn)
        session = data["session_id"]
        [request_id] = runtime.pending
        opening = asyncio.create_task(
            runtime.handle("chat.send", SendParams(text="Hola", agent_id="orion"), str(uuid4()))
        )
        await asyncio.wait_for(started.wait(), CONTROL_BUDGET_SECONDS)
        name, _ = await asyncio.wait_for(
            runtime.handle(
                "approval.respond",
                ApprovalParams(
                    session_id=session, request_id=request_id, approved=True, turn_id=turn
                ),
                str(uuid4()),
            ),
            CONTROL_BUDGET_SECONDS,
        )
        assert name == "interaction.resolved"
    finally:
        release.set()
        if opening is not None:
            await asyncio.gather(opening, return_exceptions=True)
        await runtime.close()


class SlowReopenConnection(StreamingConnection):
    """A connection that opens only when the test lets it, then resumes."""

    epoch = "reopened-epoch"

    def __init__(self, on_event, started, release):
        super().__init__(on_event)
        self.started, self.release = started, release

    async def open(self):
        self.started.set()
        await self.release.wait()
        self.connected = True

    async def resume_session(self, stored_id, history=None):
        live = f"live-{uuid4()}"
        self.live_sessions.add(live)
        return ResumedSession(live, stored_id, None, False)


async def test_an_interrupt_of_the_conversation_being_reopened_applies_to_its_new_turn(
    environment,
):
    _, store, _ = environment
    started, release = asyncio.Event(), asyncio.Event()
    made: list = []

    def factory(on_event):
        if made:
            connection = SlowReopenConnection(on_event, started, release)
        else:
            connection = StreamingConnection(on_event)
        made.append(connection)
        return connection

    agents = Agents(
        [Agent("vega", "Vega", "hermes", HermesAgent("vega"), Backend(factory))], "vega"
    )
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, agents, lambda: 1001)
    sending = interrupting = None
    try:
        _, data = await runtime.handle("chat.send", SendParams(text="Primera"), str(uuid4()))
        session = data["session_id"]
        store.status(session, "completed")
        runtime.pending.clear()
        made[0].connected = False
        turn = str(uuid4())
        sending = asyncio.create_task(
            runtime.handle("chat.send", SendParams(text="Otra", session_id=session), turn)
        )
        await asyncio.wait_for(started.wait(), CONTROL_BUDGET_SECONDS)
        interrupting = asyncio.create_task(
            runtime.handle(
                "chat.interrupt", InterruptParams(session_id=session, turn_id=turn), str(uuid4())
            )
        )
        await asyncio.sleep(0.05)
        assert not interrupting.done()
        release.set()
        name, _ = await asyncio.wait_for(interrupting, CONTROL_BUDGET_SECONDS)
        assert name == "chat.interrupted"
        assert made[1].cancelled
    finally:
        release.set()
        await asyncio.gather(
            *(task for task in (sending, interrupting) if task is not None),
            return_exceptions=True,
        )
        await runtime.close()
