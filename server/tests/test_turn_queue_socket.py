"""What the queue of a conversation's messages must hold beyond the runtime.

Over the real socket a control is reserved in the read loop before it runs, and
controls of a conversation take turns in a lane: neither may refuse the words
waiting or keep an answer from the agent that asked. And every way a turn moves
must reach a message already waiting. Synthetic upstreams, invented names.
"""

import asyncio
import contextlib
import queue
import threading
import time
from contextlib import contextmanager
from uuid import uuid4

import pytest
from conftest import device
from fastapi.testclient import TestClient
from test_turn_queue import HELD, interrupt, open_held_turn, send, settle, user_texts
from test_turn_queue import held as held  # noqa: PLC0414

from orbit_hub.app import create_app
from orbit_hub.protocol import HubError

# Long enough for a frame just sent to reach its lane and wait there.
REACHED_SECONDS = 0.2


class Frames:
    """The socket's frames, read off the test's thread: a frame that never
    comes fails its example instead of holding the run."""

    def __init__(self, ws):
        self.ws = ws
        self.seen: list[dict] = []
        self.incoming: queue.SimpleQueue = queue.SimpleQueue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        try:
            while True:
                self.incoming.put(self.ws.receive_json())
        except Exception:  # noqa: BLE001 - the socket closed, however the client reports it
            return

    def _match(self, frame_id: str, event: str) -> dict | None:
        for frame in self.seen:
            if frame.get("id") == frame_id and frame["event"] in {event, "error"}:
                return frame
        return None

    def answer(self, frame_id: str, event: str, seconds: float = 2.0) -> dict:
        """The answer to one frame: `event` or a refusal."""
        deadline = time.monotonic() + seconds
        while (found := self._match(frame_id, event)) is None:
            try:
                self.seen.append(self.incoming.get(timeout=max(deadline - time.monotonic(), 0)))
            except queue.Empty:
                pytest.fail(f"no {event} or refusal for this frame within {seconds} s")
        return found

    def assert_unanswered(
        self, frame_id: str, event: str, seconds: float = REACHED_SECONDS
    ) -> None:
        """The frame still waits after `seconds`: neither answered nor refused."""
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0:
            try:
                self.seen.append(self.incoming.get(timeout=remaining))
            except queue.Empty:
                break
        answer = self._match(frame_id, event)
        assert answer is None, f"expected it to wait, got {answer['event']}: {answer['data']}"


class Mac:
    """One paired device over the real socket, against the held synthetic agent."""

    def __init__(self, http, ws, adapter, store, owner):
        self.http, self.ws, self.adapter, self.store, self.owner = http, ws, adapter, store, owner
        self.frames = Frames(ws)

    @property
    def connection(self):
        return self.adapter.connections[0]

    def request(self, method: str, **params) -> str:
        frame_id = str(uuid4())
        self.ws.send_json({"v": 1, "id": frame_id, "method": method, "params": params})
        return frame_id

    def waiting_send(self, session: str, text: str) -> str:
        """Sends words the hub must keep waiting, and lets them reach the queue."""
        frame_id = self.request("chat.send", text=text, session_id=session)
        time.sleep(REACHED_SECONDS)
        return frame_id

    def parked_cut(self, session: str, turn: str) -> str:
        frame_id = self.request("chat.interrupt", session_id=session, turn_id=turn)
        time.sleep(REACHED_SECONDS)
        return frame_id

    def end_turn(self, status: str = "interrupted") -> None:
        self.http.portal.call(self.connection.end_held, status)

    def ask_permission(self) -> str:
        live = next(
            params["session_id"]
            for method, params in self.connection.calls
            if method == "prompt.submit"
        )
        self.http.portal.call(
            self.connection.callback,
            {
                "type": "approval.request",
                "session_id": live,
                "payload": {"request_id": "synthetic-permission"},
            },
        )
        return "synthetic-permission"

    def open_held_turn(self) -> tuple[str, str]:
        turn = self.request("chat.send", text=f"{HELD} one")
        answer = self.frames.answer(turn, "chat.session")
        assert answer["event"] == "chat.session", answer["data"]
        return answer["data"]["session_id"], turn

    def cut(self, session: str, turn: str) -> None:
        answer = self.frames.answer(
            self.request("chat.interrupt", session_id=session, turn_id=turn), "chat.interrupted"
        )
        assert answer["event"] == "chat.interrupted", answer["data"]


