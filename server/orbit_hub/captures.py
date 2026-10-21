"""Bounded, one-shot storage for screenshots uploaded by paired devices."""

from dataclasses import dataclass
from time import monotonic
from typing import Literal

# Four one-shot captures per device preserve the existing storage budget;
# this is not the global count of bridge waiters.
MAX_SAVED_CAPTURES_PER_DEVICE = 4

CAPTURE_TTL_SECONDS = 300.0

# One capture's PNG: the Mac's orbit_core::sources::MAX_SOURCE_BYTES, which it
# fits every capture into, and the helper's encoder bound. A copy of the
# registry value (contracts/constants.json), compared by
# scripts/tests/test_constants_copies.py.
MAX_CAPTURE_BYTES = 700_000


@dataclass(frozen=True)
class _Capture:
    request_id: str
    body: bytes
    expires_at: float


@dataclass(frozen=True)
class CapturePutResult:
    cause: Literal["stored", "duplicate", "full"]
    count: int
    max: int


class CaptureStore:
    """Keeps remote captures only until their matching bridge reply consumes them."""

    def __init__(self, *, now=monotonic):
        self._now = now
        self._captures: dict[tuple[str, str], _Capture] = {}

    def _expire(self, now: float) -> None:
        expired = [key for key, capture in self._captures.items() if capture.expires_at <= now]
        for key in expired:
            del self._captures[key]

    def put(self, device: str, capture_id: str, request_id: str, body: bytes) -> CapturePutResult:
        """Return the cause and counts from the same unexpired store view."""
        now = self._now()
        self._expire(now)
        key = (device, capture_id)
        count = sum(owner == device for owner, _ in self._captures)
        if key in self._captures or any(
            owner == device and capture.request_id == request_id
            for (owner, _), capture in self._captures.items()
        ):
            return CapturePutResult("duplicate", count, MAX_SAVED_CAPTURES_PER_DEVICE)
        if count >= MAX_SAVED_CAPTURES_PER_DEVICE:
            return CapturePutResult("full", count + 1, MAX_SAVED_CAPTURES_PER_DEVICE)
        self._captures[key] = _Capture(request_id, bytes(body), now + CAPTURE_TTL_SECONDS)
        return CapturePutResult("stored", count + 1, MAX_SAVED_CAPTURES_PER_DEVICE)

    def take(self, device: str, capture_id: str, request_id: str) -> bytes | None:
        """Returns exact bytes once, only for the device and request that uploaded them."""
        now = self._now()
        self._expire(now)
        key = (device, capture_id)
        capture = self._captures.get(key)
        if capture is None or capture.request_id != request_id:
            return None
        del self._captures[key]
        return capture.body
