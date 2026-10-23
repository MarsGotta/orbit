"""The checks of the recording's controls, numbered after the first twelve.

13 «Silenciar» in the recording, 14 «Terminar sin guardar» on a second
recording, 15 «Borrar reunión» on the first meeting at the end of the run,
and 16 the menu bar, which the harness cannot read (see MENU_BAR_UNREADABLE).
Pure functions over the evidence, as in checks.py, whose helpers they use.
"""

from __future__ import annotations

import json
import re

import ax_tree
import checks
from checks import _fact, _transcript, _tree

NAMES = {
    13: "«Silenciar» leaves its stretch out of the recording and the note marks it",
    14: "«Terminar sin guardar» leaves no note and nothing of the recording, here or on the hub",
    15: "«Borrar reunión» asks first and leaves nothing of the meeting, here or on the hub",
    16: "The menu bar offers «Silenciar» and «Terminar sin guardar…» and shows the time",
}

# What these checks cannot see, said in every report after checks.NOT_COVERED.
NOT_COVERED = (
    (
        "«Silenciar» and «Terminar sin guardar» anywhere but the sidebar's «Grabando» card "
        "(the floating panel, the menu bar, a call with Deimos), and that «Oye Deimos» is "
        "not heard while muted: the run presses the card's alone."
    ),
)


class Skipped(Exception):
    """What the check looks at cannot be seen by the harness at all: the
    reason says why, and the check is not passed."""


# The app's own texts (apps/desktop/src/i18n/es): recording.ts
# `microphoneMute` and `discard`, notes.ts `mutedMicrophone` and `listLabel`,
# notices/capture.ts `recording_discarded`, noteScreen.ts `meetingMenu`,
# `deleteMeeting`, `deleteQuestion` and `ready`. The dry-run test checks each
# against those files.
TEXTS = {
    "mute": "Silenciar",
    "unmute": "Reactivar micrófono",
    "muted": "Micrófono silenciado",
    "muted_line": "Micrófono silenciado · ",
    "discard": "Terminar sin guardar",
    "discard_confirm": "Borrar y terminar",
    "discarded": "Reunión descartada",
    "note_ready": "Tu nota está lista",
    "list": "Lista de reuniones",
    "meeting_menu": "Más",
    "delete": "Borrar reunión",
    "delete_question": "¿Borrar esta reunión?",
}
# «Micrófono silenciado · 12:04 – 12:30»: from the meeting's start (mmss).
MUTED_SPAN = re.compile(r"^Micrófono silenciado · (\S+) – (\S+)$")
# The code the daemon logs when «Terminar sin guardar» took (NoticeCode).
DISCARDED_CODE = "recording_discarded"
MENU_BAR_UNREADABLE = (
    "the harness cannot read Orbit Pruebas's menu bar item: CuaDriver lists layer-0 "
    "windows only and the status item lives in the menu bar's own window, with no "
    "accessibility window to read by id; macOS's menu bar is cut from every read before "
    "it is parsed or saved, because under the Apple menu it names the user's own "
    "documents (real run 36); and an open native menu is not in the window's tree, where "
    "only real key events reach it (real run 13). The menu's rows and the time in its "
    "title are covered by the app's tray tests (apps/desktop/src-tauri/src/tray/tests.rs), "
    "not on a Mac."
)


def _words(lines: list[str]) -> set[str]:
    return set(checks.WORD.findall(ax_tree.plain(" ".join(lines))))


def _own_words(lines: list[str], script: list[str]) -> set[str]:
    """The words of `lines` said nowhere else in the script."""
    return _words(lines) - _words([line for line in script if line not in lines])


def _muted_line(lines: list[str], duration_ms: int) -> str | None:
    """Why the one «Micrófono silenciado · …» line is wrong, or None."""
    if len(lines) != 1:
        return f"the transcript shows {len(lines)} «Micrófono silenciado · …» lines, not one"
    found = MUTED_SPAN.match(lines[0])
    start, end = (ax_tree.seconds(t) for t in found.groups()) if found else (None, None)
    if start is None or end is None or not start < end or end * 1000 > duration_ms:
        return f"«{lines[0]}» is not a stretch within the recording ({duration_ms // 1000} s)"
    return None


