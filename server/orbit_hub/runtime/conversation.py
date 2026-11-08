"""The locks of one conversation, so no conversation waits on another.

Each conversation of a device has its own two locks. `admission` takes its
sends one at a time, in arrival order, from their checks through waiting for
the turn under way and opening or reopening the agent's session to their
commit. `lock` takes its controls (interrupt, approval, clarification) and
each commit of its turn's state, so each runs whole, in arrival order. An
interrupt also waits for the sends of its own conversation admitted before it
(`controlling`), so it applies to the turn sent before it; an answer to a
question does not, because its question belongs to a turn already sent. None
of them waits for another conversation: a slow start, reopen or answer of one
conversation (up to minutes for an official vendor agent) holds only that
conversation.

A send that opens a new conversation has nothing to wait for: it gets locks
of its own, which become its conversation's once it has an id.
"""

import asyncio
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager


class Conversation:
    def __init__(self):
        self.lock = asyncio.Lock()
        self.admission = asyncio.Lock()
        # One entry per send admitted and not yet through admission: its
        # request id and the event set when it leaves.
        self.admitting: list[tuple[str, asyncio.Event]] = []
        # The tasks that hold or wait for this conversation's locks: it is
        # forgotten when none does, so the device keeps only those in use.
        self.users = 0


class Conversations:
    """The device's conversations in use, by session id."""

    def __init__(self):
        self.by_session: dict[str, Conversation] = {}

    @asynccontextmanager
    async def using(self, session_id: str | None) -> AsyncIterator[Conversation]:
        """The conversation's locks for as long as the caller needs them."""
        if session_id is None:
            yield Conversation()
            return
        conversation = self.by_session.setdefault(session_id, Conversation())
        conversation.users += 1
        try:
            yield conversation
        finally:
            conversation.users -= 1
            if not conversation.users and self.by_session.get(session_id) is conversation:
                del self.by_session[session_id]

    def waiting(self, session_id: str | None) -> int:
        """How many sends of the conversation have not left admission yet."""
        conversation = self.by_session.get(session_id)
        return 0 if conversation is None else len(conversation.admitting)

    def waiting_sends(self) -> set[tuple[str, str]]:
        """Each (session id, request id) of a send not yet through admission."""
        return {
            (session_id, request_id)
            for session_id, conversation in self.by_session.items()
            for request_id, _ in conversation.admitting
        }

    @asynccontextmanager
    async def admitting(self, session_id: str | None, request_id: str) -> AsyncIterator[None]:
        """One send's admission, after the sends of its conversation before it;
        an interrupt of that conversation that arrives meanwhile waits for it."""
        async with self.using(session_id) as conversation:
            entry = (request_id, asyncio.Event())
            conversation.admitting.append(entry)
            try:
                async with conversation.admission:
                    yield
            finally:
                conversation.admitting.remove(entry)
                entry[1].set()

    @asynccontextmanager
    async def controlling(self, session_id: str, turn_id: str) -> AsyncIterator[None]:
        """An interrupt's turn: after the sends of its conversation admitted
        before it, up to the one it names, under the conversation's lock. A
        send behind the named one leaves only when that turn ends, which this
        interrupt is here to bring about. Only an interrupt waits for sends;
        an answer to a question takes only the lock (`committing`)."""
        async with self.using(session_id) as conversation:
            for request_id, admitted in tuple(conversation.admitting):
                await admitted.wait()
                if request_id == turn_id:
                    break
            async with conversation.lock:
                yield

    @asynccontextmanager
    async def committing(self, session_id: str) -> AsyncIterator[None]:
        """A commit of the conversation's turn state, under its lock."""
        async with self.using(session_id) as conversation, conversation.lock:
            yield
