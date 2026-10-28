"""Short-lived evidence that a device's next turn came from the hub's STT."""

from collections.abc import Callable

TRANSCRIBED_TURN_TTL_SECONDS = 120


class TurnInputTracker:
    """Short-lived evidence that a device's next turn came from hub STT."""

    def __init__(self, now: Callable[[], float]):
        self.now = now
        self.transcriptions: dict[str, tuple[float, str]] = {}

    def mark_transcribed(self, device: str, text: str) -> None:
        current = self.now()
        self.transcriptions = {
            owner: evidence
            for owner, evidence in self.transcriptions.items()
            if evidence[0] >= current
        }
        self.transcriptions[device] = (current + TRANSCRIBED_TURN_TTL_SECONDS, text)

    def consume(self, device: str, text: str) -> str:
        evidence = self.transcriptions.pop(device, None)
        if evidence is not None and evidence[0] >= self.now() and evidence[1] == text:
            return "transcribed voice note"
        return "text"
