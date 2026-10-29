"""Every check of the real test on the Mac, run over the evidence.

The checks live by subject: checks.py (1 to 12), meeting_control_checks.py
(13 to 16), drafts_panel_checks.py (17 and 18), silence_recording.py (19), audio_route.py (20), and accessibility.py (21).
This runs them in order and says what none of them can see.
"""

from __future__ import annotations

import accessibility
import audio_route
import checks
import drafts_panel_checks
import meeting_control_checks
import silence_recording
from checks import Unchecked, failure_lines, result
from harness_copy import language_scope

MODULES = (
    checks,
    meeting_control_checks,
    drafts_panel_checks,
    silence_recording,
    audio_route,
    accessibility,
)
CHECKS = {number: check for module in MODULES for number, check in module.CHECKS.items()}
# What the run cannot see, said in every report.
NOT_COVERED = tuple(text for module in MODULES for text in module.NOT_COVERED)


def run_checks(ev: dict) -> list[dict]:
    """Every result. A step that failed fails the check it serves; the
    checks it left without evidence say which step stopped them."""
    with language_scope(ev.get("language", "es")):
        return _run_checks(ev)


def _run_checks(ev: dict) -> list[dict]:
    names = {number: name for module in MODULES for number, name in module.NAMES.items()}
    failed_steps = [s for s in ev.get("steps", []) if not s.get("ok", True)]
    failures = failure_lines(ev)
    results = []
    for number, check in CHECKS.items():
        own = next((s for s in failed_steps if number in s.get("checks", ())), None)
        if own is not None:
            res = result(number, "fail", f"step «{own['name']}»: {own['cause']}", names)
        else:
            try:
                ok, detail = check(ev)
                res = result(number, "pass" if ok else "fail", detail, names)
            except meeting_control_checks.Skipped as cause:
                res = result(number, "skipped", str(cause), names)
            except Unchecked as cause:
                stop = failed_steps[0] if failed_steps else None
                reason = (
                    f"the test stopped at «{stop['name']}»: {stop['cause']}" if stop else str(cause)
                )
                res = result(number, "unchecked", reason, names)
        if res["status"] not in ("pass", "skipped") and number != 10:
            # What the logs said while this check's steps ran.
            windows = [s for s in ev.get("steps", []) if number in s.get("checks", ())]
            res["log"] = [
                {k: f[k] for k in ("log", "code", "op")}
                for f in failures
                if f.get("ts") is not None
                and any(s["start"] <= f["ts"] <= s.get("end", float("inf")) for s in windows)
            ]
        results.append(res)
    return results
