"""A default-input change during a live meeting, with positive gap evidence.

Mac-side functions use Python 3.9. Only the consented real harness switches
hardware; the dry run uses a synthetic device listing and topic frames.
"""

from __future__ import annotations

import json

NAMES = {20: "Changing the default input keeps the meeting recording and says the gap"}
NOT_COVERED = ()
BOUNDS = {
    "audio-route": (
        30,
        "provisional: route rebuild and the first PCM/gap notice, not measured",
    ),
}


def switch_input(mac, name=None):
    from mac_controls import MacFailure

    current = mac.system.run_checked("SwitchAudioSource", "-c", "-t", "input").strip()
    if name is None:
        inputs = mac.system.run_checked("SwitchAudioSource", "-a", "-t", "input").splitlines()
        others = [i.strip() for i in inputs if i.strip() and i.strip() != current]
        # The Mac's own microphone first («MacBook Pro Microphone», «Micrófono
        # del MacBook Pro»): another input can be a phone's, which Continuity
        # would wake.
        own = [i for i in others if any(word in i for word in ("MacBook", "Built-in", "integrado"))]
        name = next(iter(own or others), None)
    if not name or name == current:
        raise MacFailure("the route check needs a second input device")
    mac.system.run_checked("SwitchAudioSource", "-t", "input", "-s", name)
    observed = mac.system.run_checked("SwitchAudioSource", "-c", "-t", "input").strip()
    if observed != name:
        raise MacFailure("the default input did not change")
    return {"previous": current, "changed": True}


def recovered(state, argument):
    from mac_controls import MacFailure

    expected = json.loads(argument)
    recording = state.get("recording") or {}
    notices = state.get("notices") or {}
    if recording.get("state") != "present" or notices.get("state") != "present":
        return None
    current = (recording.get("value") or {}).get("recording") or {}
    if current.get("id") != expected["id"] or current.get("phase") != "recording":
        raise MacFailure("the input change interrupted or replaced the recording")
    for notice in (notices.get("value") or {}).get("notices", []):
        if notice.get("sequence", 0) <= expected["sequence"]:
            continue
        if notice.get("code") == "capture_cut":
            raise MacFailure("the input change cut the capture")
        duration = (notice.get("data") or {}).get("elapsed_ms")
        if (
            notice.get("code") == "audio_route_recovered"
            and isinstance(duration, int)
            and duration >= 0
            and current.get("gaps", 0) > expected["gaps"]
        ):
            return {
                "recording_id": current["id"],
                "phase": current["phase"],
                "gaps": current["gaps"],
                "sequence": notice["sequence"],
                "code": notice["code"],
                "elapsed_ms": duration,
            }
    return None


def exercise(test):
    with test.step("change the default input during the meeting and restore it", (20,)):
        evidence = test.ev["audio_route"] = {}
        changed = test.mac.switch_input()
        expected = {"id": test.ev["recording"]["id"], "gaps": 0, "sequence": 0}
        try:
            evidence["away"] = test.wait_mac("audio-route", json.dumps(expected))
        finally:
            test.mac.switch_input(changed["previous"])
        away = evidence["away"]
        expected.update(gaps=away["gaps"], sequence=away["sequence"])
        evidence["back"] = test.wait_mac("audio-route", json.dumps(expected))


def c20_route(ev):
    from checks import _fact

    route = _fact(ev, "audio_route")
    ident = _fact(ev, "recording")["id"]
    away, back = route.get("away", {}), route.get("back", {})
    ok = all(
        r.get("recording_id") == ident
        and r.get("phase") == "recording"
        and r.get("code") == "audio_route_recovered"
        and isinstance(r.get("elapsed_ms"), int)
        and r["elapsed_ms"] >= 0
        for r in (away, back)
    )
    ok = ok and back.get("gaps", 0) > away.get("gaps", 0) > 0
    ok = ok and back.get("sequence", 0) > away.get("sequence", 0)
    return (
        ok,
        "both input changes kept the same recording with counted, measured gap notices",
    )


CHECKS = {20: c20_route}
