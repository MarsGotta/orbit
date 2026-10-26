"""Positive notice evidence for the silent recording check. Mac-side Python 3.9."""

from __future__ import annotations

from mac_controls import MacFailure


def silence_question(state: dict, ident):
    """A current question while this recording is still running, never an
    old question followed by its resolution or an absent topic."""
    recording = state.get("recording") or {}
    notices = state.get("notices") or {}
    if recording.get("state") != "present" or notices.get("state") != "present":
        return None
    current = (recording.get("value") or {}).get("recording") or {}
    if current.get("id") != ident:
        return None
    if current.get("phase") in ("idle", "interrupted", "finalizing"):
        raise MacFailure("the silent recording ended before its question was observed")
    relevant = [
        n
        for n in (notices.get("value") or {}).get("notices", [])
        if n.get("code") in ("recording_silence_question", "recording_silence_resolved")
    ]
    if (
        current.get("phase") == "recording"
        and relevant
        and relevant[-1]["code"] == "recording_silence_question"
    ):
        return {"code": relevant[-1]["code"], "recording_id": ident}
    state["seen"] = "recording continues without a current silence question"
    return None
