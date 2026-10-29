"""The Mac side of the recording's controls in the real test: the script's
lines said with «Silenciar» on, and the waits on «Silenciar», «Reactivar
micrófono», «Terminar sin guardar» and «Borrar reunión». Also the failure
every Mac-side step raises (`MacFailure`), shared with mac_drafts_panel.py.

Copied to the Mac with mac.py, which imports it; runs with the Mac's python3,
so it stays within Python 3.9. Pure: predicates over what Orbit Pruebas's
socket said (mac.py `Mac.wait`), each returning its result when the fact is
there and None, with what it saw, while it is not.
"""

from __future__ import annotations

import json


class MacFailure(Exception):
    """A fact that ends a wait or a step, with its cause."""


# The lines of the script's third part said with «Silenciar» on: three whole
# lines, from B «Podemos preguntar…» to B «Apuntado…». Most of their words
# are said nowhere else in the script, so check 13 can tell they were never
# recorded; the third part's closing summary repeats the actions they carry,
# so the note still has them.
MUTED_LINES = slice(3, 6)
# Each part said with `mac.py say PART`: a part of the script and its lines.
PARTS = {
    "part1": (0, slice(None)),
    "question": (1, slice(None)),
    "part2-before": (2, slice(None, MUTED_LINES.start)),
    "part2-muted": (2, MUTED_LINES),
    "part2-after": (2, slice(MUTED_LINES.stop, None)),
}


def muted_span(parts: list[list[tuple[str, str]]]) -> dict[str, list[str]]:
    """The third part's lines before «Silenciar», while it is on, and after."""
    return {
        side: [said for _, said in parts[PARTS[name][0]][PARTS[name][1]]]
        for side, name in (
            ("before", "part2-before"),
            ("muted", "part2-muted"),
            ("after", "part2-after"),
        )
    }


def _value(known: dict | None):
    return known.get("value") if known and known.get("state") == "present" else None


def _microphone(muted: bool):
    """«Silenciar» (`muted`) or «Reactivar micrófono» once the Mac confirmed
    it: the topic says `microphone_muted` only after the helper applied it."""

    def predicate(state: dict, _arg):
        value = _value(state.get("recording")) or {}
        phase = (value.get("recording") or {}).get("phase")
        if phase == "recording" and bool(value.get("microphone_muted")) == muted:
            return {"microphone_muted": muted}
        microphone = "muted" if value.get("microphone_muted") else "open"
        state["seen"] = f"the recording is at «{phase}» with its microphone {microphone}"
        return None

    return predicate


def gone(state: dict, ident):
    """The meeting `ident` once it is not being recorded and the list of
    Notes no longer has it («Terminar sin guardar», «Borrar reunión»). Only a
    list read whole says it is gone: a list the daemon or the hub could not
    give says nothing about it."""
    current = (_value(state.get("recording")) or {}).get("recording") or {}
    if current.get("id") == ident and current.get("phase") not in (None, "idle"):
        state["seen"] = f"the recording is still at «{current.get('phase')}»"
        return None
    read = state.get("read") or {}
    if "items" not in read:
        state["seen"] = f"the list of Notes could not be read: {json.dumps(read.get('refusal'))}"
        return None
    items = read["items"]
    if any("hub_refusal" in item for item in items):
        state["seen"] = "the hub did not give its part of the list of Notes"
        return None
    if any(item.get("recording_id") == ident for item in items):
        state["seen"] = f"recording {ident} is still in the list of Notes"
        return None
    return {"gone": ident, "listed": len(items)}


def notes_list(_arg) -> dict:
    """The read of the first page of the list of Notes."""
    return {"kind": "notes_page", "cursor": None}


# The waits these controls add to mac.py's WAITS, in its shape.
CONTROL_WAITS = {
    "muted": (("recording",), _microphone(True), None),
    "unmuted": (("recording",), _microphone(False), None),
    "discarded": (("recording", "notes"), gone, notes_list),
    "deleted": (("notes",), gone, notes_list),
}


def script_parts(text: str) -> list[list[tuple[str, str]]]:
    """The three parts, each a list of (voice, line); «---» separates them."""
    parts: list[list[tuple[str, str]]] = [[]]
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line == "---":
            parts.append([])
            continue
        voice, separator, said = line.partition(":")
        if not separator or voice.strip() not in ("A", "B") or not said.strip():
            raise MacFailure(f"script line without «A:» or «B:»: {line[:60]}")
        parts[-1].append((voice.strip(), said.strip()))
    if len(parts) != 3 or not all(parts):
        raise MacFailure(
            f"the script must have three parts separated by «---» and has {len(parts)}"
        )
    return parts
