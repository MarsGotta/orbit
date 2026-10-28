"""The key of a meeting series: the calendar's stable identifier of the event.

A recurring event shares one `calendar_item_id` across its occurrences, so the
recordings linked to them form a series. A single event has one too, and
assigning it a template works the same way.
"""

# Bound of a series key, in UTF-8 bytes. What it protects: the rows keyed by
# it (template links) and the event of every note request. Why this size: it
# is the calendar's own bound on an identifier — `MAX_LABEL_BYTES` in
# `orbit-core` and `contracts/constants.json` (1 024 bytes) — so the Mac never
# sends a key the hub refuses, and
# `tests/test_note_templates.py` compares both copies. What it is not: a
# limit on how many series exist. EventKit's identifiers measure under 100
# bytes; the bound only refuses something that is not an identifier.
MAX_SERIES_KEY_BYTES = 1024


class SeriesKeyError(ValueError):
    """A series key that is not an identifier: its cause and the numbers that say why.

    `cause` is `empty`, `too_long` (with `bytes` and `max_bytes`) or
    `control_character`; `refusal()` is what travels to whoever asked.
    """

    def __init__(self, cause: str, reason: str, **numbers: int):
        super().__init__(reason)
        self.cause, self.reason, self.numbers = cause, reason, numbers

    def refusal(self) -> dict:
        return {"parameter": "calendar_item_id", "cause": self.cause, **self.numbers}


def valid_series_key(value: str) -> str:
    """The key itself, or a refusal naming its cause: empty, too long or with a control."""
    if not isinstance(value, str) or not value:
        raise SeriesKeyError("empty", "the series key is empty")
    size = len(value.encode("utf-8"))
    if size > MAX_SERIES_KEY_BYTES:
        raise SeriesKeyError(
            "too_long",
            f"the series key is {size} bytes and the maximum is {MAX_SERIES_KEY_BYTES}",
            bytes=size,
            max_bytes=MAX_SERIES_KEY_BYTES,
        )
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        raise SeriesKeyError("control_character", "the series key contains control characters")
    return value
