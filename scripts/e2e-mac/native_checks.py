"""Real native privacy, recording lifecycle and responder actions, after check 21."""

from __future__ import annotations

import json

from checks import _fact
from harness_copy import text as copy_text
from mac_native import OBSERVE_S
from meeting_controls import wait_hub

NAMES = {
    22: "The real privacy shortcut stops capture, playback and transfers across relaunch until local rearm",
    23: "Window close keeps recording; Dock reopening and real Command-Q drain and save its audio",
    24: "Native Undo, Redo, Copy and Paste reach the focused editable field",
}
NOT_COVERED = ()
BOUNDS = {
    "lifecycle-hub": (
        60,
        "provisional: queued audio and closure reaching the isolated hub after quit",
    )
}


def start_recording(test):
    ui, main = test.ui, test.main
    ui.press(main, copy_text("shell.record"), ("AXButton",), first=True)
    ui.press(main, copy_text("meeting.sourceMicrophone"), ("AXRadioButton",), contains=True)
    ui.press(main, copy_text("meeting.record"), ("AXButton",))
    return test.wait_mac("recording")


def exercise(test):
    with test.step("native editing shortcuts in the Assistant composer", (24,), critical=False):
        test.ui.press(test.main, copy_text("shell.assistant"), ("AXLink",))
        typing = test.ui.typing(test.main, copy_text("assistant.composerPrefix"), "", contains=True)
        test.ev["native_edit"] = test.mac.native("edit", json.dumps(typing))
        # The fixture is never sent to the assistant.
        test.ui.type_into(test.main, copy_text("assistant.composerPrefix"), "", contains=True)
    privacy(test)
    lifecycle(test)


def privacy(test):
    with test.step("real privacy brake during recording", (22,)):
        row = test.ev["privacy"] = {"playback": test.mac.native("privacy-playback")}
        local_rearm(test)
        row["recording"] = start_recording(test)
        ident = row["recording"]["id"]
        wait_hub(test, "hub-chunks", ident, lambda k: k["rows"].get("chunks", 0) >= 2)
        row.update(test.mac.native("privacy-stop"))
        row["hub_before"] = test.hub_kept(ident)["rows"].get("chunks", 0)
        row["held"] = test.mac.native("stopped")
        row["hub_held"] = test.hub_kept(ident)["rows"].get("chunks", 0)
        row["exit"] = test.mac.native("quit")
        test.mac.launch(test.bounds["daemon"][0])
        test.main = test.open_main("privacy-reopened")
        row["reopened"] = test.mac.native("stopped")
        row["hub_reopened"] = test.hub_kept(ident)["rows"].get("chunks", 0)
        row["rearmed"] = local_rearm(test)


def local_rearm(test):
    # Local UI action, exactly as a person rearms Orbit. No remote rearm.
    test.ui.press(test.main, copy_text("shell.settings"), ("AXLink",), first=True)
    test.ui.press(
        test.main,
        copy_text("shell.settingsSections.privacy"),
        ("AXLink", "AXButton"),
        contains=True,
        first=True,
    )
    test.ui.press(test.main, copy_text("settings.rearm"), ("AXButton",), first=True)
    state = test.mac.native("rearmed")
    test.wait_mac("paired")
    return state


def lifecycle(test):
    with test.step("close the recording window, reopen from Dock and quit with Command-Q", (23,)):
        row = test.ev["lifecycle"] = {"recording": start_recording(test)}
        ident = row["recording"]["id"]
        row.update(test.mac.native("lifecycle", ident))
        row["hub"] = wait_hub(
            test, "lifecycle-hub", ident, lambda k: k["rows"].get("recording_manifests", 0) == 1
        )
        row["audio"] = test.hub_audio(ident)
        test.mac.launch(test.bounds["daemon"][0])
        test.main = test.open_main("lifecycle-reopened")
        test.ui.reconnect()
        test.wait_mac("paired")


def stopped(state):
    return all(
        state.get(k) is v
        for k, v in {
            "privacy_stopped": True,
            "microphone": False,
            "system_audio": False,
            "playing": False,
        }.items()
    )


def orderly(exit):
    return (
        exit.get("processes", 0) >= 2
        and exit.get("helpers") == 1
        and exit.get("left") == []
        and exit.get("came_back") == 0
    )


def c22_privacy(ev):
    row = _fact(ev, "privacy")
    playback = row.get("playback", {})
    if playback.get("before", {}).get("playing") is not True or not stopped(
        playback.get("after", {})
    ):
        return False, "the real shortcut did not stop active native playback"
    if row.get("before", {}).get("microphone") is not True:
        return False, "capture was not active before the shortcut"
    if not stopped(row.get("stopped", {})):
        return False, "the real privacy shortcut did not stop all audio and persist its latch"
    for stage in ("held", "reopened"):
        watch = row.get(stage, {})
        if (
            watch.get("seconds", 0) < OBSERVE_S
            or not watch.get("samples")
            or not all(stopped(s) for s in watch["samples"])
        ):
            return False, f"privacy did not remain stopped during {stage}"
    if not orderly(row.get("exit", {})):
        return False, "the privacy relaunch did not follow an orderly app/helper exit"
    counts = [row.get(k) for k in ("hub_before", "hub_held", "hub_reopened")]
    if not all(isinstance(n, int) and n > 0 for n in counts) or len(set(counts)) != 1:
        return False, f"new recording transfers continued after the brake: chunk counts {counts}"
    rearmed = row.get("rearmed", {})
    if rearmed != {
        "privacy_stopped": False,
        "microphone": False,
        "system_audio": False,
        "playing": False,
    }:
        return False, "the local rearm failed or restarted capture/playback on its own"
    return (
        True,
        "real shortcut; capture/playback stopped, no new chunks in both watches; local rearm left audio idle",
    )


def c23_lifecycle(ev):
    row = _fact(ev, "lifecycle")
    before, after = row.get("before", {}), row.get("after", {})
    ident = row.get("recording", {}).get("id")
    if (
        not ident
        or before.get("id") != ident
        or after.get("id") != ident
        or after.get("phase") != "recording"
        or after.get("sealed_chunks", 0) <= before.get("sealed_chunks", 0)
        or row.get("hidden") is not False
        or row.get("visible") is not True
    ):
        return False, "closing/reopening the window did not preserve the same progressing recording"
    if row.get("speech_at_quit") is not True or not orderly(row.get("exit", {})):
        return False, "Command-Q did not end the app/helper orderly while fixture speech continued"
    audio = row.get("audio", {})
    if (
        audio.get("closed") is not True
        or audio.get("complete") is not True
        or audio.get("frames", 0) <= 0
        or audio.get("finalized_at_ms", 0) < row.get("quit_at_ms", 0)
    ):
        return (
            False,
            "the test hub lacks the complete readable audio and closure from the live quit",
        )
    return (
        True,
        f"hidden recording advanced; Dock reopened it; Command-Q saved {audio['frames']} readable frames and exited",
    )


def c24_edit(ev):
    values = _fact(ev, "native_edit")
    expected = {
        "typed": "Maple budget",
        "undo": "",
        "redo": "Maple budget",
        "paste": "Maple budgetMaple budget",
    }
    return (
        values == expected,
        f"native editor contents after typing, Undo, Redo and Copy/Paste: {values}",
    )


CHECKS = {22: c22_privacy, 23: c23_lifecycle, 24: c24_edit}
