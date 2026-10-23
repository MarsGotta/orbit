"""Reopening a conversation whose live session is gone, and loading its history.

A conversation's next turn reopens it on its agent (`resume_session`) when its
live session is gone: the agent's connection changed, or the agent gave the
session up. A backend that reloads the history itself may say the load is
still under way (`loading_history`); the turn then waits for `HistoryLoaded`
(up to `RESUME_HISTORY_TIMEOUT_SECONDS`), outside its conversation's
admission, so controls still answer. While one history load waits, another
conversation of the device that would have to reopen is refused with the time
left; a live one is not.

Every reopen and every wait is the conversation's own: two conversations may
reopen at once, each keyed by its agent and live id (`LiveKey`).
"""

import asyncio
import math

from ..assistant.backend import StoredConversation, StoredMessage
from ..protocol import HubError
from ..storage import SESSION_MESSAGES_PAGE_LIMIT

# Bound a missing hydration event while the session runtime ID is reserved.
# Hermes waits up to 300 s for resume_history_ready (server.py:_await_resume_history),
# so this matches its own bound, not a model-turn or transcript-size ceiling.
RESUME_HISTORY_TIMEOUT_SECONDS = 300.0

# A live session as the runtime knows it: the agent whose connection holds it
# and its id there. Ids are only unique within one agent's connection.
LiveKey = tuple[str, str]

_NOT_SENT = "no se envió el mensaje"


