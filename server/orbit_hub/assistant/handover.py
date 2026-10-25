"""Handing a conversation over to another agent (`chat.send` with `handover_from`).

When an agent cannot answer (its plan's limit, a session that expired), the
user carries on with another one: a new conversation with that agent, whose
first turn carries what was said so far and the question left unanswered.
The new conversation is marked with the one it came from; the old one stays
as it was, with its agent.

What the new agent is told is a copy, in the first turn's text: a backend
that keeps its own history (Hermes, official vendor) keeps it from there; the hub's
stored conversation holds only the question, as the user sees it.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..meeting_context import MeetingTurn

if TYPE_CHECKING:
    from ..storage import Store

SCHEMA = """
CREATE TABLE IF NOT EXISTS session_handovers (
    session_id TEXT PRIMARY KEY, from_session_id TEXT NOT NULL, from_agent_id TEXT NOT NULL);
"""

# What the new agent is given of the conversation it takes over, in
# characters. What it protects: the first turn's share of the new agent's
# context, which also holds its own instructions, the question (up to the
# 32,000 characters `chat.send` admits) and its answer. Why 32 Ki: about
# 8,000 tokens at four characters a token, a quarter of a 32,000-token
# window, the size the hub already assumes for a local model
# (`openai.HISTORY_MAX_CHARS` is twice this, for a whole stored history).
# The newest messages are kept; the older ones are left out and counted in
# the turn. It does not bound the conversation the hub keeps, which stays
# whole, nor the question, which is sent whole.
MAX_HANDOVER_CHARS = 32 * 1024


def handover_turn(
    messages: Sequence[dict],
    total: int,
    question: str,
    from_name: str,
) -> MeetingTurn | None:
    """The first turn for the new agent and the row the conversation keeps.

    `messages` is the newest page of the old conversation, oldest first, and
    `total` how many it holds. An empty `question` carries over the one left
    unanswered: the user's messages after the last answer. None when there
    is no question to carry."""
    said = list(messages)
    cut = len(said)
    while cut and said[cut - 1]["role"] == "user":
        cut -= 1
    unanswered = "\n\n".join(message["text"] for message in said[cut:])
    # The question goes once: the unanswered one, or the Mac's own when it
    # repeats it, leaves the copy; a different one keeps it there.
    if not question or question.strip() == unanswered.strip():
        question = question or unanswered
        said = said[:cut]
        total -= len(messages) - cut
    if not question:
        return None
    lines: list[str] = []
    used = 0
    for message in reversed(said):
        speaker = "Usuario" if message["role"] == "user" else from_name
        line = f"{speaker}: {message['text']}"
        if used + len(line) + 1 > MAX_HANDOVER_CHARS:
            break
        lines.append(line)
        used += len(line) + 1
    lines.reverse()
    left_out = total - len(lines)
    head = (
        f"[Orbit · Te pasan la conversación que tenía con {from_name}: lo hablado hasta "
        "ahora, del mensaje más antiguo al más reciente"
    )
    if left_out:
        head += (
            f". {left_out} mensajes anteriores no van: pasan del tope de "
            f"{MAX_HANDOVER_CHARS} caracteres"
        )
    body = "\n".join([head + "]", *lines, "[Fin de la conversación. Sigue tú con la pregunta:]"])
    return MeetingTurn(text=f"{body}\n{question}", visible=question)


def record(store: Store, session_id: str, from_session_id: str, from_agent_id: str) -> None:
    with store.transaction() as db:
        db.execute(
            "INSERT INTO session_handovers(session_id,from_session_id,from_agent_id) VALUES(?,?,?)",
            (session_id, from_session_id, from_agent_id),
        )


def origins(store: Store, session_ids: Sequence[str]) -> dict[str, dict]:
    """`handed_over_from` of each of these conversations that has one."""
    if not session_ids:
        return {}
    marks = ",".join("?" * len(session_ids))
    with store.read_snapshot() as db:
        rows = db.execute(
            "SELECT session_id,from_session_id,from_agent_id FROM session_handovers"
            f" WHERE session_id IN ({marks})",
            tuple(session_ids),
        ).fetchall()
    return {
        row["session_id"]: {"session_id": row["from_session_id"], "agent_id": row["from_agent_id"]}
        for row in rows
    }
