"""A message sent while its conversation's turn is still under way waits for
that turn and is then answered, in arrival order; synthetic upstreams only.

In a call the Mac retires its turn the moment the user speaks over the answer
and sends the new words at once, while the hub still waits for the agent to
confirm the interruption: those words must not be refused.
"""

import asyncio
from uuid import uuid4

import pytest
from conftest import FakeConnection, device, only

from orbit_hub.assistant.backend import ConnectionLost
from orbit_hub.protocol import ApprovalParams, HubError, InterruptParams, SendParams
from orbit_hub.runtime import DeviceRuntime

# A turn whose text carries this mark is held by the synthetic agent until
# the test ends it; every other turn is answered at once.
HELD = "pregunta retenida"


class HeldConnection(FakeConnection):
    """Hermes over a synthetic transport that keeps a held turn live, accepts
    interruptions without ending it, and ends it when the test says so."""

    async def rpc(self, method, params):
        if method == "prompt.submit" and HELD in params["text"]:
            self.calls.append((method, params))
            return {"status": "streaming"}
        if method == "session.interrupt":
            self.calls.append((method, params))
            return {"status": "interrupted"}
        return await super().rpc(method, params)

    def prompts(self) -> list[str]:
        return [params["text"] for method, params in self.calls if method == "prompt.submit"]

    def interrupts(self) -> int:
        return sum(1 for method, _ in self.calls if method == "session.interrupt")

    async def end_held(self, status: str = "interrupted") -> None:
        """The agent ends the held turn, as Hermes does once it stops."""
        runtime_id = next(
            params["session_id"] for method, params in self.calls if method == "prompt.submit"
        )
        await self.callback(
            {
                "type": "message.complete",
                "session_id": runtime_id,
                "payload": {"text": "", "status": status},
            }
        )


@pytest.fixture
def held(environment, monkeypatch):
    _, _, adapter = environment

    def connection(device_id, device_name, callback):
        value = HeldConnection(callback, device_id, device_name)
        adapter.connections.append(value)
        return value

    monkeypatch.setattr(adapter, "connection", connection)
    return environment


async def settle() -> None:
    """Lets every waiting task run until it blocks again."""
    for _ in range(5):
        await asyncio.sleep(0.01)


async def open_held_turn(held, now):
    _, store, adapter = held
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: now[0])
    turn = str(uuid4())
    _, data = await runtime.handle("chat.send", SendParams(text=f"{HELD} uno"), turn)
    return runtime, store, owner, adapter.connections[0], data["session_id"], turn


def send(runtime, session, text, turn=None):
    return asyncio.create_task(
        runtime.handle("chat.send", SendParams(text=text, session_id=session), turn or str(uuid4()))
    )


async def interrupt(runtime, session, turn):
    return await runtime.handle(
        "chat.interrupt", InterruptParams(session_id=session, turn_id=turn), str(uuid4())
    )


def user_texts(store, owner, session) -> list[str]:
    page = store.chat.session_messages(owner, session, None, 50)
    return [message["text"] for message in page["messages"] if message["role"] == "user"]


async def test_words_sent_while_the_turn_is_being_interrupted_wait_and_are_answered(held):
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    waiting = None
    try:
        await interrupt(runtime, session, turn)
        assert store.chat.session(owner, session)["status"] == "interrupting"

        waiting = send(runtime, session, "y otra cosa sintética")
        await settle()
        # Nothing refused and nothing sent over the turn being interrupted.
        assert not waiting.done()
        assert len(connection.prompts()) == 1

        await connection.end_held()
        name, data = await asyncio.wait_for(waiting, 2)
        assert name == "chat.session"
        assert data["session_id"] == session
        prompts = connection.prompts()
        assert len(prompts) == 2
        assert "y otra cosa sintética" in prompts[1]
        assert user_texts(store, owner, session) == [f"{HELD} uno", "y otra cosa sintética"]
        assert store.chat.session(owner, session)["status"] == "completed"
    finally:
        if waiting is not None:
            await asyncio.gather(waiting, return_exceptions=True)
        await runtime.close()


