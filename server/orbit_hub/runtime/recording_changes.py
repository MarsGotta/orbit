"""Which meetings changed since a number this start of the hub gave.

Every `recording.changed` the hub announces is noted here first, with the
next number, which the notice carries (`change`) and `ready` says as the
latest. A Mac that was away asks which meetings changed after the last number
it read (`GET /v1/recordings/changes?since=N`) and reads again only those.

The log lives in memory and answers only for what this start of the hub saw:
a change saved just before the hub stopped may never have been noted, and a
database restored from a copy would give again numbers the lost one gave.
So each start begins at a random number, and a number from before it, one
this hub never gave, or one older than what the log still holds is unknown
(`None`: «read the list again»), never «nothing changed».
"""

import secrets
from collections import OrderedDict

# Where a start's numbering may begin: below 2**52, so that every number this
# hub gives in its life stays below 2**53 and is exact in any JSON reader
# (a double holds integers exactly up to 2**53). A number from another start
# falls into this one's range with a chance of one in 2**52 per change.
START_BELOW = 2**52

# Meetings the log remembers, each once, by its last change. It protects the
# hub's memory and the longest answer to `since`, which is a part of the log
# and must fit in what the Mac reads of one hub answer: 1 MiB
# (`MAX_HUB_HTTP_BODY_BYTES` in `contracts/constants.json`). Why this size:
# a meeting id is 36 characters plus its quotes and comma, so 1,000 take about
# 40 KB, far within that body; and a person changes a few meetings a day, so it
# covers months away, beyond the 300 that five meetings a day for two months
# would need. It does not limit meetings, changes or notices: past it the
# meeting changed longest ago leaves, the floor rises to its number, and a Mac
# asking from before it reads the list, as a Mac did before this log.
MAX_CHANGED_RECORDINGS = 1_000


class RecordingChanges:
    """The numbered changes of this start of the hub, one entry per meeting.

    Touched only from the hub's event loop, so it needs no lock."""

    def __init__(self) -> None:
        # The lowest number `since` still answers for: this start's first,
        # then the number of the last meeting to leave by the ceiling.
        self.floor = secrets.randbelow(START_BELOW)
        self.latest = self.floor
        # Meeting id to the number of its last change, the oldest first.
        self._last: OrderedDict[str, int] = OrderedDict()

    def note(self, recording: str) -> int:
        """The number of this change of `recording`, noted before its notice."""
        self.latest += 1
        self._last.pop(recording, None)
        self._last[recording] = self.latest
        if len(self._last) > MAX_CHANGED_RECORDINGS:
            _, self.floor = self._last.popitem(last=False)
        return self.latest

    def since(self, number: int) -> list[str] | None:
        """The meetings changed after `number`, each once, the oldest first;
        None when this log cannot know (below its floor, or above its latest)."""
        if not self.floor <= number <= self.latest:
            return None
        return [recording for recording, last in self._last.items() if last > number]
