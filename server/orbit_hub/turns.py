"""Explicit turn state machine for chat turns.

The session status persisted by the hub is exactly one ``TurnState``. The only
legal evolutions are the rows of :data:`TRANSITIONS`; there is no edge from
``outcome_unknown`` back into an active turn, so a turn whose result is unknown
can never be resumed or overwritten by a new submit on the same session.

Transition table:

    idle           -> submitting
    submitting     -> streaming | interrupting | outcome_unknown
    streaming      -> interrupting | completed | failed | outcome_unknown
    interrupting   -> outcome_unknown | completed | failed
    completed      -> idle
    failed         -> idle
    outcome_unknown -> (terminal: reconciliation only, never a resend)

``submitting`` covers the window in which ``prompt.submit`` has been sent but
not acknowledged; it may be interrupted while that RPC waits. An upstream
event observed in that window proves the turn is live, so it first advances to
``streaming`` before any terminal transition.
``interrupting`` keeps ``completed``/``failed`` because Hermes may still deliver
a final result while an interruption request is in flight. ``completed`` and
``failed`` return to ``idle``: only then can a new submit start. An
``interrupting`` turn that hears nothing from Hermes for
``INTERRUPT_DEADLINE_SECONDS`` seconds becomes ``outcome_unknown``.
"""

from enum import Enum

from .protocol import HubError

INTERRUPT_DEADLINE_SECONDS = 30


class TurnState(str, Enum):
    IDLE = "idle"
    SUBMITTING = "submitting"
    STREAMING = "streaming"
    INTERRUPTING = "interrupting"
    COMPLETED = "completed"
    FAILED = "failed"
    OUTCOME_UNKNOWN = "outcome_unknown"


TRANSITIONS: dict[TurnState, frozenset[TurnState]] = {
    TurnState.IDLE: frozenset({TurnState.SUBMITTING}),
    TurnState.SUBMITTING: frozenset(
        {TurnState.STREAMING, TurnState.INTERRUPTING, TurnState.OUTCOME_UNKNOWN}
    ),
    TurnState.STREAMING: frozenset(
        {
            TurnState.INTERRUPTING,
            TurnState.COMPLETED,
            TurnState.FAILED,
            TurnState.OUTCOME_UNKNOWN,
        }
    ),
    TurnState.INTERRUPTING: frozenset(
        {TurnState.OUTCOME_UNKNOWN, TurnState.COMPLETED, TurnState.FAILED}
    ),
    TurnState.COMPLETED: frozenset({TurnState.IDLE}),
    TurnState.FAILED: frozenset({TurnState.IDLE}),
    TurnState.OUTCOME_UNKNOWN: frozenset(),
}


def advance(state: TurnState, target: TurnState) -> TurnState:
    """Validate one hop against the table and return the target state."""
    if target not in TRANSITIONS[state]:
        raise HubError(
            "invalid_turn_transition",
            f"Transición de turno no permitida: {state.value} a {target.value}",
        )
    return target


def can_start_turn(state: TurnState) -> bool:
    """Whether a new submit may begin from ``state``.

    ``idle`` starts directly; ``completed`` and ``failed`` first return to
    ``idle``. Every other state still owns the session (or its outcome is
    unknown), so no new submit may reuse it.
    """
    return state is TurnState.IDLE or TurnState.IDLE in TRANSITIONS[state]
