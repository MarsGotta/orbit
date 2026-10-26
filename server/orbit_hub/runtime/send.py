"""One turn of a conversation, from `chat.send` to the agent's prompt.

A send is checked (its meeting, its handover, its files, its agent), admitted
under its own conversation's admission (`Conversations.admitting`), where the
agent's connection opens, its session opens or reopens and the turn commits;
then the prompt goes to the agent outside every lock. A history the agent
still loads is waited for outside the admission (`resume.Resumes`).
"""

import asyncio

from ..assistant import handover
from ..assistant.backend import ImageBlock, TextBlock
from ..attachments import Stored, turn_parts
from ..log import report
from ..meeting_context import MeetingTurn, meeting_context, read_meeting
from ..protocol import HubError, emit
from ..storage import SESSION_MESSAGES_PAGE_LIMIT
from ..turns import TurnState, advance, can_start_turn
from .work import TurnWork

_UNKNOWN_OUTCOME = (
    "El resultado del turno anterior es incierto; abre una sesión nueva "
    "(session.new) o espera la reconciliación"
)
# What a voice note's turn tells the agent, and never shows the user.
_READ_ALOUD = "[Orbit leerá tu respuesta en voz alta: contesta solo con texto, sin generar audio]"


class TurnSender:
    def __init__(self, runtime):
        self.runtime = runtime

    async def send(self, params, request_id: str) -> tuple[str, dict]:
        """Submit one chat turn on an explicit state machine.

        The conversation's admission covers admission and, without a history
        to wait for, commitment; its lock covers commitment; neither covers
        history hydration or the prompt. A reservation prevents another send
        to the same conversation while hydration is pending; controls can
        still answer.
        """
        runtime = self.runtime
        meeting = await self._meeting_turn(params) if params.meeting is not None else None
        if params.handover_from is not None:
            meeting = await self._handover_turn(params)
        # The files the turn names, refused before anything reaches an agent
        # when one is not this device's, was sent already, or crosses a ceiling.
        attached: list[Stored] = (
            await runtime.run_store(
                lambda: runtime.store.attachments.admit(
                    runtime.device, params.session_id, params.attachments
                )
            )
            if params.attachments
            else []
        )
        async with runtime.conversations.admitting(params.session_id):
            admitted = await self._admit(params)
            session_id, agent_id, connection, binding, loading = admitted
            waits = runtime.resumes.must_wait(agent_id, binding, loading)
            if waits:
                runtime.resumes.reserve(session_id, agent_id)
            else:
                await runtime.resumes.wait_history(agent_id, binding, connection, loading)
                upstream_text = await self._commit(params, request_id, admitted, meeting)
        if waits:
            try:
                await runtime.resumes.wait_history(agent_id, binding, connection, loading)
                upstream_text = await self._commit(params, request_id, admitted, meeting)
            finally:
                runtime.resumes.release(session_id)
        # A failure here fails the turn itself: nothing reached the agent.
        blocks = await self._blocks(session_id, agent_id, upstream_text, attached)
        await self._prompt(session_id, agent_id, connection, binding, blocks)
        async with runtime.conversations.committing(session_id):
            runtime.apply_state(session_id, TurnState.STREAMING)
            # An interrupt that landed while the prompt waited already moved
            # the persisted state to interrupting; apply_state skips the
            # forbidden hop back, and this reply must not claim it either.
            status = runtime.turn_state(session_id).value
        return "chat.session", {"session_id": session_id, "status": status}

    async def _admit(self, params) -> tuple:
        """Under the conversation's admission: the agent, its connection and
        the conversation's binding there, opened or reopened."""
        runtime = self.runtime
        if not runtime.store.devices.is_active(runtime.device):
            raise HubError("revoked", "Dispositivo revocado")
        session_id = params.session_id
        session = (
            None if session_id is None else runtime.store.chat.session(runtime.device, session_id)
        )
        agent_id = self._turn_agent(params, session)
        runtime.resumes.refuse_while_loading(session_id, agent_id)
        connection = await runtime.connect(agent_id)
        runtime.resumes.refuse_other_reopen(session_id, connection)
        if session_id is None:
            session_id = await self._open_session(params, agent_id, connection)
        runtime.expire_stale_interrupt(session_id)
        self._startable(session_id)
        binding, loading = await runtime.resumes.reopen(session_id, agent_id, connection)
        return session_id, agent_id, connection, binding, loading

    async def _open_session(self, params, agent_id: str, connection) -> str:
        runtime = self.runtime
        agent = runtime.agents.get(agent_id)
        created = await agent.integration.start(
            agent_id, agent.name, connection.create_session, round(runtime.now() * 1000)
        )
        if runtime.revoked or not runtime.store.devices.is_active(runtime.device):
            raise HubError("revoked", "Dispositivo revocado")
        session_id = runtime.store.chat.create_session(
            runtime.device,
            agent_id,
            created.session_id,
            created.stored_id,
            connection.epoch,
            runtime.now(),
        )
        runtime.bindings[agent_id, created.session_id] = session_id
        if params.handover_from is not None:
            source = runtime.store.chat.session(runtime.device, params.handover_from)
            handover.record(
                runtime.store, session_id, params.handover_from, runtime.agents.owner(source)
            )
        return session_id

    def _turn_agent(self, params, session: dict | None) -> str:
        """The agent a turn goes to: the one it names, or else its session's,
        or else the default; refused before anything reaches an agent."""
        agents = self.runtime.agents
        # A Mac remembers the id it was told, `default` included.
        asked = None if params.agent_id is None else agents.known(params.agent_id)
        if asked is not None:
            named = agents.get(asked)
            if session is None:
                return named.id
        if session is None:
            # The primary: the agent that answers when none is named.
            return agents.primary_id
        owner = agents.owner(session)
        if asked is not None and asked != owner:
            # A conversation is one agent's: its history lives with that agent.
            owner_name = agents.by_id[owner].name if owner in agents.by_id else owner
            raise HubError(
                "agent_mismatch",
                f"Esta conversación es de {owner_name} («{owner}»), no de «{params.agent_id}»; "
                "abre una conversación nueva para hablar con otro agente",
                numbers={"session_agent_id": owner, "agent_id": params.agent_id},
            )
        # A conversation whose agent has left the configuration.
        return agents.get(owner).id

    async def _commit(self, params, request_id: str, admitted: tuple, meeting) -> str:
        """Commit the turn under its conversation's lock; the text for the agent."""
        session_id, agent_id, connection, binding, _ = admitted
        runtime = self.runtime
        async with runtime.conversations.committing(session_id):
            if runtime.revoked or not runtime.store.devices.is_active(runtime.device):
                raise HubError("revoked", "Dispositivo revocado")
            if runtime.closed:
                raise HubError(
                    "session_resume_closed", "Se cerró la sesión de Orbit; no se envió el mensaje"
                )
            if (
                runtime.connections.get(agent_id) is not connection
                or not connection.connected
                or binding["epoch"] != connection.epoch
                or binding["runtime_id"] not in connection.live_sessions
            ):
                raise HubError(
                    "session_resume_connection_lost",
                    f"Se perdió la conexión con {runtime.agent_name(agent_id)} antes de enviar "
                    "el mensaje; no se envió",
                )
            runtime.expire_stale_interrupt(session_id)
            state = self._startable(session_id)
            return await self._commit_turn(params, request_id, session_id, agent_id, state, meeting)

    def _startable(self, session_id: str) -> TurnState:
        """The conversation's state, refused when no turn may start in it: one
        is under way, or one this run of the hub sent has an unknown outcome
        (never resent over: it may already have reached the agent). An unknown
        outcome this run never recorded was cut by a restart (`turns`)."""
        runtime = self.runtime
        state = runtime.turn_state(session_id)
        if state is TurnState.OUTCOME_UNKNOWN:
            if session_id in runtime.turn_ids:
                raise HubError("turn_outcome_unknown", _UNKNOWN_OUTCOME)
            return state
        if not can_start_turn(state):
            raise HubError("session_busy", "Espera o interrumpe el turno en curso")
        return state

    async def _commit_turn(self, params, request_id, session_id, agent_id, state, meeting) -> str:
        runtime = self.runtime
        tracked_input_type = runtime.input_tracker.consume(runtime.device, params.text)
        input_type = "llamada de voz en curso" if params.call_active else tracked_input_type
        sent = meeting.text if meeting is not None else params.text
        header = f"[Orbit · {runtime.device_name} · {input_type}]"
        if params.voice is not None and meeting is None:
            # Orbit reads the answer to a voice note aloud: audio the agent
            # made itself would be a file on the server Orbit never plays.
            header += "\n" + _READ_ALOUD
        upstream_text = f"{header}\n{sent}"
        runtime.events.previews.pop(session_id, None)
        runtime.events.retries.pop(session_id, None)
        runtime.turn_ids[session_id] = request_id
        now_ms = round(runtime.now() * 1000)
        runtime.work[session_id] = TurnWork(now_ms)
        # The conversation keeps what the user sees: a meeting question without
        # the transcript that went with it.
        runtime.store.chat.append_message(
            session_id,
            "user",
            meeting.visible if meeting is not None else params.text,
            now_ms=now_ms,
            kind="voice" if params.voice is not None else "text",
            duration_ms=params.voice.duration_ms if params.voice is not None else None,
            client_id=params.client_id,
            attachments=params.attachments,
        )
        runtime.commit(request_id)
        if state is not TurnState.IDLE:
            # completed, failed and a cut turn first return to idle; the table
            # has no direct edge back into submitting.
            runtime.store.chat.set_status(session_id, TurnState.IDLE.value)
        runtime.store.chat.set_status(
            session_id, advance(TurnState.IDLE, TurnState.SUBMITTING).value
        )
        # Publish before the prompt: upstream may emit before its acknowledgement.
        await runtime.publish(emit("chat.session", {"session_id": session_id}, request_id))
        if state is TurnState.OUTCOME_UNKNOWN:
            await runtime.events.status(
                session_id,
                request_id,
                "warning",
                f"La respuesta anterior de {runtime.agent_name(agent_id)} no llegó: se cortó y "
                "el hub se ha reiniciado desde entonces",
            )
        return upstream_text

    async def _prompt(self, session_id, agent_id, connection, binding, blocks) -> None:
        runtime = self.runtime
        try:
            await connection.prompt(binding["runtime_id"], blocks)
        except HubError as exc:
            # The submit may have reached the agent; never resend it. A terminal
            # event observed during the RPC keeps its more authoritative state.
            async with runtime.conversations.committing(session_id):
                if exc.code == "session_runtime_not_found":
                    connection.live_sessions.discard(binding["runtime_id"])
                    runtime.resumed_message_counts.pop((agent_id, binding["runtime_id"]), None)
                    runtime.store.chat.set_status(session_id, TurnState.FAILED.value)
                    await runtime.changed()
                elif exc.code == "attachment_refused":
                    # The agent refused a file before the prompt went: the
                    # turn never started, and its staged images were taken back.
                    runtime.store.chat.set_status(session_id, TurnState.FAILED.value)
                    await runtime.changed()
                else:
                    runtime.apply_state(session_id, TurnState.OUTCOME_UNKNOWN)
            raise

    async def _blocks(self, session_id: str, agent_id: str, text: str, attached: list[Stored]):
        """The turn's content: its text, what it says of its files and the
        images that go with it (`attachments.turn_parts`, off the loop)."""
        runtime = self.runtime
        if not attached:
            return (TextBlock(text),)
        images = runtime.agents.get(agent_id).backend.capabilities.images
        try:
            said, carried = await asyncio.to_thread(turn_parts, attached, images)
        except Exception as exc:
            report(event="attachment_render_failed", code="attachment_unreadable")
            async with runtime.conversations.committing(session_id):
                runtime.store.chat.set_status(session_id, TurnState.FAILED.value)
                await runtime.changed()
            raise HubError(
                "attachment_unreadable",
                "No se pudieron preparar los adjuntos del mensaje para el agente; no se envió",
                numbers={"cause": "render_failed"},
            ) from exc
        return (
            TextBlock(f"{text}\n\n{said}"),
            *(
                ImageBlock(image.attachment_id, image.name, image.mime, image.data)
                for image in carried
            ),
        )

    async def _handover_turn(self, params) -> MeetingTurn:
        """The first turn of a conversation handed over from another one of
        this device: what was said there and the question, refused before
        anything reaches an agent when there is no question to carry."""
        runtime = self.runtime
        source = runtime.store.chat.session(runtime.device, params.handover_from)
        page = await runtime.run_store(
            lambda: runtime.store.chat.session_messages(
                runtime.device, params.handover_from, None, SESSION_MESSAGES_PAGE_LIMIT
            )
        )
        turn = handover.handover_turn(
            page["messages"],
            page["total"],
            params.text,
            runtime.agent_name(runtime.agents.owner(source), starts=True),
        )
        if turn is None:
            raise HubError(
                "handover_no_question",
                "La conversación no tiene ninguna pregunta sin respuesta que pasar; "
                "escribe la pregunta",
                numbers={"messages": page["total"]},
            )
        return turn

    async def _meeting_turn(self, params) -> MeetingTurn:
        """The question with the meeting the hub holds; refused as
        `meeting_not_found` before anything reaches the agent."""
        runtime = self.runtime
        meeting = params.meeting
        lines, started, pending = await runtime.run_store(
            lambda: read_meeting(runtime.store, runtime.device, meeting.recording_id)
        )
        elapsed = None if started is None else round(runtime.now() * 1000) - started
        return meeting_context(
            params.text,
            meeting.recording_id,
            meeting.title,
            meeting.minutes,
            lines,
            elapsed,
            pending,
        )
