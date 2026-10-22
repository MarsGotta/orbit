"""Isolated Hermes adapter and durable hub session bindings."""

import asyncio
from uuid import uuid4

import pytest
from conftest import FakeConnection, device, only, said

from orbit_hub.assistant.backend import ConnectionLost, ResumedSession
from orbit_hub.assistant.hermes import HermesBackend, HermesConnection
from orbit_hub.protocol import HubError, InterruptParams, SendParams
from orbit_hub.runtime import DeviceRuntime


class ResumableConnection(FakeConnection):
    def __init__(self, callback, stored_sessions, epoch, adapter):
        super().__init__(callback)
        self.backend = adapter
        self.stored_sessions = stored_sessions
        self.epoch = epoch
        self.adapter = adapter
        self.runtime_to_stored = {}
        self.resumed_runtime_ids = set()
        self.last_resumed_id = None

    async def create_session(self):
        created = await super().create_session()
        self.stored_sessions[created.stored_id] = []
        self.runtime_to_stored[created.session_id] = created.stored_id
        return created

    async def rpc(self, method, params):
        if method == "session.resume":
            self.calls.append((method, params))
            if params["session_id"] not in self.stored_sessions:
                raise HubError(
                    "session_resume_not_found",
                    "Hermes no conserva la conversación guardada (código 4007)",
                )
            runtime_id = self.adapter.reuse_runtime_id or str(uuid4())
            self.live_sessions.add(runtime_id)
            self.runtime_to_stored[runtime_id] = params["session_id"]
            self.last_resumed_id = runtime_id
            self.adapter.resume_called.set()
            if self.adapter.resume_progress in {"complete", "failed"}:
                await self.callback(
                    {
                        "type": "session.resume_progress",
                        "session_id": runtime_id,
                        "payload": {
                            "phase": "history",
                            "status": self.adapter.resume_progress,
                            "message_count": self.adapter.resume_count,
                        },
                    }
                )
            return {
                "session_id": runtime_id,
                "resumed": params["session_id"],
                "message_count": self.adapter.resume_count,
                "hydrating": self.adapter.resume_hydrating,
            }
        if method == "prompt.submit":
            if self.adapter.prompt_not_found:
                self.calls.append((method, params))
                self.adapter.prompt_not_found = False
                raise HubError(
                    "session_runtime_not_found", "Hermes no encontró la sesión activa (4001)"
                )
            stored_id = self.runtime_to_stored[params["session_id"]]
            self.stored_sessions[stored_id].append(params["text"])
            if (
                self.adapter.defer_resume_failure
                and params["session_id"] in self.resumed_runtime_ids
            ):
                self.calls.append((method, params))
                return {"status": "streaming"}
        return await super().rpc(method, params)

    async def resume_session(self, stored_session_id):
        result = await HermesConnection.resume_session(self, stored_session_id)
        self.resumed_runtime_ids.add(result.session_id)
        return result


class ResumableAdapter:
    # Hermes's: it keeps each conversation and reloads it itself.
    capabilities = HermesBackend.capabilities

    def __init__(self):
        self.connections = []
        self.stored_sessions = {}
        self.defer_resume_failure = False
        self.resume_count = 17
        self.resume_hydrating = True
        self.resume_progress = "complete"
        self.resume_called = asyncio.Event()
        self.prompt_not_found = False
        self.reuse_runtime_id = None
        self.profile = "deimos"

    def connection(self, device, device_name, callback):
        connection = ResumableConnection(
            callback, self.stored_sessions, f"synthetic-epoch-{len(self.connections)}", self
        )
        self.connections.append(connection)
        return connection


async def sent(runtime, text, session_id=None):
    _, result = await runtime.handle(
        "chat.send", SendParams(text=text, session_id=session_id), str(uuid4())
    )
    return result["session_id"]


