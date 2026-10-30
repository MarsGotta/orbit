"""Check 25: a disposable meeting with independently routed speech on both sides."""

from __future__ import annotations

import re
import unicodedata

from checks import _fact
from harness_copy import text as copy_text
from interface import StepFailed
from mac_remote_audio import PLAYER, phrases, require_silent
from meeting_controls import TEXTS

NAMES = {25: "Independent remote speech is stored on its own side and survives microphone mute"}
NOT_COVERED = ()


def exercise(test):
    if "remote_preflight" not in test.ev:
        return
    # A separate meeting leaves checks 1–24's scripts, times and sides intact.
    with test.step("independent remote meeting and microphone mute", (25,), critical=False):
        row = test.ev["remote_audio"] = {}
        try:
            player = row["player"] = test.mac.remote_audio("prepare")
            row["route"] = player["route"]
            ui, main = test.ui, test.main
            ui.press(main, copy_text("shell.record"), ("AXButton",), first=True)
            ui.press(
                main,
                copy_text("meeting.sourceApplication"),
                ("AXRadioButton",),
                contains=True,
            )
            test.measure(
                "screen",
                ui.read_until(
                    main,
                    "remote-app-picker",
                    None,
                    lambda text: PLAYER in text,
                    test.bounds["screen"][0],
                ),
            )
            ui.press(main, PLAYER, ("AXButton",))
            ui.press(main, copy_text("meeting.record"), ("AXButton",))
            row["recording"] = test.wait_mac("recording")
            if row["recording"].get("audio") != {"application": {"application_pid": player["pid"]}}:
                raise StepFailed(
                    f"expected application_pid {player['pid']}, got audio {row['recording'].get('audio')}"
                )
            for phase in ("mic", "remote"):
                test.mac.remote_audio(phase)
            ui.press(main, TEXTS["mute"], ("AXButton",), first=True)
            row["muted"] = test.wait_mac("muted")
            test.measure(
                "screen",
                ui.read_until(
                    main,
                    "remote-muted",
                    None,
                    lambda text: TEXTS["muted"] in text,
                    test.bounds["screen"][0],
                ),
            )
            row["muted_ui"] = True
            for phase in ("mic-muted", "remote-muted"):
                test.mac.remote_audio(phase)
            ui.press(main, TEXTS["unmute"], ("AXButton",), first=True)
            row["unmuted"] = test.wait_mac("unmuted")
            _finish(test, row)
            ident = row["recording"]["id"]
            test.wait_mac("note", ident)
            row["stored"] = test.hub_remote_audio(ident)
        finally:
            try:
                if row.get("recording") and not row.get("finish_requested"):
                    _finish(test, row)
            finally:
                row.update(test.mac.remote_audio("stop"))


def _finish(test, row):
    # Mark before dispatch: a missing receipt must never replay an input action.
    row["finish_requested"] = True
    test.ui.press(test.main, copy_text("meeting.finish"), ("AXButton",), first=True)
    row["saved"] = test.wait_mac("saved", row["recording"]["id"])


def _plain(text):
    text = re.sub(r"seven hundred|setecientos", "700", text.casefold())
    return " ".join(
        re.findall(
            r"[^\W_]+",
            "".join(
                c
                for c in unicodedata.normalize("NFD", text.casefold())
                if not unicodedata.combining(c)
            ),
        )
    )


def c25_remote(ev):
    from mac_controls import MacFailure

    row = _fact(ev, "remote_audio")
    try:
        require_silent(row.get("route", {}))
    except MacFailure as cause:
        return False, str(cause)
    pid = row.get("player", {}).get("pid")
    if not pid or row.get("recording", {}).get("audio") != {
        "application": {"application_pid": pid}
    }:
        return False, "the fixture application's capture selection was not confirmed"
    if not (
        row.get("restored")
        and row.get("muted_ui")
        and row.get("muted", {}).get("microphone_muted") is True
        and row.get("unmuted", {}).get("microphone_muted") is False
    ):
        return (
            False,
            "microphone mute, its UI span, unmute or fixture cleanup was not confirmed",
        )
    stored = row.get("stored", {})
    tracks = stored.get("tracks", {})
    if not (
        stored.get("closed")
        and stored.get("complete")
        and set(tracks) == {"mic_local", "remote_app"}
    ):
        return False, "the hub lacks a complete, closed two-track recording"
    for name, track in tracks.items():
        if not (
            track.get("frames", 0) > 0
            and track.get("chunks", 0) > 0
            and track.get("nonzero")
            and track.get("sha256")
        ):
            return False, f"stored {name} audio is empty, silent or lacks a checksum"
    if tracks["mic_local"]["sha256"] == tracks["remote_app"]["sha256"]:
        return False, "the two stored tracks contain duplicate PCM"
    sides = {
        side: _plain(" ".join(s["text"] for s in stored.get("segments", []) if s["track"] == side))
        for side in tracks
    }
    script = phrases(ev.get("language", "es"))
    for phase, side in (
        ("mic", "mic_local"),
        ("remote", "remote_app"),
        ("remote-muted", "remote_app"),
    ):
        phrase = _plain(script[phase])
        other = next(s for s in sides if s != side)
        if phrase not in sides[side] or phrase in sides[other]:
            return (
                False,
                f"{phase} phrase is missing from stored {side} or leaked to {other}",
            )
    # Any distinctive content of the muted mic phrase is a leak, even partial ASR.
    forbidden = set(_plain(script["mic-muted"]).split()) - set(
        _plain(" ".join(script[p] for p in ("mic", "remote", "remote-muted"))).split()
    )
    if forbidden.intersection(" ".join(sides.values()).split()):
        return False, "muted microphone words leaked into the hub's stored transcript"
    return (
        True,
        "distinct stored mic_local/remote_app PCM and phrases; remote speech survives microphone mute; silent fixture stopped",
    )


CHECKS = {25: c25_remote}
