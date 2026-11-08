"""What ends or cuts a message waiting in its conversation's queue, beyond the
agent's own events; synthetic upstreams, invented words."""

import asyncio

import pytest
from test_turn_queue import HELD, interrupt, open_held_turn, send, settle, user_texts
from test_turn_queue import held as held  # noqa: PLC0414
from test_turn_queue_socket import finish

from orbit_hub.protocol import HubError
from orbit_hub.routes.assistants import _stop_processes


async def test_a_waiting_message_is_refused_when_the_hub_stops_its_agent(held):
    """Deleting an agent or signing it out closes its connection from the hub,
    and no backend reports a close it was asked for: the turn would stay under
    way with no deadline and the waiting message would never be answered."""
    now = [1001.0]
    runtime, store, owner, connection, session, _ = await open_held_turn(held, now)
    waiting = None
    try:
        waiting = send(runtime, session, "words behind a stopped agent")
        await settle()
        assert not waiting.done()

        await _stop_processes({owner: runtime}, "default")
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(waiting, 2)
        assert refused.value.code == "turn_outcome_unknown"
        assert store.chat.session(owner, session)["status"] == "outcome_unknown"
        assert len(connection.prompts()) == 1
        assert user_texts(store, owner, session) == [f"{HELD} uno"]
    finally:
        await finish(runtime, [waiting])


async def test_an_interrupt_of_a_waiting_message_does_not_wait_for_the_one_behind_it(held):
    """The cut names words with more words behind them: it applies once the
    words it names have gone. Those behind only leave when that turn ends, so
    waiting for them too would keep the cut from ever reaching its turn."""
    now = [1001.0]
    runtime, store, owner, connection, session, turn = await open_held_turn(held, now)
    named = "11111111-1111-4111-8111-111111111111"
    first = behind = stop = None
    try:
        await interrupt(runtime, session, turn)
        first = send(runtime, session, f"{HELD} dos", named)
        await settle()
        behind = send(runtime, session, "words behind the named ones")
        await settle()
        stop = asyncio.create_task(interrupt(runtime, session, named))
        await settle()
        assert not stop.done()

        await connection.end_held()
        await asyncio.wait_for(first, 2)
        name, _ = await asyncio.wait_for(stop, 1)
        assert name == "chat.interrupted"
        assert connection.interrupts() == 2
        assert store.chat.session(owner, session)["status"] == "interrupting"
        assert not behind.done()

        await connection.callback(
            {
                "type": "message.complete",
                "session_id": connection.calls[-1][1]["session_id"],
                "payload": {"text": "", "status": "interrupted"},
            }
        )
        await asyncio.wait_for(behind, 2)
        assert user_texts(store, owner, session) == [
            f"{HELD} uno",
            f"{HELD} dos",
            "words behind the named ones",
        ]
    finally:
        await finish(runtime, [first, behind, stop])