async def test_only_inflight_session_becomes_unknown_on_upstream_loss(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    active = await sent(runtime, "primer turno")
    idle = await sent(runtime, "conversación quieta")
    assert store.session(owner, active)["status"] == "completed"
    assert store.session(owner, idle)["status"] == "completed"
    store.status(active, "streaming")
    queue = runtime.subscribe()
    await runtime.on_event("default", ConnectionLost())
    errors = []
    while not queue.empty():
        frame = queue.get_nowait()
        if frame["event"] == "error":
            errors.append(frame)
    assert store.session(owner, active)["status"] == "outcome_unknown"
    assert store.session(owner, idle)["status"] == "completed"
    assert [frame["data"]["session_id"] for frame in errors] == [active]
    await runtime.close()


async def test_lost_hermes_keeps_reconnect_snapshot_for_unknown_turn(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "primer turno")
    store.status(session_id, "streaming")
    await runtime.on_event("default", ConnectionLost())

    viewer = runtime.subscribe()
    assert viewer.get_nowait()["data"]["sessions"] == [
        {
            "session_id": session_id,
            "turn_id": runtime.turn_ids[session_id],
            "status": "outcome_unknown",
            "pending": [],
        }
    ]
    await runtime.close()


async def test_new_hub_runtime_resumes_existing_hermes_history(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    stored_id = store.session(owner, session_id)["stored_id"]
    assert said(store, owner, session_id)[0]["text"] == "mensaje anterior"
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    assert await sent(restarted, "mensaje siguiente", session_id) == session_id
    assert store.session(owner, session_id)["stored_id"] == stored_id
    assert store.sessions_after(owner, None)[1] == 1
    assert len(adapter.stored_sessions) == 1
    assert len(adapter.stored_sessions[stored_id]) == 2
    assert "mensaje anterior" in adapter.stored_sessions[stored_id][0]
    assert "mensaje siguiente" in adapter.stored_sessions[stored_id][1]
    assert [method for method, _ in adapter.connections[1].calls] == [
        "session.resume",
        "prompt.submit",
    ]
    assert adapter.connections[1].calls[0][1]["session_id"] == stored_id
    assert adapter.connections[1].calls[0][1]["defer_history"] is True
    assert adapter.connections[1].calls[0][1]["omit_messages"] is False
    await restarted.close()


async def test_hermes_epoch_change_resumes_stored_session(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "mensaje anterior")
    old_binding = store.session(owner, session_id)
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())

    assert await sent(runtime, "mensaje tras reinicio", session_id) == session_id
    binding = store.session(owner, session_id)
    assert binding["epoch"] == "synthetic-epoch-1"
    assert binding["runtime_id"] != old_binding["runtime_id"]
    assert adapter.connections[1].calls[0][0] == "session.resume"
    assert adapter.connections[1].calls[0][1]["session_id"] == old_binding["stored_id"]
    assert len(adapter.stored_sessions[old_binding["stored_id"]]) == 2
    await runtime.close()


async def test_unknown_turn_is_never_resumed_or_resent(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "mensaje anterior")
    store.status(session_id, "outcome_unknown")
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()

    with pytest.raises(HubError) as error:
        await sent(runtime, "intento nuevo", session_id)
    assert error.value.code == "turn_outcome_unknown"
    assert len(adapter.connections) == 2
    assert adapter.connections[1].calls == []
    assert said(store, owner, session_id)[0]["text"] == "mensaje anterior"
    await runtime.close()


async def test_missing_hermes_history_has_a_specific_refusal(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()
    adapter.stored_sessions.clear()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    with pytest.raises(HubError) as error:
        await sent(restarted, "mensaje siguiente", session_id)
    assert error.value.code == "session_resume_not_found"
    assert "4007" in error.value.message
    assert [method for method, _ in adapter.connections[1].calls] == ["session.resume"]
    assert said(store, owner, session_id)[0]["text"] == "mensaje anterior"
    await restarted.close()


async def test_deferred_resume_history_failure_is_reported_and_next_turn_reopens(
    environment, monkeypatch
):
    from orbit_hub.assistant import hermes as hermes_module

    monkeypatch.setattr(
        hermes_module, "RESUME_HISTORY_ERROR_PREFIX", "Synthetic unreadable transcript."
    )
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.defer_resume_failure = True
    adapter.resume_hydrating = False
    adapter.resume_progress = None
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    viewer = restarted.subscribe()
    assert await sent(restarted, "mensaje sin historial", session_id) == session_id
    connection = adapter.connections[1]
    first_runtime_id = store.session(owner, session_id)["runtime_id"]
    assert store.session(owner, session_id)["status"] == "streaming"
    await connection.callback(
        {
            "type": "message.complete",
            "session_id": first_runtime_id,
            "payload": {
                "status": "error",
                "text": "Synthetic unreadable transcript.",
                "error": "Synthetic unreadable transcript.",
                "error_surface": {
                    "layer": "runtime",
                    "code": "agent_init_failed",
                    "retryable": True,
                },
            },
        }
    )
    frames = []
    while not viewer.empty():
        frames.append(viewer.get_nowait())
    error = next(frame for frame in frames if frame["event"] == "error")
    assert error["data"]["code"] == "session_resume_history_unreadable"
    assert "17 mensajes" in error["data"]["message"]
    assert "historial" in error["data"]["message"]
    assert "Could not reopen" not in str(frames)
    assert store.session(owner, session_id)["status"] == "failed"
    assert first_runtime_id not in connection.live_sessions
    assert [method for method, _ in connection.calls] == ["session.resume", "prompt.submit"]

    assert await sent(restarted, "mensaje nuevo", session_id) == session_id
    assert [method for method, _ in connection.calls] == [
        "session.resume",
        "prompt.submit",
        "session.resume",
        "prompt.submit",
    ]
    stored_id = store.session(owner, session_id)["stored_id"]
    assert (
        len(
            [text for text in adapter.stored_sessions[stored_id] if "mensaje sin historial" in text]
        )
        == 1
    )
    await restarted.close()


async def test_deferred_history_failure_refuses_before_prompt_and_retries_next_send(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_progress = "failed"
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    with pytest.raises(HubError) as error:
        await sent(restarted, "mensaje rechazado", session_id)
    assert error.value.code == "session_resume_history_unreadable"
    assert "17 mensajes" in error.value.message
    assert "Could not reopen" not in error.value.message
    connection = adapter.connections[1]
    runtime_id = connection.last_resumed_id
    assert runtime_id not in connection.live_sessions
    assert [method for method, _ in connection.calls] == ["session.resume"]
    assert store.session(owner, session_id)["status"] == "completed"
    assert len(said(store, owner, session_id)) == 2
    await connection.callback(
        {
            "type": "error",
            "session_id": runtime_id,
            "payload": {
                "message": "Could not reopen that session (its transcript could not be read). Details: synthetic"
            },
        }
    )
    assert restarted.reconciliations == {}

    adapter.resume_progress = "complete"
    assert await sent(restarted, "mensaje nuevo", session_id) == session_id
    assert [method for method, _ in connection.calls] == [
        "session.resume",
        "session.resume",
        "prompt.submit",
    ]
    await restarted.close()


async def test_late_history_error_does_not_reconcile_completed_turn(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    old_turn_id = runtime.turn_ids[session_id]
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_progress = "failed"
    viewer = runtime.subscribe()
    viewer.get_nowait()

    with pytest.raises(HubError) as error:
        await sent(runtime, "mensaje rechazado", session_id)
    assert error.value.code == "session_resume_history_unreadable"
    connection = adapter.connections[1]
    await connection.callback(
        {
            "type": "error",
            "session_id": connection.last_resumed_id,
            "payload": {
                "message": "Could not reopen that session (its transcript could not be read). Details: synthetic"
            },
        }
    )
    assert runtime.reconciliations == {}
    assert store.session(owner, session_id)["status"] == "completed"
    frames = []
    while not viewer.empty():
        frames.append(viewer.get_nowait())
    assert not any(
        frame["event"] == "chat.status" and frame["id"] == old_turn_id for frame in frames
    )
    await runtime.close()


async def test_interrupt_and_new_send_do_not_wait_for_history(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    old_turn_id = runtime.turn_ids[session_id]
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_progress = None
    sending = asyncio.create_task(sent(runtime, "mensaje tras reconexión", session_id))
    await adapter.resume_called.wait()
    connection = adapter.connections[1]
    interrupt = asyncio.create_task(
        runtime.handle(
            "chat.interrupt",
            InterruptParams(session_id=session_id, turn_id=old_turn_id),
            str(uuid4()),
        )
    )
    duplicate = asyncio.create_task(sent(runtime, "segundo intento", session_id))
    try:
        await asyncio.sleep(0)
        assert interrupt.done(), "the control waited behind the history load"
        assert duplicate.done(), "another send was left waiting on the same runtime_id"
        with pytest.raises(HubError) as error:
            await interrupt
        assert error.value.code == "stale_turn"
        with pytest.raises(HubError) as error:
            await duplicate
        assert error.value.code == "session_resume_in_progress"
        assert [method for method, _ in connection.calls] == ["session.resume"]
    finally:
        for task in (interrupt, duplicate):
            if not task.done():
                task.cancel()
        await asyncio.gather(interrupt, duplicate, return_exceptions=True)
        await connection.callback(
            {
                "type": "session.resume_progress",
                "session_id": connection.last_resumed_id,
                "payload": {"phase": "history", "status": "complete", "message_count": 17},
            }
        )
        await sending
        await runtime.close()


async def test_other_new_session_can_send_while_history_waits(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_progress = None
    sending = asyncio.create_task(sent(runtime, "mensaje tras reconexión", session_id))
    await adapter.resume_called.wait()
    connection = adapter.connections[1]
    other_send = asyncio.create_task(sent(runtime, "otra conversación"))
    try:
        await asyncio.sleep(0)
        assert other_send.done(), "the new session waited behind the history load"
        other = await other_send
        assert other != session_id
        assert [method for method, _ in connection.calls] == [
            "session.resume",
            "prompt.submit",
        ]
        assert store.session(owner, other)["status"] == "completed"
    finally:
        if not other_send.done():
            other_send.cancel()
            await asyncio.gather(other_send, return_exceptions=True)
        await connection.callback(
            {
                "type": "session.resume_progress",
                "session_id": connection.last_resumed_id,
                "payload": {"phase": "history", "status": "complete", "message_count": 17},
            }
        )
        await sending
        await runtime.close()


async def test_live_session_sends_do_not_claim_history_reservation_under_lock_contention(
    environment,
):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    await runtime.lock.acquire()
    first = asyncio.create_task(sent(runtime, "primer envío", session_id))
    second = asyncio.create_task(sent(runtime, "segundo envío", session_id))
    await asyncio.sleep(0)
    runtime.lock.release()
    try:
        assert await first == session_id
        assert await second == session_id
        assert [method for method, _ in adapter.connections[0].calls].count("session.resume") == 0
    finally:
        for task in (first, second):
            if not task.done():
                task.cancel()
        await asyncio.gather(first, second, return_exceptions=True)
        await runtime.close()


@pytest.mark.parametrize(("hydrating", "progress"), [(False, None), (True, "complete")])
async def test_other_disconnected_session_can_resume_without_pending_hydration(
    environment, hydrating, progress
):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    first_id = await sent(runtime, "primera conversación")
    second_id = await sent(runtime, "segunda conversación")
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_hydrating = hydrating
    adapter.resume_progress = progress
    await runtime.lock.acquire()
    first = asyncio.create_task(sent(runtime, "mensaje uno", first_id))
    second = asyncio.create_task(sent(runtime, "mensaje dos", second_id))
    await asyncio.sleep(0)
    runtime.lock.release()
    try:
        assert await first == first_id
        assert await second == second_id
        assert [method for method, _ in adapter.connections[1].calls] == [
            "session.resume",
            "prompt.submit",
            "session.resume",
            "prompt.submit",
        ]
    finally:
        for task in (first, second):
            if not task.done():
                task.cancel()
        await asyncio.gather(first, second, return_exceptions=True)
        await runtime.close()


async def test_other_disconnected_session_refusal_names_remaining_history_time(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    first_id = await sent(runtime, "primera conversación")
    second_id = await sent(runtime, "segunda conversación")
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_progress = None
    sending = asyncio.create_task(sent(runtime, "mensaje uno", first_id))
    await adapter.resume_called.wait()
    connection = adapter.connections[1]
    try:
        with pytest.raises(HubError) as error:
            await sent(runtime, "mensaje dos", second_id)
        assert error.value.code == "session_resume_in_progress"
        assert "otro historial" in error.value.message
        remaining = int(error.value.message.split("quedan hasta ")[1].split(" s")[0])
        assert 0 <= remaining <= 300
        assert "plazo de 300 s" in error.value.message
        assert [method for method, _ in connection.calls] == ["session.resume"]
    finally:
        await connection.callback(
            {
                "type": "session.resume_progress",
                "session_id": connection.last_resumed_id,
                "payload": {"phase": "history", "status": "complete", "message_count": 17},
            }
        )
        await sending
        await runtime.close()


@pytest.mark.parametrize("ending", ["close", "revoke", "unpair"])
async def test_ending_runtime_wakes_history_wait_with_its_cause(environment, ending):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    adapter.resume_progress = None
    sending = asyncio.create_task(sent(runtime, "mensaje tras reconexión", session_id))
    await adapter.resume_called.wait()
    await asyncio.sleep(0)
    assert runtime.resume_waiter is not None
    connection = adapter.connections[1]
    try:
        if ending == "unpair":
            store.revoke(owner)
            await runtime.revoke()
        else:
            await getattr(runtime, ending)()
        await asyncio.sleep(0)
        assert sending.done(), f"{ending} left the history load pending"
        with pytest.raises(HubError) as error:
            await sending
        assert error.value.code == ("session_resume_closed" if ending == "close" else "revoked")
        assert [method for method, _ in connection.calls if method == "prompt.submit"] == []
    finally:
        if not sending.done():
            sending.cancel()
        await asyncio.gather(sending, return_exceptions=True)
        await runtime.close()


async def test_reused_live_runtime_keeps_early_history_completion(environment, monkeypatch):
    from orbit_hub import runtime as runtime_module

    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "turno anterior")
    adapter.reuse_runtime_id = store.session(owner, session_id)["runtime_id"]
    adapter.connections[0].connected = False
    adapter.connections[0].live_sessions.clear()
    await runtime.on_event("default", ConnectionLost())
    monkeypatch.setattr(runtime_module, "RESUME_HISTORY_TIMEOUT_SECONDS", 0.001)
    assert await sent(runtime, "mensaje nuevo", session_id) == session_id
    assert [method for method, _ in adapter.connections[1].calls] == [
        "session.resume",
        "prompt.submit",
    ]
    await runtime.close()


async def test_deferred_history_completes_before_prompt(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_progress = None
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    sending = asyncio.create_task(sent(restarted, "mensaje nuevo", session_id))
    await adapter.resume_called.wait()
    connection = adapter.connections[1]
    assert [method for method, _ in connection.calls] == ["session.resume"]
    runtime_id = connection.last_resumed_id
    await connection.callback(
        {
            "type": "session.resume_progress",
            "session_id": runtime_id,
            "payload": {"phase": "history", "status": "complete", "message_count": 17},
        }
    )
    assert await sending == session_id
    assert [method for method, _ in connection.calls] == ["session.resume", "prompt.submit"]
    await restarted.close()


async def test_deferred_history_error_event_refuses_without_progress(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_progress = None
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    sending = asyncio.create_task(sent(restarted, "mensaje rechazado", session_id))
    await adapter.resume_called.wait()
    connection = adapter.connections[1]
    await connection.callback(
        {
            "type": "error",
            "session_id": connection.last_resumed_id,
            "payload": {
                "message": "Could not reopen that session (its transcript could not be read). Details: synthetic"
            },
        }
    )
    with pytest.raises(HubError) as error:
        await sending
    assert error.value.code == "session_resume_history_unreadable"
    assert [method for method, _ in connection.calls] == ["session.resume"]
    assert connection.last_resumed_id not in connection.live_sessions
    await restarted.close()


async def test_resume_without_hydration_signal_refuses_before_prompt(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_hydrating = None
    adapter.resume_progress = None
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    with pytest.raises(HubError) as error:
        await sent(restarted, "mensaje rechazado", session_id)
    assert error.value.code == "session_resume_progress_unavailable"
    connection = adapter.connections[1]
    assert [method for method, _ in connection.calls] == ["session.resume"]
    assert connection.last_resumed_id not in connection.live_sessions
    await restarted.close()


async def test_deferred_history_timeout_refuses_before_prompt(environment, monkeypatch):
    from orbit_hub import runtime as runtime_module

    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_progress = None
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    monkeypatch.setattr(runtime_module, "RESUME_HISTORY_TIMEOUT_SECONDS", 0.001, raising=False)
    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    with pytest.raises(HubError) as error:
        await sent(restarted, "mensaje sin fase", session_id)
    assert error.value.code == "session_resume_history_timeout"
    assert "1 ms" in error.value.message
    connection = adapter.connections[1]
    runtime_id = store.session(owner, session_id)["runtime_id"]
    assert runtime_id not in connection.live_sessions
    assert [method for method, _ in connection.calls] == ["session.resume"]
    assert store.session(owner, session_id)["status"] == "completed"
    await restarted.close()


async def test_rejected_prompt_4001_is_not_outcome_unknown(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    adapter.prompt_not_found = True
    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    with pytest.raises(HubError) as error:
        await sent(restarted, "mensaje rechazado", session_id)
    assert error.value.code == "session_runtime_not_found"
    connection = adapter.connections[1]
    assert store.session(owner, session_id)["status"] == "failed"
    assert store.session(owner, session_id)["runtime_id"] not in connection.live_sessions
    assert [method for method, _ in connection.calls] == ["session.resume", "prompt.submit"]
    assert await sent(restarted, "mensaje siguiente", session_id) == session_id
    assert [method for method, _ in connection.calls] == [
        "session.resume",
        "prompt.submit",
        "session.resume",
        "prompt.submit",
    ]
    await restarted.close()


async def test_unknown_resume_message_count_is_not_reported_as_zero(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.resume_count = 0
    adapter.resume_hydrating = False
    adapter.resume_progress = None
    adapter.defer_resume_failure = True
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()
    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    await sent(restarted, "mensaje nuevo", session_id)
    viewer = restarted.subscribe()
    viewer.get_nowait()
    connection = adapter.connections[1]
    await connection.callback(
        {
            "type": "message.complete",
            "session_id": store.session(owner, session_id)["runtime_id"],
            "payload": {
                "status": "error",
                "error": "Could not reopen that session (its transcript could not be read).",
                "error_surface": {"code": "agent_init_failed"},
            },
        }
    )
    frames = []
    while not viewer.empty():
        frames.append(viewer.get_nowait())
    error = next(frame for frame in frames if frame["event"] == "error")
    assert error["data"]["code"] == "session_resume_history_unreadable"
    assert "0 mensajes" not in error["data"]["message"]
    await restarted.close()


async def test_other_agent_init_failure_is_not_called_unreadable_history(environment):
    _, store, _ = environment
    adapter = ResumableAdapter()
    adapter.defer_resume_failure = True
    owner = device(store)["device_id"]
    first = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(first, "mensaje anterior")
    await first.close()

    restarted = DeviceRuntime(owner, store, only(adapter), lambda: 1002)
    assert await sent(restarted, "mensaje nuevo", session_id) == session_id
    connection = adapter.connections[1]
    runtime_id = store.session(owner, session_id)["runtime_id"]
    viewer = restarted.subscribe()
    viewer.get_nowait()
    await connection.callback(
        {
            "type": "message.complete",
            "session_id": runtime_id,
            "payload": {
                "status": "error",
                "error": "No LLM provider configured",
                "text": "No LLM provider configured",
                "error_surface": {"layer": "runtime", "code": "agent_init_failed"},
            },
        }
    )
    frames = []
    while not viewer.empty():
        frames.append(viewer.get_nowait())
    assert not any(
        frame["event"] == "error" and frame["data"]["code"] == "session_resume_history_unreadable"
        for frame in frames
    )
    assert runtime_id in connection.live_sessions
    await restarted.close()


async def test_agent_init_failure_on_new_session_is_not_resume_history_error(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = await sent(runtime, "mensaje nuevo")
    binding = store.session(owner, session_id)
    viewer = runtime.subscribe()
    viewer.get_nowait()
    store.status(session_id, "streaming")
    await runtime.connections["default"].callback(
        {
            "type": "message.complete",
            "session_id": binding["runtime_id"],
            "payload": {
                "status": "error",
                "error": "Could not reopen that session (its transcript could not be read).",
                "error_surface": {"code": "agent_init_failed"},
            },
        }
    )
    frames = []
    while not viewer.empty():
        frames.append(viewer.get_nowait())
    assert not any(
        frame["event"] == "error"
        and frame["data"].get("code") == "session_resume_history_unreadable"
        for frame in frames
    )
    assert binding["runtime_id"] in runtime.connections["default"].live_sessions
    await runtime.close()


async def test_replaced_resume_connection_has_its_own_cause(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    connection = await runtime.connect("default")
    session_id = store.create_session(owner, "default", "old-runtime", "stored", "old-epoch", 1001)

    async def replace_connection(_stored_id):
        runtime.connections["default"] = object()
        return ResumedSession("new-runtime", "stored", None, None)

    connection.resume_session = replace_connection
    with pytest.raises(HubError) as error:
        await runtime.resume_if_disconnected(session_id, "default", connection)
    assert error.value.code == "session_resume_connection_replaced"
    assert store.session(owner, session_id)["runtime_id"] == "old-runtime"
    await connection.close()


async def test_disconnected_session_names_its_retry(environment):
    _, store, adapter = environment
    owner = device(store)["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    session_id = store.create_session(owner, "default", "old-runtime", "stored", "old-epoch", 1001)
    with pytest.raises(HubError) as error:
        runtime.require_live(session_id)
    assert error.value.code == "session_disconnected"
    assert "se intentará reanudar" in error.value.message


async def test_foreign_device_does_not_connect_or_resume(environment):
    _, store, adapter = environment
    owner = device(store, "propietaria")["device_id"]
    other = device(store, "otro dispositivo")["device_id"]
    session_id = store.create_session(owner, "default", "old-runtime", "stored", "old-epoch", 1001)
    runtime = DeviceRuntime(other, store, only(adapter), lambda: 1001)
    with pytest.raises(HubError) as error:
        await sent(runtime, "intento ajeno", session_id)
    assert error.value.code == "session_not_found"
    assert adapter.connections == []
    assert store.session(owner, session_id)["status"] == "idle"