async def test_words_sent_while_the_turn_still_answers_wait_for_its_end(held):
    now = [1001.0]
    runtime, store, owner, connection, session, _ = await open_held_turn(held, now)
    waiting = None
    try:
        waiting = send(runtime, session, "pregunta siguiente")
        await settle()
        assert not waiting.done()
        assert len(connection.prompts()) == 1

        await connection.end_held(status="success")
        await asyncio.wait_for(waiting, 2)
        assert "pregunta siguiente" in connection.prompts()[1]
        assert user_texts(store, owner, session) == [f"{HELD} uno", "pregunta siguiente"]
    finally:
        if waiting is not None:
            await asyncio.gather(waiting, return_exceptions=True)
        await runtime.close()


async def test_waiting_messages_go_in_arrival_order(held):
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    waiting = []
    try:
        await interrupt(runtime, session, turn)
        for text in ("primero sintético", "segundo sintético", "tercero sintético"):
            waiting.append(send(runtime, session, text))
            await settle()

        await connection.end_held()
        await asyncio.wait_for(asyncio.gather(*waiting), 2)
        prompts = connection.prompts()
        assert len(prompts) == 4
        assert ["primero" in prompts[1], "segundo" in prompts[2], "tercero" in prompts[3]] == [
            True,
            True,
            True,
        ]
        assert user_texts(store, owner, session) == [
            f"{HELD} uno",
            "primero sintético",
            "segundo sintético",
            "tercero sintético",
        ]
    finally:
        await asyncio.gather(*waiting, return_exceptions=True)
        await runtime.close()


async def test_speaking_over_a_waiting_turn_interrupts_it_once_sent_and_the_next_words_follow(
    held,
):
    """Two barge-ins in a row: the first words wait behind the turn being
    interrupted, the second barge-in interrupts them, and the second words
    wait in their turn. Nothing is refused and nothing is lost."""
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    first_turn = str(uuid4())
    tasks = []
    try:
        await interrupt(runtime, session, turn)
        first = send(runtime, session, f"{HELD} dos", first_turn)
        tasks.append(first)
        await settle()
        # The Mac interrupts the turn it has just sent, then sends what follows.
        stop_first = asyncio.create_task(interrupt(runtime, session, first_turn))
        tasks.append(stop_first)
        await settle()
        second = send(runtime, session, "tercera frase sintética")
        tasks.append(second)
        await settle()
        assert len(connection.prompts()) == 1

        await connection.end_held()
        await asyncio.wait_for(first, 2)
        name, _ = await asyncio.wait_for(stop_first, 2)
        assert name == "chat.interrupted"
        assert connection.interrupts() == 2
        assert not second.done()

        # The agent stops the second held turn: the words after it go.
        runtime_id = connection.calls[-1][1]["session_id"]
        await connection.callback(
            {
                "type": "message.complete",
                "session_id": runtime_id,
                "payload": {"text": "", "status": "interrupted"},
            }
        )
        await asyncio.wait_for(second, 2)
        prompts = connection.prompts()
        assert len(prompts) == 3
        assert f"{HELD} dos" in prompts[1]
        assert "tercera frase sintética" in prompts[2]
        assert user_texts(store, owner, session) == [
            f"{HELD} uno",
            f"{HELD} dos",
            "tercera frase sintética",
        ]
    finally:
        await asyncio.gather(*tasks, return_exceptions=True)
        await runtime.close()


