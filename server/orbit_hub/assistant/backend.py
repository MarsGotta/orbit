"""The vendor-independent contract between the runtime and its assistants.

Connections hold sessions, sessions accept turns of content blocks, and turns
emit typed text, activity and human requests. Vendor backends translate their
own protocol here. The runtime builds the Mac protocol (contracts/hub-v1.md)
from these types, independently of the backend.
"""

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Literal, Protocol

from ..errors import HubError
from .deletion import Deletion


@dataclass(frozen=True)
class Capabilities:
    """What a backend declares it accepts."""

    # A turn may carry `ImageBlock`s: the images the user attached, and the
    # pages of a PDF that have almost no text. Without it the turn only names
    # them, and the agent reads them with orbit-mcp's `orbit_attachment`.
    images: bool
    # A stored session reopens by its stored id after its live id is gone.
    resume: bool
    # The backend keeps each conversation and reloads it itself on resume
    # (a vendor's session or thread resume); a load still under way
    # says so in `ResumedSession.loading_history` and ends with `HistoryLoaded`.
    # False: it keeps none, and the hub hands it the conversation it stores
    # when it resumes.
    history: bool
    # The backend has a `SpeechBackend` of its own for dictation and read-aloud.
    voice: bool


@dataclass(frozen=True)
class TextBlock:
    text: str


@dataclass(frozen=True)
class ImageBlock:
    """An image the user attached (or a PDF page rendered by the hub), whole.
    Only a backend whose `Capabilities.images` is true receives one."""

    # The attachment it comes from, which `orbit_attachment` reads again.
    attachment_id: str
    # What the user sees: the file's name, or «name · página N».
    name: str
    # image/png, image/jpeg, image/gif or image/webp.
    mime: str
    data: bytes


ContentBlock = TextBlock | ImageBlock


@dataclass(frozen=True)
class OpenedSession:
    # Live on the connection that opened it; events name it.
    session_id: str
    # Durable: what `resume_session` takes after the live id is gone.
    stored_id: str


@dataclass(frozen=True)
class StoredMessage:
    role: Literal["user", "assistant"]
    text: str


@dataclass(frozen=True)
class StoredConversation:
    """What the hub stores of a conversation, for a backend that keeps none.

    `messages` are the newest the hub reads in one page, oldest first, as the
    Mac shows them; `total` counts every message the conversation holds, so
    `total - len(messages)` were left out before the backend trims anything.
    """

    messages: tuple[StoredMessage, ...]
    total: int


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
    # What it deletes, when what it runs deletes files (`deletion`): the
    # runtime then offers it one at a time.
    deletes: Deletion | None = None


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
class TurnFailed:
    """The turn ended without an answer, for a cause the backend names.

    Unlike `TurnError`, the backend knows the turn is over and why: `code`,
    a Spanish `message` and the cause's `numbers` reach the Mac as the turn's
    `error` event. Nothing is resent.
    """

    session_id: str
    code: str
    message: str
    # Numbers, and the name of a file a refusal is about (`attachment_refused`).
    numbers: dict[str, int | str] | None = None


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
class PlanStep:
    text: str
    status: Literal["pending", "in_progress", "completed", "cancelled"]


@dataclass(frozen=True)
class ActivityItem:
    """One step of the agent's work: official SDK tool/thinking blocks,
    app-server items and plans; Hermes's `tool.*`,
    `todo.updated`, `reasoning.available` and `subagent.*`; pydantic-ai's
    tool calls, tool results and thinking parts (`openai`). The runtime
    publishes it as `chat.item`, labelled by `activity`.

    `item_id` is stable across one step's phases within the turn. A tool
    carries its name as the agent called it and its arguments when known; a
    plan its whole list of steps; reasoning its summary and a subagent its
    goal in `text`; a failed step its cause, in the agent's words.
    """

    session_id: str
    item_id: str
    kind: Literal["tool", "reasoning", "plan", "subagent"]
    phase: Literal["started", "progress", "completed", "failed"]
    tool: str | None = None
    arguments: dict | None = None
    text: str = ""
    steps: tuple[PlanStep, ...] = ()
    cause: str | None = None
    cause_code: str | None = None


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
    | TurnFailed
    | HistoryLoaded
    | UnreadableUpdate
    | ActivityItem
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

    async def resume_session(
        self, stored_id: str, history: StoredConversation | None = None
    ) -> ResumedSession:
        """`history` is passed only to a backend that keeps no conversation
        (`capabilities.history` False): what the hub stores of this one."""

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        """Start a turn. It returns once the backend accepted it; the answer
        arrives as events."""

    async def cancel(self, session_id: str) -> None: ...

    async def respond_permission(
        self, session_id: str, request_id: str, *, choice: str
    ) -> PermissionReceipt:
        """Answer with one of the request's `choices` (`once` and `deny` when
        it named none); `deny` is the only refusal. Keyword-only, so a backend
        still written for the old `allow: bool` fails instead of reading
        `"deny"` as true."""

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
