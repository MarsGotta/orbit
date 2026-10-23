"""The steps of the recording's controls in the real test on the Mac.

«Silenciar» runs inside the meeting the test records, over three lines of the
script's second half. «Terminar sin guardar» throws away a second, short
recording, and «Borrar reunión» deletes the first meeting; both run after the
final screen, when every check that needs the first meeting has its
evidence, so the first twelve checks keep their order and timing. The menu
bar is not read (meeting_control_checks.MENU_BAR_UNREADABLE).

Each step works on the RealTest of real_test.py: its interface, its Mac, its
waits and its evidence.
"""

from __future__ import annotations

import json
import time

import checks
from interface import StepFailed
from meeting_control_checks import TEXTS

# Every wait these steps add: its bound and why it is that size, as
# real_test.bounds says of its own.
BOUNDS = {
    "muted": (
        15,
        "provisional: the helper applying «Silenciar» and the Mac saying so, not measured",
    ),
    "unmuted": (
        15,
        "provisional: the helper applying «Reactivar micrófono» and the Mac saying so, not measured",
    ),
    "hub-chunks": (
        60,
        (
            "provisional: the first chunks sealed (a second each) reaching the test hub, "
            "not measured"
        ),
    ),
    "discarded": (
        60,
        (
            "provisional: the capture stopping without sealing and the list of Notes leaving "
            "it, not measured"
        ),
    ),
    "deleted": (30, "provisional: the list of Notes leaving a deleted meeting, not measured"),
    "hub-deleted": (
        60,
        (
            "provisional: the Mac's order reaching the test hub, sent at once while it is "
            "online, not measured"
        ),
    ),
}
# What each reading of these steps asks CuaDriver for (real_test.READS).
# «Micrófono silenciado» and «Reactivar micrófono» both contain the first; the
# list of meetings after a deletion is read whole: a few rows.
READS = {
    "recording-muted": "icrófono",
    "discard-notice": "descartada",
    "delete-question": "Borrar",
}
# Chunks of the second recording the test hub must hold before it is thrown
# away, so that «nothing of it is kept» is about something it had.
MIN_CHUNKS = 2
# Between two reads of the test hub's database while a wait on it lasts.
# Only spaces the reads; the bound is the wait's.
HUB_READ_INTERVAL_S = 1.0


def wait_hub(test, what: str, recording: str, done) -> dict:
    """What the test hub keeps of `recording`, once `done` holds for it."""
    max_s, _ = test.bounds[what]
    row = {"wait": what, "max_s": max_s}
    test.ev["waits"].append(row)
    start = time.monotonic()
    while True:
        kept = test.hub_kept(recording)
        if done(kept):
            row["seconds"] = round(time.monotonic() - start, 3)
            return kept
        if time.monotonic() - start >= max_s:
            row["failure"] = f"the test hub keeps {json.dumps(kept)}"
            raise StepFailed(f"«{what}» did not arrive within {max_s:.0f} s; {row['failure']}")
        time.sleep(HUB_READ_INTERVAL_S)


def _deleted(kept: dict) -> bool:
    return not kept["rows"] and kept["deleted"]


def second_half(test) -> None:
    """The script's second half, its middle lines with «Silenciar» on."""
    with test.step("second half of the script, up to «Silenciar»"):
        test.mac.say("part2-before")
    _mute(test)
    with test.step("second half of the script, after «Reactivar micrófono»"):
        test.mac.say("part2-after")


def _mute(test) -> None:
    ui, main = test.ui, test.main
    with test.step("«Silenciar» over three lines of the script", (13,), critical=False):
        # In the sidebar's «Grabando» card (RecordingCard.tsx), which comes
        # first in the tree.
        ui.press(main, TEXTS["mute"], ("AXButton",), first=True)
        test.ev["mute"] = {"muted": test.wait_mac("muted")}
        test.measure(
            "screen",
            ui.read_until(
                main,
                "recording-muted",
                READS["recording-muted"],
                lambda text: TEXTS["muted"] in text,
                test.bounds["screen"][0],
            ),
        )
        # Said only once the Mac confirmed it: nothing of these lines may be
        # recorded.
        test.mac.say("part2-muted")
    if "mute" not in test.ev:
        return
    with test.step("«Reactivar micrófono»", (13,), critical=False):
        ui.press(main, TEXTS["unmute"], ("AXButton",), first=True)
        test.ev["mute"]["unmuted"] = test.wait_mac("unmuted")


