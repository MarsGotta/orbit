"""A conversation's controls: interrupting its turn, answering its questions.

Each runs whole under its conversation's lock, in arrival order, after the
sends of that conversation admitted before it (`Conversations.controlling`);
never behind another conversation.
"""

from dataclasses import replace

from ..assistant.backend import UnreadableReceipt
from ..interactions import public_receipt
from ..protocol import HubError
from ..turns import INTERRUPT_DEADLINE_SECONDS, TurnState, advance


class Controls:
    def __init__(self, runtime):
        self.runtime = runtime

    async def handle(self, method: str, params) -> tuple[str, dict]:
        runtime = self.runtime
        async with runtime.conversations.controlling(params.session_id):
            if not runtime.store.devices.is_active(runtime.device):
                raise HubError("revoked", "Device revoked")
            binding, connection = runtime.require_live(params.session_id)
            if method == "chat.interrupt":
                return await self.interrupt(params, binding, connection)
            return await self.answer(method, params, binding, connection)

    async def interrupt(self, params, binding: dict, connection) -> tuple[str, dict]:
        runtime = self.runtime
        runtime.safety_target("chat.interrupt", params)
        await connection.cancel(binding["runtime_id"])
        state = runtime.turn_state(params.session_id)
        if state in {TurnState.SUBMITTING, TurnState.STREAMING}:
            # Only the agent's acknowledgement of the interruption request
            # moves a live turn to interrupting; the deadline after which
            # it becomes outcome_unknown starts here. A late submit ACK
            # cannot move interrupting back to streaming.
            runtime.store.chat.set_status(
                params.session_id, advance(state, TurnState.INTERRUPTING).value
            )
            runtime.interrupt_deadlines[params.session_id] = (
                runtime.now() + INTERRUPT_DEADLINE_SECONDS
            )
        return "chat.interrupted", {"session_id": params.session_id, "status": "requested"}

    def _pending(self, kind: str, params):
        """The question being answered, refused when it is not this turn's."""
        runtime = self.runtime
        pending = runtime.pending.get(params.request_id)
        if pending is None or pending[:2] != (params.session_id, kind):
            raise HubError(
                "interaction_not_found",
                "Request belongs to another session, is unknown or has expired",
            )
        if (
            kind == "approval"
            and params.turn_id is not None
            and params.turn_id != runtime.turn_ids.get(params.session_id)
        ):
            raise HubError("stale_turn", "The response does not belong to the current turn")
        if kind == "approval":
            choices = pending[2].choices
            allowed = ("once", "deny") if choices is None else choices
            if params.answer() not in allowed:
                raise HubError(
                    "approval_not_allowed",
                    f"The agent does not offer «{params.answer()[:64]}» in this request; it offers {len(allowed)}: {', '.join(choice[:64] for choice in allowed[:8])}",
                )
        return pending

    async def answer(self, method, params, binding: dict, connection) -> tuple[str, dict]:
        runtime = self.runtime
        kind = "approval" if method == "approval.respond" else "clarify"
        pending = self._pending(kind, params)
        runtime_id = binding["runtime_id"]
        agent_id = runtime.agents.owner(binding)
        turn_id = runtime.turn_ids.get(params.session_id)
        epoch = connection.epoch

        def still_current() -> bool:
            # An entry's object identity is its generation: every request/partial
            # update replaces the tuple, including reuse of an identical request ID.
            # The event reader remains unlocked and can expire/complete/replace it.
            return (
                runtime.pending.get(params.request_id) is pending
                and turn_id is not None
                and runtime.turn_ids.get(params.session_id) == turn_id
                and runtime.connections.get(agent_id) is connection
                and connection.connected
                and connection.epoch == epoch
                and runtime_id in connection.live_sessions
                and not runtime.revoked
                and runtime.store.devices.is_active(runtime.device)
                and runtime.store.chat.session(runtime.device, params.session_id)["status"]
                == "streaming"
            )

        # The backend knows the request by its own id.
        upstream_id = pending[2].request_id
        try:
            if kind == "approval":
                answered = await connection.respond_permission(
                    runtime_id, upstream_id, choice=params.answer()
                )
            else:
                answered = await connection.respond_clarification(
                    runtime_id, upstream_id, params.answer, params.question_id
                )
        except UnreadableReceipt:
            if still_current():
                runtime.pending.pop(params.request_id, None)
                await runtime.changed()
            raise
        return self._resolved(kind, params, pending, turn_id, still_current(), answered)

    def _resolved(self, kind, params, pending, turn_id, current: bool, answered):
        runtime = self.runtime
        remaining, receipt = public_receipt(answered)
        if not current:
            # Preserve a genuinely applied historical ACK, but never use it
            # to mutate a terminal/replaced interaction. Clients must consume
            # this response without clearing pending; the snapshot is current.
            receipt = {**receipt, "stale_interaction": True, "turn_id": turn_id}
        elif not remaining:
            runtime.pending.pop(params.request_id, None)
        elif kind == "clarify":
            # Batch replies acknowledge individual qids. Reconnection must
            # not present an already answered question as still pending.
            updated = pending[2]
            if updated.questions is not None:
                updated = replace(
                    updated,
                    questions=tuple(
                        question
                        for question in updated.questions
                        if question.get("qid") in receipt["remaining"]
                    ),
                )
            runtime.pending[params.request_id] = (params.session_id, kind, updated)
        # Boundary emits the result first, then this authoritative cached snapshot.
        runtime.touched()
        name = "interaction.expired" if receipt["status"] == "expired" else "interaction.resolved"
        return name, {"session_id": params.session_id, "request_id": params.request_id, **receipt}