def c13_mute(ev: dict) -> tuple[bool, str]:
    span = _fact(ev, "muted_script")
    card = _tree(ev, "recording-muted")
    if TEXTS["muted"] not in [text for node in card for text in node.texts()]:
        return (
            False,
            "with the microphone muted, the «Grabando» card does not say «Micrófono silenciado»",
        )
    if not any(n.role == "AXButton" and TEXTS["unmute"] in n.texts() for n in card):
        return False, "with the microphone muted, the card offers no «Reactivar micrófono»"
    before, segments = _transcript(ev)
    texts = before + [s.text for s in segments]
    lines = [t for t in texts if t.startswith(TEXTS["muted_line"])]
    wrong = _muted_line(lines, int(_fact(ev, "recording")["duration_ms"]))
    if wrong:
        return False, wrong
    heard = _words([s.text for s in segments])
    script = [*_fact(ev, "script"), *span["muted"]]
    back = sorted(_own_words(span["muted"], script) & heard)
    if back:
        return False, f"{len(back)} words said while muted are in the transcript: {', '.join(back)}"
    for side in ("before", "after"):
        own = _own_words(span[side], script)
        recall = len(own & heard) / len(own) if own else 0.0
        if recall < checks.MINIMUM_RECALL:
            missing = sorted(own - heard)
            return False, (
                f"only {recall:.0%} of the words said just {side} the muted stretch come back "
                f"(minimum {checks.MINIMUM_RECALL:.0%}); missing: {', '.join(missing[:10])}"
            )
    muted = _own_words(span["muted"], script)
    return True, (
        f"the card said «Micrófono silenciado»; «{lines[0]}»; none of the {len(muted)} words "
        "said only while muted came back, and the words around it did"
    )


def _left_on_hub(name: str, evidence: dict) -> str | None:
    """Why the hub still has something of the meeting, or None."""
    before = (evidence.get("hub_before") or {}).get("rows") or {}
    after = evidence.get("hub_after") or {}
    if not before:
        return f"the test hub had nothing of the {name} before it was deleted: nothing was checked"
    if after.get("rows") or not after.get("deleted"):
        return (
            f"the test hub keeps rows of the {name} {after.get('rows')} "
            f"(tombstone: {bool(after.get('deleted'))})"
        )
    return None


def _gone_from_list(ev: dict, label: str, title: str) -> str | None:
    """Why the list «Reuniones» read as `label` does not show `title` gone, or None."""
    nodes = _tree(ev, label)
    if not any(TEXTS["list"] in node.texts() for node in nodes):
        return f"«{label}» does not show the list of meetings («{TEXTS['list']}»)"
    if title and title in ax_tree.all_text(nodes):
        return f"«{title}» is still on screen in the list of meetings"
    return None


def c14_discard(ev: dict) -> tuple[bool, str]:
    discard = _fact(ev, "discard")
    _fact(discard, "gone")
    if TEXTS["discarded"] not in ax_tree.all_text(_tree(ev, "discard-notice")):
        logged = any(f.get("code") == DISCARDED_CODE for f in _logged(ev))
        return False, (
            "«Reunión descartada» was not on screen right after «Borrar y terminar» (it shows "
            f"5 s); the daemon {'did' if logged else 'did not'} log «{DISCARDED_CODE}»"
        )
    trees = ev.get("trees", {})
    ready = [
        label
        for label, text in trees.items()
        if label.startswith(("discard-", "delete-"))
        and TEXTS["note_ready"] in ax_tree.all_text(ax_tree.parse(text))
    ]
    if ready:
        return False, f"«Tu nota está lista» appeared after the recording was discarded ({ready})"
    title = (discard.get("recording") or {}).get("title") or ""
    wrong = _gone_from_list(ev, "discard-meetings", title) or _left_on_hub(
        "discarded recording", discard
    )
    if wrong:
        return False, wrong
    chunks = discard["hub_before"]["rows"].get("chunks", 0)
    return True, (
        f"«Reunión descartada»; no note; gone from «Reuniones»; the {chunks} chunks the hub had "
        "are gone with everything else, and its tombstone is kept"
    )


def _logged(ev: dict) -> list[dict]:
    """Every line of the Mac's report logs that carries a code."""
    rows = []
    for text in (ev.get("logs") or {}).values():
        for line in text.splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if isinstance(row, dict) and row.get("code"):
                rows.append(row)
    return rows


def c15_delete(ev: dict) -> tuple[bool, str]:
    deletion = _fact(ev, "delete")
    _fact(deletion, "gone")
    question = ax_tree.all_text(_tree(ev, "delete-question"))
    if TEXTS["delete_question"] not in question:
        return False, "the first «Borrar reunión» did not ask «¿Borrar esta reunión?» first"
    wrong = _gone_from_list(ev, "delete-meetings", deletion.get("title") or "") or _left_on_hub(
        "deleted meeting", deletion
    )
    if wrong:
        return False, wrong
    kinds = len(deletion["hub_before"]["rows"])
    return True, (
        f"asked first; gone from «Reuniones»; the hub's rows of it in {kinds} tables are gone "
        "and its tombstone is kept"
    )


def c16_menu_bar(_ev: dict) -> tuple[bool, str]:
    raise Skipped(MENU_BAR_UNREADABLE)


CHECKS = {13: c13_mute, 14: c14_discard, 15: c15_delete, 16: c16_menu_bar}