def discard_second_recording(test) -> None:
    ui, main = test.ui, test.main
    name = "a second recording, thrown away with «Terminar sin guardar»"
    with test.step(name, (14,), critical=False):
        ui.press(main, "Grabar reunión", ("AXButton",), first=True)
        ui.press(main, "Solo mi micrófono", ("AXRadioButton",), contains=True)
        ui.press(main, "Empezar a grabar", ("AXButton",))
        second = test.wait_mac("recording")
        if second.get("id") == test.ev.get("recording", {}).get("id"):
            raise StepFailed("«Empezar a grabar» did not start a second recording")
        evidence = test.ev["discard"] = {"recording": dict(second)}
        evidence["hub_before"] = wait_hub(
            test,
            "hub-chunks",
            second["id"],
            lambda kept: kept["rows"].get("chunks", 0) >= MIN_CHUNKS,
        )
        # Beside «Terminar» in the card; the first press only asks, in place.
        ui.press(main, TEXTS["discard"], ("AXButton",), first=True)
        ui.press(main, TEXTS["discard_confirm"], ("AXButton",))
        # «Reunión descartada» shows five seconds: read before anything waits.
        test.measure(
            "screen",
            ui.read_until(
                main,
                "discard-notice",
                READS["discard-notice"],
                lambda text: TEXTS["discarded"] in text,
                test.bounds["screen"][0],
            ),
        )
        evidence["gone"] = test.wait_mac("discarded", second["id"])
        evidence["hub_after"] = wait_hub(test, "hub-deleted", second["id"], _deleted)
        ui.press(main, "Reuniones", ("AXLink",), first=True)
        _read_list(test, "discard-meetings", second.get("title") or "")


def delete_first_meeting(test) -> None:
    ui, main = test.ui, test.main
    with test.step("«Borrar reunión» from the first meeting's «Más»", (15,), critical=False):
        title = checks._title(test.ev)
        if not title:
            raise StepFailed("the first meeting has no title to find it by in the list")
        identifier = test.ev["recording"]["id"]
        evidence = test.ev["delete"] = {"recording_id": identifier, "title": title}
        evidence["hub_before"] = test.hub_kept(identifier)
        ui.press(main, "Reuniones", ("AXLink",), first=True)
        ui.press(main, title, ("AXLink", "AXButton"), contains=True, first=True)
        test.measure(
            "screen",
            ui.read_until(
                main,
                "delete-note",
                checks.TITLE_FIELD,
                lambda text: title in text,
                test.bounds["screen"][0],
            ),
        )
        # «Más ⌄» in the note's header (MeetingMenu.tsx): its first «Borrar
        # reunión» only asks, in the same menu, with a second one to confirm.
        ui.press(main, TEXTS["meeting_menu"], ("AXButton", "AXPopUpButton"))
        ui.press(main, TEXTS["delete"], ("AXButton",))
        test.measure(
            "screen",
            ui.read_until(
                main,
                "delete-question",
                READS["delete-question"],
                lambda text: TEXTS["delete_question"] in text,
                test.bounds["screen"][0],
            ),
        )
        ui.press(main, TEXTS["delete"], ("AXButton",))
        evidence["gone"] = test.wait_mac("deleted", identifier)
        evidence["hub_after"] = wait_hub(test, "hub-deleted", identifier, _deleted)
        # The app goes back to «Reuniones» by itself.
        _read_list(test, "delete-meetings", title)


def _read_list(test, label: str, title: str) -> None:
    """The list of meetings, read until it is on screen without `title`."""
    test.measure(
        "screen",
        test.ui.read_until(
            test.main,
            label,
            None,
            lambda text: TEXTS["list"] in text and not (title and title in text),
            test.bounds["screen"][0],
        ),
    )
