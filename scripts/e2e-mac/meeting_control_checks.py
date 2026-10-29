"""The checks of the recording's controls, numbered after the first twelve.

13 «Silenciar» in the recording, 14 «Terminar sin guardar» on a second
recording, 15 «Borrar reunión» on the first meeting at the end of the run,
and 16 the menu bar, read through the in-app test bridge.
Pure functions over the evidence, as in checks.py, whose helpers they use.
"""

from __future__ import annotations

import json
import re

import ax_tree
import checks
from checks import _fact, _transcript, _tree
from harness_copy import Aliases, Messages
from harness_copy import text as copy_text

NAMES = Messages(
    {
        13: "«{recordingMute.mute}» leaves its stretch out of the recording and the note marks it",
        14: "«{discard.action}» leaves no note and nothing of the recording, here or on the hub",
        15: "«{noteScreen.deleteMeeting}» asks first and leaves nothing of the meeting, here or on the hub",
        16: "The menu bar offers «{native.mute}» and «{native.discard}» and shows the time",
    }
)

# What these checks cannot see, said in every report after checks.NOT_COVERED.
NOT_COVERED = (
    "«{recordingMute.mute}» and «{discard.action}» outside the sidebar recording card, and whether «{recordingCard.wakePhrase}» is heard while muted: the run presses only the card's controls.",
)


class Skipped(Exception):
    """What the check looks at cannot be seen by the harness at all: the
    reason says why, and the check is not passed."""


# The app's own texts (apps/desktop/src/i18n/es): recording.ts
# `recordingMute` and `discard`, notes.ts `mutedMicrophone`, the list's
# `section`, `refresh`, `loadMore`, its day groups and what it says before
# it is read (`loadingIndex`, `indexFailed`, `needsHub`),
# notices/capture.ts `recording_discarded`, noteScreen.ts `meetingMenu`,
# `deleteMeeting`, `deleteQuestion` and `ready`. The dry-run test checks each
# against those files.
TEXTS = Aliases(
    {
        "mute": "recordingMute.mute",
        "unmute": "recordingMute.unmute",
        "muted": "recordingMute.muted",
        "muted_line": "notes.mutedPrefix",
        "discard": "discard.action",
        "discard_confirm": "discard.confirm",
        "discarded": "notices.recording_discarded.text",
        "note_ready": "noteScreen.ready",
        "list": "shell.notes",
        "refresh": "notes.refresh",
        "load_more": "notes.loadMore",
        "today": "notes.today",
        "yesterday": "notes.yesterday",
        "this_week": "notes.thisWeek",
        "last_30_days": "notes.last30Days",
        "before": "notes.before",
        "list_loading": "notes.loadingIndex",
        "list_failed": "notes.indexFailed",
        "list_needs_hub": "notes.needsHub",
        "meeting_menu": "noteScreen.meetingMenu",
        "delete": "noteScreen.deleteMeeting",
        "delete_question": "noteScreen.deleteQuestion",
    }
)
DAY_KEYS = ("today", "yesterday", "this_week", "last_30_days", "before")
UNREAD_KEYS = ("list_loading", "list_failed", "list_needs_hub")
# The code the daemon logs when «Terminar sin guardar» took (NoticeCode).
DISCARDED_CODE = "recording_discarded"


def _words(lines: list[str]) -> set[str]:
    return set(checks.WORD.findall(ax_tree.plain(" ".join(lines))))


def _own_words(lines: list[str], script: list[str]) -> set[str]:
    """The words of `lines` said nowhere else in the script."""
    return _words(lines) - _words([line for line in script if line not in lines])


def _muted_line(lines: list[str], duration_ms: int) -> str | None:
    """Why the one muted-microphone line is wrong, or None."""
    if len(lines) != 1:
        return f"the transcript shows {len(lines)} muted-microphone lines, not one"
    pattern = re.escape(copy_text("notes.mutedMicrophone"))
    pattern = pattern.replace(re.escape("{start}"), r"(\S+)").replace(re.escape("{end}"), r"(\S+)")
    found = re.fullmatch(pattern, lines[0])
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
            f"""with the microphone muted, the «{copy_text("meeting.recording")}» card does not say «{copy_text("recordingMute.muted")}»""",
        )
    if not any(n.role == "AXButton" and TEXTS["unmute"] in n.texts() for n in card):
        return (
            False,
            f"""with the microphone muted, the card offers no «{copy_text("recordingMute.unmute")}»""",
        )
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
        return (
            False,
            f"{len(back)} words said while muted are in the transcript: {', '.join(back)}",
        )
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
        f"the card said «{copy_text('recordingMute.muted')}»; «{lines[0]}»; none of the {len(muted)} words "
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


def _subtree(nodes: list[ax_tree.Node], start: int) -> list[ax_tree.Node]:
    """The node at `start` and every node under it."""
    end = start + 1
    while end < len(nodes) and nodes[end].depth > nodes[start].depth:
        end += 1
    return nodes[start:end]


def _texts(nodes: list[ax_tree.Node]) -> list[str]:
    return [text for node in nodes for text in node.texts()]