@contextmanager
def paired_mac(held):
    settings, store, adapter = held
    paired = device(store, name="Synthetic Laptop")
    headers = {"Authorization": "Bearer " + paired["token"]}
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        yield Mac(http, ws, adapter, store, paired["device_id"])


def test_an_interrupt_naming_a_waiting_message_waits_with_it_over_the_socket(held):
    """Two barge-ins in a row, as the Mac sends them: the second interrupt names
    the words still waiting. Three sends, two interrupts, no refusal."""
    with paired_mac(held) as mac:
        session, first = mac.open_held_turn()
        mac.cut(session, first)
        later = mac.waiting_send(session, f"{HELD} two")

        cut_later = mac.parked_cut(session, later)
        mac.frames.assert_unanswered(cut_later, "chat.interrupted")
        third = mac.waiting_send(session, "third synthetic words")
        assert len(mac.connection.prompts()) == 1

        mac.end_turn()
        assert mac.frames.answer(later, "chat.session")["event"] == "chat.session"
        answer = mac.frames.answer(cut_later, "chat.interrupted")
        assert answer["event"] == "chat.interrupted", answer["data"]
        assert mac.connection.interrupts() == 2
        mac.frames.assert_unanswered(third, "chat.session")

        mac.end_turn()
        assert mac.frames.answer(third, "chat.session")["event"] == "chat.session"
        prompts = mac.connection.prompts()
        assert len(prompts) == 3
        assert f"{HELD} two" in prompts[1]
        assert "third synthetic words" in prompts[2]
        assert user_texts(mac.store, mac.owner, session) == [
            f"{HELD} one",
            f"{HELD} two",
            "third synthetic words",
        ]


def test_a_repeated_interrupt_of_a_waiting_message_keeps_its_receipt(held):
    """A cut parked behind waiting words keeps its receipt while another control
    is reserved: the same cut sent again is refused, never applied twice."""
    with paired_mac(held) as mac:
        session, first = mac.open_held_turn()
        mac.cut(session, first)
        later = mac.waiting_send(session, f"{HELD} two")
        cut_later = mac.parked_cut(session, later)
        third = mac.waiting_send(session, f"{HELD} three")
        cut_third = mac.parked_cut(session, third)

        again = mac.request("chat.interrupt", session_id=session, turn_id=later)
        answer = mac.frames.answer(again, "chat.interrupted")
        assert answer["event"] == "error"
        assert answer["data"]["code"] == "outcome_unknown"

        mac.end_turn()
        assert mac.frames.answer(cut_later, "chat.interrupted")["event"] == "chat.interrupted"
        mac.end_turn()
        assert mac.frames.answer(cut_third, "chat.interrupted")["event"] == "chat.interrupted"
        # The first turn, the second and the third: once each.
        assert mac.connection.interrupts() == 3


@pytest.mark.parametrize("named", ["waiting_message", "current_turn"])
def test_an_answer_goes_past_an_interrupt_parked_behind_a_waiting_message(held, named):
    """The turn under way asks for a permission, words wait behind it and a cut
    is parked behind those words: the answer must reach the agent, or the turn
    waits for the answer, the answer for the cut and the cut for the turn."""
    with paired_mac(held) as mac:
        session, first = mac.open_held_turn()
        request_id = mac.ask_permission()
        later = mac.waiting_send(session, f"{HELD} two")
        cut = mac.parked_cut(session, later if named == "waiting_message" else first)
        mac.frames.assert_unanswered(cut, "chat.interrupted")

        reply = mac.request(
            "approval.respond",
            session_id=session,
            request_id=request_id,
            approved=True,
            turn_id=first,
        )
        answer = mac.frames.answer(reply, "interaction.resolved", seconds=1)
        assert answer["event"] == "interaction.resolved", answer["data"]
        assert answer["data"]["applied"] is True
        calls = [method for method, _ in mac.connection.calls]
        assert calls.count("approval.respond") == 1
        # The answer neither ended the turn nor let the cut or the words through.
        mac.frames.assert_unanswered(cut, "chat.interrupted")
        assert len(mac.connection.prompts()) == 1

        mac.end_turn("success")
        assert mac.frames.answer(later, "chat.session")["event"] == "chat.session"
        closing = mac.frames.answer(cut, "chat.interrupted")
        if named == "waiting_message":
            assert closing["event"] == "chat.interrupted", closing["data"]
            assert mac.connection.interrupts() == 1
        else:
            # It named a turn that has ended by the time it runs.
            assert closing["event"] == "error"
            assert closing["data"]["code"] == "stale_turn"
            assert mac.connection.interrupts() == 0