async def test_an_answer_to_the_current_turn_does_not_wait_for_a_waiting_message(held):
    """The agent asks for an approval in the turn still answering and a message
    waits behind that turn: the answer reaches the agent at once, without
    waiting for the turn to end, and the message goes once it ends."""
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    runtime_id = next(
        params["session_id"] for method, params in connection.calls if method == "prompt.submit"
    )
    waiting = None
    try:
        await connection.callback(
            {
                "type": "approval.request",
                "session_id": runtime_id,
                "payload": {"request_id": "permiso-sintetico"},
            }
        )
        [request_id] = runtime.pending
        assert store.chat.session(owner, session)["status"] == "streaming"

        waiting = send(runtime, session, "mensaje tras el permiso")
        await settle()
        assert not waiting.done(), f"the message did not wait: {waiting.exception()!r}"

        # Its own short deadline: an answer that waits for the message would
        # wait for the whole held turn, which only the test ends.
        name, data = await asyncio.wait_for(
            runtime.handle(
                "approval.respond",
                ApprovalParams(
                    session_id=session, request_id=request_id, approved=True, turn_id=turn
                ),
                str(uuid4()),
            ),
            1,
        )
        assert name == "interaction.resolved"
        assert data["applied"] is True
        assert "stale_interaction" not in data
        assert [method for method, _ in connection.calls].count("approval.respond") == 1
        assert request_id not in runtime.pending
        # The answer did not end the turn, nor let the message through.
        assert not waiting.done()
        assert len(connection.prompts()) == 1

        await connection.end_held(status="success")
        name, data = await asyncio.wait_for(waiting, 2)
        assert name == "chat.session"
        assert data["session_id"] == session
        prompts = connection.prompts()
        assert len(prompts) == 2
        assert "mensaje tras el permiso" in prompts[1]
        assert user_texts(store, owner, session) == [f"{HELD} uno", "mensaje tras el permiso"]
        assert store.chat.session(owner, session)["status"] == "completed"
    finally:
        if waiting is not None:
            # A held turn that never ends would keep the message waiting.
            if not waiting.done():
                waiting.cancel()
            await asyncio.gather(waiting, return_exceptions=True)
        await runtime.close()


async def test_a_waiting_message_is_refused_as_unknown_outcome_when_the_interrupt_expires(held):
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    waiting = None
    try:
        await interrupt(runtime, session, turn)
        now[0] = 1030.9
        waiting = send(runtime, session, "espera sin final")
        await settle()
        assert not waiting.done()

        # The agent never confirms: at the deadline the turn's outcome is
        # unknown, and the waiting message is never sent over it.
        now[0] = 1031.0
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(waiting, 5)
        assert refused.value.code == "turn_outcome_unknown"
        assert store.chat.session(owner, session)["status"] == "outcome_unknown"
        assert runtime.turn_ids[session] == turn
        assert len(connection.prompts()) == 1
        assert user_texts(store, owner, session) == [f"{HELD} uno"]
    finally:
        if waiting is not None:
            await asyncio.gather(waiting, return_exceptions=True)
        await runtime.close()


async def test_a_waiting_message_is_refused_with_its_cause_when_the_agent_connection_is_lost(
    held,
):
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    waiting = None
    try:
        await interrupt(runtime, session, turn)
        waiting = send(runtime, session, "se pierde la conexión")
        await settle()
        assert not waiting.done()

        connection.connected = False
        await runtime.on_event("default", ConnectionLost())
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(waiting, 2)
        assert refused.value.code in {"turn_outcome_unknown", "session_resume_connection_lost"}
        assert len(connection.prompts()) == 1
        assert user_texts(store, owner, session) == [f"{HELD} uno"]
    finally:
        if waiting is not None:
            await asyncio.gather(waiting, return_exceptions=True)
        await runtime.close()


async def test_the_waiting_messages_of_a_conversation_have_a_ceiling_with_its_numbers(held):
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    waiting = []
    refused = None
    try:
        await interrupt(runtime, session, turn)
        for index in range(50):
            task = send(runtime, session, f"mensaje sintético {index}")
            await settle()
            if task.done():
                refused = task.exception()
                break
            waiting.append(task)
        assert isinstance(refused, HubError)
        # Its own code: `too_many` already means pending captures to the app.
        assert refused.code == "too_many_waiting_turns"
        assert refused.numbers["max"] == len(waiting)
        assert refused.numbers["count"] == len(waiting) + 1
        # Two barge-ins in a row must fit.
        assert len(waiting) >= 2

        # The accepted ones are all answered, in order.
        await connection.end_held()
        await asyncio.wait_for(asyncio.gather(*waiting), 2)
        assert user_texts(store, owner, session) == [f"{HELD} uno"] + [
            f"mensaje sintético {index}" for index in range(len(waiting))
        ]
    finally:
        await asyncio.gather(*waiting, return_exceptions=True)
        await runtime.close()
