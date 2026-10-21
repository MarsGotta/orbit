"""The text budget shared by meeting reads for Deimos."""

# Characters of meeting text per answer. What it protects: Deimos's context,
# through Hermes's 50 000-character whole-result limit. Why this size: it
# leaves 2 000 characters for fixed page headers, state and speaker names.
# Missing-group details retain their numbers; this budget bounds text only. At
# the measured 3.8 characters per token of Spanish speech
# it is about 12 600 tokens, some 90 minutes
# of conversation. This does not limit what the hub stores —
# the rest is one `after_index` away, and nothing is ever dropped.
MAX_ANSWER_CHARS = 48_000
# A page cut prefers the end of a line, unless that would waste more than half
# the page (one very long turn); then it cuts mid-line and the numbers say so.
_MIN_LINE_CUT = MAX_ANSWER_CHARS // 2


def cut(text: str) -> str:
    """At most MAX_ANSWER_CHARS of `text`, ending at a line when that loses little."""
    if len(text) <= MAX_ANSWER_CHARS:
        return text
    window = text[:MAX_ANSWER_CHARS]
    end = window.rfind("\n")
    return window[: end + 1] if end >= _MIN_LINE_CUT else window