class Resumes:
    def __init__(self, runtime):
        self.runtime = runtime
        # The turns waiting for their history, by live session.
        self.waiters: dict[LiveKey, asyncio.Future[str]] = {}
        # Progress that arrived while its reopen was still being answered.
        self.early: dict[LiveKey, str] = {}
        # The reopens being answered, by agent.
        self.reopening: dict[str, int] = {}
        # The conversations whose turn waits for its history: its agent and
        # the loop time its wait ends.
        self.loading: dict[str, tuple[str, float]] = {}

    @property
    def waiter(self) -> tuple[LiveKey, asyncio.Future[str]] | None:
        return next(iter(self.waiters.items()), None)

    def record_progress(self, key: LiveKey, status: str) -> bool:
        """Hand a history load's outcome to the turn waiting for it; True when
        it belongs to a reopen of this device."""
        if not isinstance(key[1], str):
            return False
        if (future := self.waiters.get(key)) is not None:
            if not future.done():
                future.set_result(status)
            return True
        if self.reopening.get(key[0]):
            self.early[key] = status
            return True
        return False

    def connection_lost(self, agent_id: str) -> None:
        for (agent, _), future in self.waiters.items():
            if agent == agent_id and not future.done():
                future.set_result("lost")
        self.early = {key: status for key, status in self.early.items() if key[0] != agent_id}

    def end(self, status: str) -> None:
        """The runtime is closing or revoked: every wait ends with `status`."""
        for future in self.waiters.values():
            if not future.done():
                future.set_result(status)

    def refuse_while_loading(self, session_id: str | None, agent_id: str) -> None:
        """A second turn of a conversation whose history still loads."""
        if session_id in self.loading:
            raise HubError(
                "session_resume_in_progress",
                f"{self.runtime.agent_name(agent_id, starts=True)} sigue cargando el historial "
                f"de esta conversación; espera: quedan hasta {self._left(session_id)} s del "
                f"plazo de {RESUME_HISTORY_TIMEOUT_SECONDS:g} s",
            )

    def refuse_other_reopen(self, session_id: str | None, connection) -> None:
        """A conversation that would have to reopen while another one loads."""
        if not self.loading or session_id is None:
            return
        existing = self.runtime.store.chat.session(self.runtime.device, session_id)
        if (
            existing["epoch"] == connection.epoch
            and existing["runtime_id"] in connection.live_sessions
        ):
            return
        loading = next(iter(self.loading))
        raise HubError(
            "session_resume_in_progress",
            f"{self.runtime.agent_name(self.loading[loading][0], starts=True)} ya carga otro "
            f"historial en este dispositivo; quedan hasta {self._left(loading)} s del plazo de "
            f"{RESUME_HISTORY_TIMEOUT_SECONDS:g} s",
        )

    def reserve(self, session_id: str, agent_id: str) -> None:
        deadline = asyncio.get_running_loop().time() + RESUME_HISTORY_TIMEOUT_SECONDS
        self.loading[session_id] = (agent_id, deadline)

    def release(self, session_id: str) -> None:
        self.loading.pop(session_id, None)

    def must_wait(self, agent_id: str, binding: dict, loading_history: bool | None) -> bool:
        """Whether the turn waits for its history outside its admission."""
        return loading_history is True and (agent_id, binding["runtime_id"]) not in self.early

    def _left(self, session_id: str) -> int:
        deadline = self.loading[session_id][1]
        return math.ceil(max(0.0, deadline - asyncio.get_running_loop().time()))

    async def reopen(self, session_id: str, agent_id: str, connection) -> tuple[dict, bool | None]:
        """The conversation's binding, reopened on `connection` unless it is
        live there, and whether its history still loads (False: live)."""
        runtime = self.runtime
        binding = runtime.store.chat.session(runtime.device, session_id)
        if (
            binding["epoch"] == connection.epoch
            and binding["runtime_id"] in connection.live_sessions
        ):
            return binding, False
        history = await self._history(session_id, agent_id)
        agent = runtime.agents.get(agent_id)
        self.reopening[agent_id] = self.reopening.get(agent_id, 0) + 1
        resumed = None
        try:
            resumed = await agent.integration.start(
                agent_id,
                agent.name,
                lambda: (
                    connection.resume_session(binding["stored_id"])
                    if history is None
                    else connection.resume_session(binding["stored_id"], history)
                ),
                round(runtime.now() * 1000),
                reopen=True,
            )
        finally:
            self._settle(agent_id, None if resumed is None else (agent_id, resumed.session_id))
        self._check_connection(agent_id, connection, "durante la reanudación")
        runtime.store.chat.resume_session(
            runtime.device, session_id, resumed.session_id, resumed.stored_id, connection.epoch
        )
        runtime.bindings.pop((agent_id, binding["runtime_id"]), None)
        runtime.resumed_message_counts.pop((agent_id, binding["runtime_id"]), None)
        runtime.bindings[agent_id, resumed.session_id] = session_id
        runtime.resumed_message_counts[agent_id, resumed.session_id] = resumed.message_count
        return runtime.store.chat.session(runtime.device, session_id), resumed.loading_history

    async def _history(self, session_id: str, agent_id: str) -> StoredConversation | None:
        """For a backend that keeps no conversation, the one the hub stores: its
        newest page, as the Mac reads it, read before this turn's own message."""
        runtime = self.runtime
        if runtime.agents.get(agent_id).backend.capabilities.history:
            return None
        page = await runtime.run_store(
            lambda: runtime.store.chat.session_messages(
                runtime.device, session_id, None, SESSION_MESSAGES_PAGE_LIMIT
            )
        )
        return StoredConversation(
            tuple(StoredMessage(message["role"], message["text"]) for message in page["messages"]),
            page["total"],
        )

    def _settle(self, agent_id: str, keep: LiveKey | None) -> None:
        """One reopen of the agent was answered; progress no reopen can claim
        any more is dropped."""
        self.reopening[agent_id] -= 1
        if self.reopening[agent_id]:
            return
        del self.reopening[agent_id]
        self.early = {
            key: status for key, status in self.early.items() if key[0] != agent_id or key == keep
        }

    def _check_connection(self, agent_id: str, connection, when: str) -> None:
        name = self.runtime.agent_name(agent_id)
        if self.runtime.connections.get(agent_id) is not connection:
            raise HubError(
                "session_resume_connection_replaced",
                f"La conexión con {name} cambió {when}; {_NOT_SENT}",
            )
        if not connection.connected:
            raise HubError(
                "session_resume_connection_lost",
                f"Se perdió la conexión con {name} {when}; {_NOT_SENT}",
            )

    async def wait_history(self, agent_id: str, binding: dict, connection, loading) -> None:
        """Wait, when the agent said its history still loads, for it to load;
        refused with the cause when it cannot, the live session given up."""
        runtime = self.runtime
        key = (agent_id, binding["runtime_id"])
        try:
            self._still_open()
            status = self.early.pop(key, None)
            if status is None:
                if loading is False:
                    return
                if loading is not True:
                    raise HubError(
                        "session_resume_progress_unavailable",
                        f"{runtime.agent_name(agent_id, starts=True)} no confirmó si terminó de "
                        f"cargar el historial; {_NOT_SENT}",
                    )
                status = await self._wait(key, agent_id)
            self._loaded(status, agent_id, connection, key)
        except HubError:
            connection.live_sessions.discard(binding["runtime_id"])
            runtime.resumed_message_counts.pop(key, None)
            raise

    def _still_open(self) -> None:
        runtime = self.runtime
        if runtime.revoked or not runtime.store.devices.is_active(runtime.device):
            raise HubError("revoked", f"Dispositivo revocado; {_NOT_SENT}")
        if runtime.closed:
            raise HubError("session_resume_closed", f"Se cerró la sesión de Orbit; {_NOT_SENT}")

    async def _wait(self, key: LiveKey, agent_id: str) -> str:
        future: asyncio.Future[str] = asyncio.get_running_loop().create_future()
        self.waiters[key] = future
        try:
            return await asyncio.wait_for(future, RESUME_HISTORY_TIMEOUT_SECONDS)
        except TimeoutError as exc:
            milliseconds = int(RESUME_HISTORY_TIMEOUT_SECONDS * 1000)
            raise HubError(
                "session_resume_history_timeout",
                f"{self.runtime.agent_name(agent_id, starts=True)} no confirmó la carga del "
                f"historial en {milliseconds} ms; {_NOT_SENT}",
            ) from exc
        finally:
            if self.waiters.get(key) is future:
                del self.waiters[key]

    def _loaded(self, status: str, agent_id: str, connection, key: LiveKey) -> None:
        runtime = self.runtime
        if status == "revoked":
            raise HubError("revoked", f"Dispositivo revocado; {_NOT_SENT}")
        if status == "closed":
            raise HubError("session_resume_closed", f"Se cerró la sesión de Orbit; {_NOT_SENT}")
        if (
            runtime.connections.get(agent_id) is not connection
            or not connection.connected
            or status == "lost"
        ):
            raise HubError(
                "session_resume_connection_lost",
                f"Se perdió la conexión con {runtime.agent_name(agent_id)} durante la carga "
                f"del historial; {_NOT_SENT}",
            )
        if status == "failed":
            count = runtime.resumed_message_counts.pop(key, None)
            count_text = f" ({count} mensajes en el último tramo)" if count else ""
            raise HubError(
                "session_resume_history_unreadable",
                f"{runtime.agent_name(agent_id, starts=True)} no pudo leer el historial "
                f"guardado{count_text}; {_NOT_SENT}. Puedes enviar uno nuevo para volver a "
                "reanudar",
            )
