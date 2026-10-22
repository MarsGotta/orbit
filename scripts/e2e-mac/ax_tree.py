"""The accessibility tree cua-driver returns, read as data.

`get_window_state` (cua-driver 0.28.2, the version Orbit bundles) renders the
tree of one window as Markdown, one node per line, two spaces of indent per
level (libs/cua-driver/rust/crates/platform-macos/src/ax/tree.rs):

    - [N] AXRole "title" = "value" (description) [id=… help="…" actions=[press]]
    - AXStaticText = "value"

`[N]` is the element index an action takes; only nodes that expose an action
have one. Nothing is escaped, so a quote inside a text is read up to the last
delimiter that still fits the line.

Kept free of I/O: the checks and the dry run read the same trees the Mac gives.
"""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass

_LINE = re.compile(
    r"^(?P<indent>(?:  )*)- (?:\[(?P<index>\d+)\] )?(?P<role>\S+)"
    r'(?: "(?P<title>.*?)")?(?: = "(?P<value>.*?)")?(?: \((?P<description>.*?)\))?'
    r"(?: \[(?P<attributes>(?:[^\[\]]|\[[^\]]*\])*)\])?$"
)
# «00:12», «12:34» or «1:02:03»: how the note screen writes a turn's offset
# (formatTimestamp, apps/desktop/src/features/notes/notes-format.ts).
TIME = re.compile(r"^(?:(\d+):)?(\d{2}):(\d{2})$")


@dataclass(frozen=True)
class Node:
    depth: int
    index: int | None
    role: str
    title: str | None
    value: str | None
    description: str | None

    def texts(self) -> list[str]:
        """What a person reads on this node: its title, value and description."""
        return [text for text in (self.title, self.value, self.description) if text]

    def text(self) -> str:
        return " ".join(self.texts())


# Where a node starts: its indent, «- », the optional index and an AX role.
# A text that holds a line break continues on lines that do not: a row whose
# initial and title are separate lines (real run 15), or Deimos's answer in
# the panel, whose list items start «- **Presupuesto**…» (real run 18).
_NODE = re.compile(r"^(?:  )*- (?:\[\d+\] )?AX")


def _lines(markdown: str) -> list[str]:
    """One string per node; the lines that continue a text join it with a space."""
    lines: list[str] = []
    for line in markdown.splitlines():
        if _NODE.match(line) or not lines:
            lines.append(line)
        else:
            lines[-1] += " " + line.strip()
    return lines


# macOS's menu bar, not Orbit's: a whole-window read brings it, and under the
# Apple menu «Ítems recientes» names the user's own documents (real run 36). No
# step presses or reads a menu, so its subtree never reaches the evidence.
_MENU_BAR = re.compile(r"^(?P<indent>(?:  )*)- (?:\[\d+\] )?AXMenuBar(?:\s|$)")


def without_menu_bar(markdown: str) -> str:
    """The tree without the menu bar and everything under it."""
    kept: list[str] = []
    inside: int | None = None  # the indent of the menu bar being left out
    for line in markdown.splitlines(keepends=True):
        if inside is not None:
            if not _NODE.match(line) or len(line) - len(line.lstrip(" ")) > inside:
                continue
            inside = None
        bar = _MENU_BAR.match(line)
        if bar is not None:
            inside = len(bar["indent"])
            continue
        kept.append(line)
    return "".join(kept)


def parse(markdown: str) -> list[Node]:
    nodes = []
    for line in _lines(markdown):
        found = _LINE.match(line.rstrip())
        if found is None:
            continue
        index = found["index"]
        nodes.append(
            Node(
                depth=len(found["indent"]) // 2,
                index=int(index) if index is not None else None,
                role=found["role"],
                title=found["title"],
                value=found["value"],
                description=found["description"],
            )
        )
    return nodes


def plain(text: str) -> str:
    """Lower case without accents, for comparisons a person would call equal."""
    decomposed = unicodedata.normalize("NFD", text.casefold())
    return "".join(c for c in decomposed if unicodedata.category(c) != "Mn")


def all_text(nodes: list[Node]) -> str:
    return "\n".join(node.text() for node in nodes if node.texts())


def find(
    nodes: list[Node],
    text: str,
    roles: tuple[str, ...] = (),
    contains: bool = False,
) -> list[Node]:
    """Actionable nodes whose title, value or description is `text`."""

    def matches(candidate: str) -> bool:
        return text in candidate if contains else candidate == text

    return [
        node
        for node in nodes
        if node.index is not None
        and (not roles or node.role in roles)
        and any(matches(candidate) for candidate in node.texts())
    ]


def seconds(time: str) -> int | None:
    found = TIME.match(time)
    if found is None:
        return None
    hours, minutes, secs = found.groups()
    return int(hours or 0) * 3600 + int(minutes) * 60 + int(secs)


@dataclass(frozen=True)
class Segment:
    time: str
    speaker: str
    text: str


def transcript(nodes: list[Node], start: str, end: str) -> tuple[list[str], list[Segment]]:
    """The turns of the «Transcripción» tab, in the order they are shown.

    Each turn is an <article> whose header holds the time, the speaker and
    then the text (TranscriptSegment.tsx), so in the tree a time starts a turn,
    the next text is its speaker and what follows up to the next time is what
    was said. Reading starts after the text that begins with `start` (the
    toolbar's «Idioma de la transcripción: …») and stops at `end` («Aviso de
    Orbit», a corner notice: the only thing after the view in the window). Returns the texts up to the first turn, the
    toolbar's included, and the turns.
    """
    header: list[str] = []
    segments: list[Segment] = []
    inside = False
    current: list[str] | None = None

    def close() -> None:
        if current is not None:
            speaker = current[1] if len(current) > 1 else ""
            segments.append(Segment(current[0], speaker, " ".join(current[2:]).strip()))

    for node in nodes:
        texts = node.texts()
        if not inside:
            header.extend(texts)
            inside = any(text.startswith(start) for text in texts)
            continue
        if end in texts:
            break
        for text in texts:
            if TIME.match(text):
                close()
                current = [text]
            elif current is not None:
                current.append(text)
            else:
                header.append(text)
    close()
    return header, segments