def listed(nodes: list[ax_tree.Node]) -> list[str] | None:
    """The meetings «Reuniones» lists, by name, or None when the list was
    not read whole.

    WebKit gives macOS no group named after the list's label («Lista de
    reuniones», MeetingsColumn.tsx; real run of 2026-10-23), so the list is
    read from what the tree shows: the «Reuniones» heading followed by
    «Actualizar», then, as its siblings, a heading per day and under each a
    link per meeting whose first text is its name. The first other sibling
    (the open note's breadcrumb, the main area) ends it. A list with no rows
    is read, and empty, unless it says it is still being read or could not
    be; a link before any day heading is not a row.
    """
    for at, node in enumerate(nodes):
        if node.role != "AXHeading" or TEXTS["list"] not in _texts(_subtree(nodes, at)):
            continue
        siblings, after = [], at + len(_subtree(nodes, at))
        while after < len(nodes) and nodes[after].depth == node.depth:
            siblings.append(_subtree(nodes, after))
            after += len(siblings[-1])
        if not siblings or not (
            siblings[0][0].role == "AXButton" and TEXTS["refresh"] in siblings[0][0].texts()
        ):
            continue
        names: list[str] = []
        day = False
        for sibling in siblings[1:]:
            first, texts = sibling[0], _texts(sibling)
            if first.role == "AXHeading" and any(
                text in (TEXTS[key] for key in DAY_KEYS) for text in texts
            ):
                day = True
            elif first.role == "AXLink" and day:
                names.append(sibling[1].text() if len(sibling) > 1 else first.text())
            elif not (first.role == "AXButton" and TEXTS["load_more"] in texts):
                if not names and any(text in (TEXTS[key] for key in UNREAD_KEYS) for text in texts):
                    return None
                break
        return names
    return None


def note_still_open(nodes: list[ax_tree.Node], title: str) -> bool:
    """The note of the meeting `title` is open: its title field names it."""
    return any(
        node.role == "AXTextField"
        and copy_text("noteScreen.titleLabel") in node.texts()
        and node.value == title
        for node in nodes
    )


def _gone_from_list(ev: dict, label: str, title: str) -> str | None:
    """Why the list «Reuniones» read as `label` does not show `title` gone, or None."""
    names = listed(_tree(ev, label))
    if names is None:
        return (
            f"«{label}» does not show the list of meetings read whole («{TEXTS['list']}» "
            f"with «{TEXTS['refresh']}», then its days and meetings)"
        )
    if title and title in names:
        return f"«{title}» is still in the list of meetings"
    return None


def c14_discard(ev: dict) -> tuple[bool, str]:
    discard = _fact(ev, "discard")
    _fact(discard, "gone")
    if TEXTS["discarded"] not in ax_tree.all_text(_tree(ev, "discard-notice")):
        logged = any(f.get("code") == DISCARDED_CODE for f in _logged(ev))
        return False, (
            f"""«{copy_text("notices.recording_discarded.text")}» was not on screen right after «{copy_text("discard.confirm")}» (it shows """
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
        return (
            False,
            f"«{copy_text('noteScreen.ready')}» appeared after the recording was discarded ({ready})",
        )
    title = (discard.get("recording") or {}).get("title") or ""
    wrong = _gone_from_list(ev, "discard-meetings", title) or _left_on_hub(
        "discarded recording", discard
    )
    if wrong:
        return False, wrong
    chunks = discard["hub_before"]["rows"].get("chunks", 0)
    return True, (
        f"«{TEXTS['discarded']}»; no note; gone from «{TEXTS['list']}»; the {chunks} chunks the hub had "
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
        return (
            False,
            f"""the first «{copy_text("noteScreen.deleteMeeting")}» did not ask «{copy_text("noteScreen.deleteQuestion")}» first""",
        )
    title = deletion.get("title") or ""
    wrong = _gone_from_list(ev, "delete-meetings", title) or _left_on_hub(
        "deleted meeting", deletion
    )
    if wrong:
        return False, wrong
    if title and note_still_open(_tree(ev, "delete-meetings"), title):
        return (
            False,
            f"the note of «{title}» is still open beside the list after its deletion",
        )
    kinds = len(deletion["hub_before"]["rows"])
    return True, (
        f"asked first; gone from «{TEXTS['list']}»; the hub's rows of it in {kinds} tables are gone "
        "and its tombstone is kept"
    )


def c16_menu_bar(ev: dict) -> tuple[bool, str]:
    menus = _fact(ev, "menus")
    if not menus.get("ok"):
        return False, "the test bridge refused menus: " + json.dumps(menus)
    tray = menus.get("tray") or {}
    rows = tray.get("rows") or []
    for name in (copy_text("native.mute"), copy_text("native.discard")):
        matches = [row for row in rows if row.get("name") == name]
        if len(matches) != 1 or matches[0].get("enabled") is not True:
            return (
                False,
                f"the menu bar has {len(matches)} enabled/disabled rows named «{name}»: {matches}",
            )
    title = tray.get("title") or ""
    if ax_tree.seconds(title) is None:
        return False, f"the menu bar title has no recording time: «{title}»"
    return (
        True,
        f"enabled «{copy_text('native.mute')}» and «{copy_text('native.discard')}»; recording time «{title}»",
    )


CHECKS = {13: c13_mute, 14: c14_discard, 15: c15_delete, 16: c16_menu_bar}
