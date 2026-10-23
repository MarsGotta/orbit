"""One conversation, or one agent, never holds up another on the same Mac.

Each agent here is a double whose calls the test holds open: a start that
never finishes, a start that fails, a reopen (`session/load`) that takes long,
an answer to an approval that takes long. Meanwhile another conversation of
the same Mac, with the same agent or another, still answers within a budget
far under any agent's own start bound and far over the hub's own work. Every
name is invented.
"""

import asyncio
from uuid import uuid4

import pytest
from conftest import device

from orbit_hub.assistant.agents import Agent, Agents
from orbit_hub.assistant.backend import (
    Capabilities,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    ResumedSession,
)
from orbit_hub.assistant.hermes import HermesAgent
from orbit_hub.protocol import ApprovalParams, HubError, InterruptParams, SendParams
from orbit_hub.runtime import DeviceRuntime

# Far under any agent's start budget (ACP's is 150 s); far over the hub's own
# work for a turn of a double.
BUDGET_SECONDS = 2


class Gate:
    """Holds a call until the test opens it, and says when one arrived."""

    def __init__(self):
        self.arrived, self.opened = asyncio.Event(), asyncio.Event()

    async def hold(self):
        self.arrived.set()
        await self.opened.wait()


class Connection:
    """A connection whose calls a test can hold or fail."""

    epoch = "double-epoch"

    def __init__(self, on_event, options):
        self.on_event = on_event
        self.options = options
        self.live_sessions: set[str] = set()
        self.connected = False
        self.closed = False
        self.cancelled: list[str] = []

    async def open(self):
        failures = self.options.get("open_failures")
        if failures:
            raise failures.pop(0)
        if (gate := self.options.get("open_gate")) is not None:
            await gate.hold()
        self.connected = True

    async def close(self):
        self.connected, self.closed = False, True

    async def create_session(self):
        live = f"live-{uuid4()}"
        self.live_sessions.add(live)
        return OpenedSession(live, f"stored-{live}")

    async def resume_session(self, stored_id, history=None):
        if (gate := self.options.get("resume_gate")) is not None:
            await gate.hold()
        live = f"live-{uuid4()}"
        self.live_sessions.add(live)
        return ResumedSession(live, stored_id, None, False)

    async def prompt(self, session_id, blocks):
        if self.options.get("ask"):
            request = PermissionRequest("permiso-1", "Crear la nota", ("once", "deny"))
            await self.on_event(PermissionRequested(session_id, request))
        else:
            await self.on_event(MessageComplete(session_id, "Hecho", "success"))

    async def cancel(self, session_id):
        self.cancelled.append(session_id)

    async def respond_permission(self, session_id, request_id, *, choice):
        if (gate := self.options.get("answer_gate")) is not None:
            await gate.hold()
        return PermissionReceipt(applied=True)

    async def turn_stopped(self, session_id):
        return False


class Backend:
    capabilities = Capabilities(images=False, resume=True, history=True, voice=False)

    def __init__(self, **options):
        self.options = options
        self.connections: list[Connection] = []

    def connection(self, device, device_name, on_event):
        connection = Connection(on_event, self.options)
        self.connections.append(connection)
        return connection


def registry(**backends) -> Agents:
    """Invented agents, the first the default, each on its double."""
    return Agents(
        [
            Agent(agent_id, agent_id.capitalize(), "hermes", HermesAgent(agent_id), backend)
            for agent_id, backend in backends.items()
        ],
        next(iter(backends)),
    )


def send(runtime, text, *, session=None, agent=None, turn=None):
    return runtime.handle(
        "chat.send", SendParams(text=text, session_id=session, agent_id=agent), turn or str(uuid4())
    )


def listed(agents: Agents) -> dict[str, dict]:
    return {agent["id"]: agent for agent in agents.public()}


async def test_an_agent_that_never_finishes_starting_does_not_stop_the_others(environment):
    _, store, _ = environment
    gate = Gate()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(
        owner, store, registry(vega=Backend(), orion=Backend(open_gate=gate)), lambda: 1001
    )
    stuck = asyncio.create_task(send(runtime, "Hola", agent="orion"))
    try:
        await asyncio.wait_for(gate.arrived.wait(), BUDGET_SECONDS)
        name, data = await asyncio.wait_for(send(runtime, "Hola", agent="vega"), BUDGET_SECONDS)
        assert name == "chat.session" and data["status"] == "completed"
    finally:
        gate.opened.set()
        await asyncio.gather(stuck, return_exceptions=True)
        await runtime.close()


