"""Correlating a remote request with the answer that belongs to it.

The device decides whether it will serve a request; this decides which answer
belongs to which question, and what happens when no answer arrives. Both halves
have to be conservative, but for different reasons: the device is protecting a
Mac, and this is protecting the fact that an answer carries a picture of
someone's screen. Handing it to the wrong waiting caller is the failure mode
that matters here, so every ambiguous case drops the payload instead.

Pure on purpose: no sockets, no clock, no tasks. Time is passed in.
"""

from __future__ import annotations

from dataclasses import dataclass, field

# One device answering an assistant, not a queue of assistants driving a Mac.
# The device enforces its own single-request rule; this bound exists so a
# caller cannot fill the hub's memory with questions the Mac never sees.
MAX_PENDING_REQUESTS = 4

# The daemon's capture upload gives up at 20 s and needs 10 s for the answer;
# the translator waits 35 s. This bounds one hub waiter, not a retry interval.
REQUEST_TIMEOUT_SECONDS = 30.0

# The engine configures a 5 s Cua request timeout; Cua rejects timeouts above
# 30 s. This outer 30 s bound covers that validated ceiling and frees one of
# four pending bridge slots even if the response never arrives. Five synthetic
# action-suite runs on 2026-09-30 took at most 0.65 s per suite, with the
# slowest action call at 0.08 s, on a loaded host; these do not measure macOS
# input. This bound is not an input retry or proof that a
# real action finishes within it.
ACTION_REPLY_TIMEOUT_SECONDS = 30.0


class BridgeRefused(Exception):
    """The hub will not carry this request."""

    def __init__(self, code: str, message: str, numbers: dict[str, int] | None = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.numbers = numbers


@dataclass(frozen=True)
class Pending:
    request_id: str
    device: str
    capability: str
    asked_at: float


@dataclass
class Bridge:
    """Which questions are outstanding, and whose they are."""

    pending: dict[str, Pending] = field(default_factory=dict)

    def open(self, request_id: str, device: str, capability: str, asked_at: float) -> Pending:
        # Reusing an id would let a late answer to the old question resolve the
        # new one, which is the exact confusion this module exists to prevent.
        if request_id in self.pending:
            raise BridgeRefused("duplicate_request", "That request is already in progress")
        if len(self.pending) >= MAX_PENDING_REQUESTS:
            raise BridgeRefused(
                "too_many_requests",
                "Hay demasiadas peticiones en curso",
                {"count": len(self.pending) + 1, "max": MAX_PENDING_REQUESTS},
            )
        entry = Pending(request_id, device, capability, asked_at)
        self.pending[request_id] = entry
        return entry

    def resolve(self, request_id: str, device: str) -> Pending | None:
        """Claims the question this answer belongs to, or nothing.

        Returning None means the payload must be dropped rather than delivered
        anywhere: an unknown id has no owner, and an id belonging to another
        device is a device answering a question it was never asked.
        """
        entry = self.pending.get(request_id)
        if entry is None or entry.device != device:
            return None
        del self.pending[request_id]
        return entry

    def expire(self, now: float, request_id: str | None = None) -> list[Pending]:
        """Drops questions old enough that their answer is no longer wanted.

        A caller that already gave up must not be handed a screenshot later,
        and the entry must not sit here occupying one of the few slots.
        """
        stale = [
            entry
            for entry in self.pending.values()
            if (request_id is None or entry.request_id == request_id)
            and now - entry.asked_at
            >= (
                ACTION_REPLY_TIMEOUT_SECONDS
                if entry.capability == "perform_action"
                else REQUEST_TIMEOUT_SECONDS
            )
        ]
        for entry in stale:
            del self.pending[entry.request_id]
        return stale

    def abandon(self, device: str) -> list[Pending]:
        """Drops everything outstanding for a device that went away.

        A Mac that disconnected will never answer, and keeping the question
        alive across a reconnection would let a reply arrive with the person no
        longer watching the screen it came from.
        """
        gone = [entry for entry in self.pending.values() if entry.device == device]
        for entry in gone:
            del self.pending[entry.request_id]
        return gone
