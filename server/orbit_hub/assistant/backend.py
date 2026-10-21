"""The interface between the hub's runtime and the assistant that answers.

Shaped after the Agent Client Protocol (ACP), which Hermes, Claude Code and
Codex adapters speak: a connection carries sessions; a session takes turns
made of content blocks; a turn streams typed updates, may ask for permission
or for a clarification, and can be cancelled. The ACP names, for whoever
writes the next backend:

    create_session        session/new
    resume_session        session/load (Hermes: session.resume)
    prompt                session/prompt
    cancel                session/cancel
    PermissionRequested   session/request_permission
    MessageChunk          session/update agent_message_chunk
    MessageComplete       the session/prompt response (stop reason)

It carries only what the runtime uses today. Everything here is internal to
the hub: the Mac's protocol (`contracts/hub-v1.md`) is built from these types
by the runtime and does not change with the backend.
"""

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Literal, Protocol

from ..errors import HubError


@dataclass(frozen=True)
class Capabilities:
    """What a backend declares it accepts."""

    # A turn may carry images. No turn does today: SendParams holds text only,
    # and screenshots reach the assistant as orbit-mcp tool results.
    images: bool
    # A stored session reopens by its stored id after its live id is gone.
    resume: bool
    # The backend keeps each conversation and reloads it itself on resume,
    # announcing the end of that load with `HistoryLoaded`.
    history: bool
    # The backend has a `SpeechBackend` of its own for dictation and read-aloud.
    voice: bool


@dataclass(frozen=True)
class TextBlock:
    """The one content block a turn carries today."""

    text: str


ContentBlock = TextBlock


@dataclass(frozen=True)
class OpenedSession:
    # Live on the connection that opened it; events name it.
    session_id: str
    # Durable: what `resume_session` takes after the live id is gone.
    stored_id: str


@dataclass(frozen=True)
class ResumedSession:
    session_id: str
    stored_id: str
    # Messages in the reloaded segment, a positive count, when the backend says.
    message_count: int | None
    # True: the history is still loading and `HistoryLoaded` will follow.
    # False: it is ready. None: the backend did not say, so nothing is sent.
    loading_history: bool | None


@dataclass(frozen=True)
class PermissionRequest:
    request_id: str
    description: str
    # The answers the backend admits, in the Mac's vocabulary (`once`,
    # `deny`…). None: the backend named none, so `once` and `deny`.
    choices: tuple[str, ...] | None


@dataclass(frozen=True)
class ClarificationRequest:
    request_id: str
    question: str
    # Several questions at once, each a {qid, question, …} object the Mac
    # shows as it came. None: a single question; empty: the backend sent a
    # list the hub could not read.
    questions: tuple[dict, ...] | None
    choices: tuple[str, ...] | None


@dataclass(frozen=True)
class PermissionReceipt:
    # Whether the backend applied the answer; False means the request expired.
    applied: bool


@dataclass(frozen=True)
class ClarificationReceipt:
    status: Literal["ok", "expired"]
    # The question ids of a several-question request still unanswered.
    remaining: tuple[str, ...]


class UnreadableReceipt(HubError):
    """The backend acknowledged an answer with a receipt the hub cannot read.

    Distinct from a refused or lost call: the backend did receive the answer,
    so the hub retires the request instead of offering it again.
    """


@dataclass(frozen=True)
class MessageChunk:
    session_id: str
    text: str


@dataclass(frozen=True)
class MessageComplete:
    """The turn ended. `status` is the backend's, as the Mac reads it:
    `error` is a failed turn, anything else a finished one."""

    session_id: str
    text: str
    status: str
    # The turn failed because a resumed session's stored history could not be read.
    history_unreadable: bool = False


@dataclass(frozen=True)
class PermissionRequested:
    session_id: str
    request: PermissionRequest


@dataclass(frozen=True)
class ClarificationRequested:
    session_id: str
    request: ClarificationRequest


@dataclass(frozen=True)
class RequestExpired:
    """A permission or clarification request the backend no longer waits for."""

    session_id: str
    request_id: str


@dataclass(frozen=True)
class StatusNotice:
    """A transient status line of the running turn (Hermes's `status.update`)."""

    session_id: str
    kind: str
    text: str


@dataclass(frozen=True)
class TurnError:
    """The backend reported an error in this session without ending the turn.

    `terminal`: the turn is over without a final answer. `unconfirmed`: it may
    still be running, and only `turn_stopped` can tell. `history_unreadable`:
    a resumed session's stored history could not be read; outside a resume it
    is as unconfirmed as any other.
    """

    session_id: str
    cause: Literal["terminal", "unconfirmed", "history_unreadable"]


@dataclass(frozen=True)
class HistoryLoaded:
    """A resumed session finished loading its history, or failed to."""

    session_id: str
    loaded: bool


@dataclass(frozen=True)
class UnreadableUpdate:
    """The backend sent something for this session the hub could not read.
    `text` tells the user, in Spanish, what was ignored."""

    session_id: str
    text: str


@dataclass(frozen=True)
class ConnectionLost:
    """The connection ended without being closed: every live session is gone
    and the outcome of what was in flight is unknown."""


AssistantEvent = (
    MessageChunk
    | MessageComplete
    | PermissionRequested
    | ClarificationRequested
    | RequestExpired
    | StatusNotice
    | TurnError
    | HistoryLoaded
    | UnreadableUpdate
    | ConnectionLost
)
EventSink = Callable[[AssistantEvent], Awaitable[None]]


class AssistantConnection(Protocol):
    """One device's connection to the assistant.

    Calls that fail raise `HubError` with the cause. One whose answer was lost
    raises `outcome_unknown` and is never repeated: the backend may have acted.
    """

    # Identifies the backend's current run. A live session id from another
    # epoch is not live on this connection.
    epoch: str
    # The live session ids this connection holds. The hub discards one it
    # learns is gone, so the next turn resumes it from its stored id.
    live_sessions: set[str]

    @property
    def connected(self) -> bool: ...

    async def open(self) -> None: ...

    async def close(self) -> None: ...

    async def create_session(self) -> OpenedSession: ...

    async def resume_session(self, stored_id: str) -> ResumedSession: ...

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        """Start a turn. It returns once the backend accepted it; the answer
        arrives as events."""

    async def cancel(self, session_id: str) -> None: ...

    async def respond_permission(
        self, session_id: str, request_id: str, allow: bool
    ) -> PermissionReceipt: ...

    async def respond_clarification(
        self, session_id: str, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt: ...

    async def turn_stopped(self, session_id: str) -> bool:
        """True only when the backend confirms the session runs no turn.
        Read-only: it neither resumes nor repeats anything."""


class AssistantBackend(Protocol):
    capabilities: Capabilities

    def connection(self, device: str, device_name: str, on_event: EventSink) -> AssistantConnection:
        """A connection for one paired device, not yet open. `device_name` is
        the name its owner gave it; events go to `on_event`."""
