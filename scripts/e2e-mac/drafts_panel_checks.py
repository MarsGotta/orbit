"""Checks 17 and 18 of the real test on the Mac: a draft across ⌘Q and the
Dock's quit, and the floating panel holding its answer under the pointer.

Pure functions over the evidence the steps of drafts_panel.py collect, as in
checks.py, whose helpers they use.
"""

from __future__ import annotations

import ax_tree
import checks
from checks import _fact, _tree

NAMES = {
    17: "A draft survives ⌘Q and the Dock's quit and comes back with «Recuperado…»",
    18: "The panel holds its answer under the pointer and lets it go after",
}

# The app's own texts (apps/desktop/src/i18n/es/assistant.ts): the
# composer's label, `Mensaje para ${assistantName}`, and `draftRecovered`. The
# dry-run test checks them against that file.
TEXTS = {
    "composer": "Mensaje para",
    "recovered": "Recuperado el borrador que no llegaste a enviar.",
}
# The panel shows this under an answer only once it is whole (PanelAnswer.tsx);
# check 7 reads it too.
ANSWER_WHOLE = "Queda también en tu conversación"
QUIT_NAMES = {
    "cmd-q": "⌘Q",
    "apple-event": "the Dock's quit (the Apple Event «quit»)",
}
STORED = {
    True: "WebKit's storage had it after the quit",
    False: "WebKit's storage did not have it after the quit",
    None: "WebKit's storage could not be read",
}

# What these checks cannot see, said in every report.
NOT_COVERED = (
    (
        "the panel under the pointer on a second screen with another scale: the run's Mac "
        "has one screen, and the mix of scales cannot be made without a second one."
    ),
    (
        "hover events of the webview over the panel: the run warps the pointer "
        "(CGWarpMouseCursorPosition), which sends no mouse event; Orbit reads the "
        "pointer's position itself (panel_hover.rs), and that is what the check exercises."
    ),
    (
        "drafts other than the Assistant's composer (a new calendar event, a note's "
        "summary or private notes, the personality or a skill): reaching them needs the "
        "Calendar permission or the hub's agent configuration, which the run does not "
        "have; both quits use the composer, each with its own draft. The Dock's own menu "
        "is not clicked: the run sends the Apple Event «quit» that menu sends."
    ),
)


def _composer(nodes: list[ax_tree.Node]) -> str | None:
    """What the Assistant's composer holds, or None when it is not on screen."""
    for node in nodes:
        if node.role == "AXTextArea" and TEXTS["composer"] in (node.description or ""):
            return node.value or ""
    return None


def _draft_wrong(ev: dict, row: dict) -> str | None:
    """Why the draft typed right before the quit `row` did not come back, or None."""
    name = QUIT_NAMES[row["quit"]]
    nodes = _tree(ev, f"draft-after-{row['quit']}")
    if row.get("came_back"):
        return (
            f"after {name}, Orbit Pruebas came back by itself ({row['came_back']} processes, "
            "stopped) before the run opened it again"
        )
    value = _composer(nodes)
    if value is None or row["text"] not in value:
        return (
            f"after {name}, the composer came back without the draft typed {row['gap_s']} s "
            f"before quitting; {STORED[row.get('stored')]}"
        )
    if TEXTS["recovered"] not in [text for node in nodes for text in node.texts()]:
        return f"after {name}, the draft came back without «{TEXTS['recovered']}»"
    return None


def c17_drafts(ev: dict) -> tuple[bool, str]:
    rows = _fact(ev, "drafts")
    if [row.get("quit") for row in rows] != list(QUIT_NAMES):
        return False, f"{len(rows)} of the {len(QUIT_NAMES)} quits ran"
    wrong = [why for row in rows if (why := _draft_wrong(ev, row)) is not None]
    if wrong:
        return False, "; ".join(wrong)
    gaps = " and ".join(f"{row['gap_s']} s" for row in rows)
    return True, (
        f"after ⌘Q and after the Dock's quit, each draft, typed {gaps} before quitting, came "
        f"back in the composer with «{TEXTS['recovered']}»"
    )


def c18_panel_hold(ev: dict) -> tuple[bool, str]:
    hold = ev.get("panel_hold")
    if hold is None:
        raise checks.Unchecked("no answer of «Oye Deimos» reached the panel to hold (check 7)")
    over, away = _fact(hold, "over"), _fact(hold, "away")
    answer = _fact(ev, "question").get("answer") or ""
    held = ax_tree.all_text(_tree(ev, "panel-held"))
    after = f"{hold['held_for_s']:.0f} s after it arrived (it shows {hold['shown_s']:.0f} s alone)"
    if hold["held_for_s"] < hold["required_s"]:
        return False, f"the panel was read {after}, before {hold['required_s']:.0f} s"
    if ANSWER_WHOLE not in held or checks._missing_words(answer, held):
        return False, f"with the pointer over it, {after}, the panel no longer shows the answer"
    if ANSWER_WHOLE in ax_tree.all_text(_tree(ev, "panel-released")):
        return False, (
            f"{away['seconds']} s after the pointer left, the daemon let the answer go and "
            "the panel still shows the answer"
        )
    pointer = over["pointer"]
    bounds = pointer.get("bounds") or {}
    return True, (
        f"with the pointer over it, the answer was still there {after}; it went {away['seconds']} s "
        f"after the pointer left (grace {hold['grace_s']:.0f} s, bound "
        f"{hold['release_bound_s']:.0f} s); a {bounds.get('width', 0):.0f}×"
        f"{bounds.get('height', 0):.0f} pt panel on a display at scale {pointer.get('scale')}"
    )


CHECKS = {17: c17_drafts, 18: c18_panel_hold}
