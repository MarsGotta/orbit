"""What a turn did, kept with its answer: “Worked for 46 s”, “Used the screen ·
3 steps · 46 s”. The steps themselves stay live (`chat.item`); this summary is
what the answer keeps after later turns and when the conversation is opened
again, on any Mac. It is the Mac's own count (`orbit_core::agent_work`): a tool
step counts once, when it first ends, and what it used is named once.
"""

# The distinct things a summary names (“the screen and the calendar”): past
# eight a sentence is no longer read at a glance. The Mac's copy is
# `orbit_core::agent_work::MAX_WORK_SUBJECTS` (contracts/constants.json). It
# bounds the summary kept with each answer, not what the agent does.
MAX_WORK_SUBJECTS = 8

_ENDED = {"completed", "failed"}


class TurnWork:
    """The tool steps of one turn, from its `chat.item`s."""

    def __init__(self, started_at_ms: int):
        self.started_at_ms = started_at_ms
        self._ended: set[str] = set()
        self._subjects: list[str] = []
        self._subject_codes: dict[str, str] = {}

    def record(self, item: dict) -> None:
        """One public step (`activity.public_item`)."""
        if item.get("kind") != "tool":
            return
        if item.get("phase") in _ENDED:
            self._ended.add(item["item_id"])
        subject = item.get("subject")
        if subject and subject not in self._subjects and len(self._subjects) < MAX_WORK_SUBJECTS:
            self._subjects.append(subject)
            if code := item.get("subject_code"):
                self._subject_codes[subject] = code

    def summary(self, ended_at_ms: int) -> dict:
        return {
            "started_at_ms": self.started_at_ms,
            "ended_at_ms": max(ended_at_ms, self.started_at_ms),
            "tools": len(self._ended),
            "subjects": list(self._subjects),
            **({"subject_codes": dict(self._subject_codes)} if self._subject_codes else {}),
        }