async def test_an_agent_that_fails_to_start_is_listed_failed_and_the_others_answer(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    failure = HubError("agent_start_failed", "El agente «Orion» (/opt/orion) no arranca: no existe")
    agents = registry(vega=Backend(), orion=Backend(open_failures=[failure]))
    changes: list[dict] = []
    agents.changed = lambda: changes.append(listed(agents))
    runtime = DeviceRuntime(owner, store, agents, lambda: 1001)
    try:
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(send(runtime, "Hola", agent="orion"), BUDGET_SECONDS)
        assert refused.value.code == "agent_start_failed"
        # The Macs learn it, as `agents.changed`, and the list says why.
        assert changes[-1]["orion"]["backend"] == {
            "state": "failed",
            "code": "agent_start_failed",
            "at_ms": 1001000,
        }
        assert "backend" not in changes[-1]["vega"]
        name, data = await asyncio.wait_for(send(runtime, "Hola", agent="vega"), BUDGET_SECONDS)
        assert name == "chat.session" and data["status"] == "completed"
        # It starts the next time, and the list stops saying it failed.
        name, data = await asyncio.wait_for(send(runtime, "Otra", agent="orion"), BUDGET_SECONDS)
        assert data["status"] == "completed"
        assert "backend" not in listed(agents)["orion"]
        assert "backend" not in changes[-1]["orion"]
    finally:
        await runtime.close()


async def test_an_agent_start_that_outlives_its_bound_is_cut_and_said(environment, monkeypatch):
    monkeypatch.setattr("orbit_hub.assistant.integration.START_CEILING_SECONDS", 0.05)
    _, store, _ = environment
    gate = Gate()
    owner = device(store)["device_id"]
    orion = Backend(open_gate=gate)
    agents = registry(vega=Backend(), orion=orion)
    runtime = DeviceRuntime(owner, store, agents, lambda: 1001)
    try:
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(send(runtime, "Hola", agent="orion"), BUDGET_SECONDS)
        assert refused.value.code == "agent_start_timeout"
        assert refused.value.numbers == {"agent_id": "orion", "timeout_ms": 50}
        assert refused.value.message.startswith("Orion no terminó de arrancar en 50 ms")
        # The half-open connection is closed, never left holding the device.
        assert orion.connections[0].closed
        assert listed(agents)["orion"]["backend"]["code"] == "agent_start_timeout"
        _, data = await asyncio.wait_for(send(runtime, "Hola", agent="vega"), BUDGET_SECONDS)
        assert data["status"] == "completed"
    finally:
        gate.opened.set()
        await runtime.close()


async def test_a_slow_answer_in_one_conversation_does_not_delay_another(environment):
    _, store, _ = environment
    gate = Gate()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(
        owner, store, registry(vega=Backend(ask=True, answer_gate=gate)), lambda: 1001
    )
    first_turn = str(uuid4())
    answering = None
    try:
        _, first = await send(runtime, "Escribe la nota", turn=first_turn)
        [request_id] = runtime.pending
        answering = asyncio.create_task(
            runtime.handle(
                "approval.respond",
                ApprovalParams(
                    session_id=first["session_id"],
                    request_id=request_id,
                    approved=True,
                    turn_id=first_turn,
                ),
                str(uuid4()),
            )
        )
        await asyncio.wait_for(gate.arrived.wait(), BUDGET_SECONDS)
        # Another conversation of the same Mac starts a turn and stops it.
        second_turn = str(uuid4())
        name, second = await asyncio.wait_for(
            send(runtime, "Otra cosa", turn=second_turn), BUDGET_SECONDS
        )
        assert name == "chat.session" and second["status"] == "streaming"
        name, _ = await asyncio.wait_for(
            runtime.handle(
                "chat.interrupt",
                InterruptParams(session_id=second["session_id"], turn_id=second_turn),
                str(uuid4()),
            ),
            BUDGET_SECONDS,
        )
        assert name == "chat.interrupted"
        assert not answering.done()
        gate.opened.set()
        name, _ = await asyncio.wait_for(answering, BUDGET_SECONDS)
        assert name == "interaction.resolved"
    finally:
        gate.opened.set()
        if answering is not None:
            await asyncio.gather(answering, return_exceptions=True)
        await runtime.close()


async def test_a_slow_reopen_of_one_conversation_does_not_delay_another(environment):
    _, store, _ = environment
    gate = Gate()
    owner = device(store)["device_id"]
    vega = Backend(resume_gate=gate)
    runtime = DeviceRuntime(owner, store, registry(vega=vega), lambda: 1001)
    reopening = None
    try:
        _, first = await send(runtime, "Primera")
        _, second = await send(runtime, "Segunda")
        # The agent gave the first conversation's process up between turns, as
        # an idle ACP session does: its next turn reopens it (`session/load`).
        [live] = [key[1] for key, held in runtime.bindings.items() if held == first["session_id"]]
        vega.connections[0].live_sessions.discard(live)
        reopening = asyncio.create_task(send(runtime, "Sigo", session=first["session_id"]))
        await asyncio.wait_for(gate.arrived.wait(), BUDGET_SECONDS)
        name, data = await asyncio.wait_for(
            send(runtime, "Y aquí", session=second["session_id"]), BUDGET_SECONDS
        )
        assert name == "chat.session" and data["status"] == "completed"
        assert not reopening.done()
        gate.opened.set()
        _, reopened = await asyncio.wait_for(reopening, BUDGET_SECONDS)
        assert reopened == {"session_id": first["session_id"], "status": "completed"}
    finally:
        gate.opened.set()
        if reopening is not None:
            await asyncio.gather(reopening, return_exceptions=True)
        await runtime.close()
