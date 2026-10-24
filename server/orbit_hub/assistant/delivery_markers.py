"""Hermes's platform-delivery markers, taken out of an answer before the hub
stores it or sends it to the Mac.

Hermes hands a file to a messaging platform with a `MEDIA:<path>` tag in the
agent's answer (its `text_to_speech` tool returns one), and asks for it as a
voice bubble or as a document with `[[audio_as_voice]]` and `[[as_document]]`
(`gateway/platforms/base.py`, read 2026-10-24). Orbit is not such a platform:
it reads answers aloud itself, and the path names a file on the server. A
marker inside a fenced code block is text the answer shows, and stays.
"""

from __future__ import annotations

import re

# A tag with a path as Hermes accepts one: quoted, home-relative or absolute,
# with the emphasis or quotes a model may put around it.
_MEDIA = re.compile(
    r"""[`"'*_]{0,3}MEDIA:\s*(?:`[^`\n]+`|"[^"\n]+"|'[^'\n]+'|(?:~/|/|[A-Za-z]:[/\\])\S+)"""
    r"""[`"'*_]{0,3}""",
    re.IGNORECASE,
)
_DIRECTIVE = re.compile(r"\[\[(?:audio_as_voice|as_document)\]\]", re.IGNORECASE)
_FENCE = re.compile(r" {0,3}(?:```|~~~)")
# How a line that is a marker begins, upper-cased.
_STARTS = ("MEDIA:", "[[AUDIO_AS_VOICE]]", "[[AS_DOCUMENT]]")
_WRAPPING = "`\"'*_"


def _clean_line(line: str) -> tuple[str | None, int]:
    """The line without its markers, None when nothing else was on it, and
    how many it had."""
    cleaned, media = _MEDIA.subn("", line)
    cleaned, directives = _DIRECTIVE.subn("", cleaned)
    if media + directives == 0:
        return line, 0
    return (cleaned.rstrip() if cleaned.strip() else None), media + directives


def without_markers(text: str) -> tuple[str, int]:
    """`text` without its markers, and how many there were."""
    kept: list[str] = []
    removed = 0
    fenced = False
    for line in text.split("\n"):
        if _FENCE.match(line):
            fenced = not fenced
        elif not fenced:
            cleaned, count = _clean_line(line)
            removed += count
            if cleaned is None:
                continue
            line = cleaned
        kept.append(line)
    if removed == 0:
        return text, 0
    return "\n".join(kept).strip("\n"), removed


def _may_be_marker(start: str) -> bool:
    """Whether a line that begins with `start` may still be a marker line."""
    begun = start.lstrip().lstrip(_WRAPPING).upper()
    return any(tag.startswith(begun) or begun.startswith(tag) for tag in _STARTS)


class MarkerStream:
    """The same over an answer that arrives in pieces: a line that may still
    turn out to be a marker waits for its end, the rest passes at once. Only
    a whole line of markers is dropped here; the final answer, cleaned by
    `without_markers`, replaces what was streamed."""

    def __init__(self) -> None:
        self.line = ""
        self.shown = 0
        self.fenced = False

    def feed(self, piece: str) -> str:
        """What of `piece` can be shown now."""
        out: list[str] = []
        *ended, self.line = (self.line + piece).split("\n")
        for line in ended:
            out.append(self._ended(line))
        if self.shown or not _may_be_marker(self.line):
            out.append(self.line[self.shown :])
            self.shown = len(self.line)
        return "".join(out)

    def _ended(self, line: str) -> str:
        shown, self.shown = self.shown, 0
        if _FENCE.match(line):
            self.fenced = not self.fenced
        elif not self.fenced and not shown:
            cleaned, _ = _clean_line(line)
            if cleaned is None:
                return ""
            line = cleaned
        return line[shown:] + "\n"