async def finish(runtime, tasks) -> None:
    """Closes the runtime and ends what a failed example left waiting, so a
    wrong build fails its example instead of holding the run."""
    await runtime.close()
    left = [task for task in tasks if task is not None]
    with contextlib.suppress(TimeoutError):
        await asyncio.wait_for(asyncio.gather(*left, return_exceptions=True), 2)


class WallClock:
    """The hub's clock, for a deadline that passes by itself."""

    def __getitem__(self, _index) -> float:
        return time.monotonic()


async def test_a_message_arriving_while_the_interrupt_is_in_flight_is_refused_at_the_deadline(
    held, monkeypatch
):
    """The barge-in as it happens: the words arrive while the agent has not yet
    accepted the interruption, so the turn is still `streaming` and has no
    deadline. The deadline is set once the words wait, and it must reach them."""
    monkeypatch.setattr("orbit_hub.runtime.controls.INTERRUPT_DEADLINE_SECONDS", 0.3)
    runtime, store, owner, connection, session, turn = await open_held_turn(held, WallClock())
    original = connection.rpc
    accepted = asyncio.Event()

    async def accepts_when_told(method, params):
        if method == "session.interrupt":
            await accepted.wait()
        return await original(method, params)

    connection.rpc = accepts_when_told
    stop = waiting = None
    try:
        stop = asyncio.create_task(interrupt(runtime, session, turn))
        await settle()
        waiting = send(runtime, session, "words after the cut")
        await settle()
        assert not waiting.done()
        assert store.chat.session(owner, session)["status"] == "streaming"

        accepted.set()
        await asyncio.wait_for(stop, 2)
        with pytest.raises(HubError) as refused:
            await asyncio.wait_for(waiting, 2)
        assert refused.value.code == "turn_outcome_unknown"
        assert len(connection.prompts()) == 1
    finally:
        # A failed example must not leave the interrupt held by the agent.
        accepted.set()
        await finish(runtime, [stop, waiting])


async def test_the_next_message_is_refused_when_the_one_before_it_ends_without_a_receipt(held):
    """A waiting message goes and its submit ends with no receipt, an outcome
    that publishes nothing: the one waiting behind it must learn of it."""
    now = [1001.0]
    runtime, _, _, connection, session, turn = await open_held_turn(held, now)
    first = second = None
    try:
        await interrupt(runtime, session, turn)
        first = send(runtime, session, "first synthetic words")
        await settle()
        second = send(runtime, session, "second synthetic words")
        await settle()
        original = connection.rpc

        async def unconfirmed(method, params):
            if method == "prompt.submit":
                # A real submit takes a moment: the next one is waiting by then.
                await asyncio.sleep(0.05)
                raise HubError("outcome_unknown", "Synthetic: no receipt")
            return await original(method, params)

        connection.rpc = unconfirmed
        await connection.end_held()
        with pytest.raises(HubError) as refused_first:
            await asyncio.wait_for(first, 2)
        assert refused_first.value.code == "outcome_unknown"
        with pytest.raises(HubError) as refused_second:
            await asyncio.wait_for(second, 2)
        assert refused_second.value.code == "turn_outcome_unknown"
    finally:
        await finish(runtime, [first, second])
