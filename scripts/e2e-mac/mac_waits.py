"""What each wait of the real test's Mac side waits for: the predicates
over what Orbit Pruebas's socket said, and the table `Mac.wait` (mac.py) reads
them from, the recording controls' (mac_controls.py) and the drafts' and
panel's (mac_drafts_panel.py) included.

Copied to the Mac with mac.py, which imports it; runs with the Mac's python3,
so it stays within Python 3.9. Pure: no socket, no process, no clock.
"""

from __future__ import annotations

import json

from audio_route import recovered
from harness_copy import language
from harness_copy import text as copy_text
from mac_controls import CONTROL_WAITS, MacFailure, notes_list
from mac_drafts_panel import PANEL_WAITS
from mac_native import progress
from mac_silence import silence_question


def _value(known: dict | None):
    if known and known.get("state") == "present":
        return known.get("value")
    return None


# ── What each wait waits for ────────────────────────────────────────────────
#
# A predicate looks at the latest knowledge of each topic and returns the
# result when the fact is there, None to keep waiting, or raises MacFailure
# when what it sees means the fact will not come.


def paired(state: dict, _arg):
    value = _value(state.get("status")) or {}
    if value.get("hub") == "revoked":
        raise MacFailure("the test hub revoked this Mac")
    return {"hub": "online"} if value.get("hub") == "online" else None


def recording(state: dict, _arg):
    current = (_value(state.get("recording")) or {}).get("recording") or {}
    if current.get("phase") == "recording":
        return {k: current.get(k) for k in ("id", "title", "audio", "started_at_ms")}
    return None


# Causes that do not pass while the recording goes on.
_NOT_LISTENING = (
    "macos_version",
    "locale_unsupported",
    "unavailable",
    "failed",
    "helper_without_phrase",
    "no_microphone",
    "privacy_stopped",
)


def listening(state: dict, _arg):
    voice = (_value(state.get("recording")) or {}).get("voice_trigger") or {}
    if voice.get("state") == "listening":
        return {"voice_trigger": "listening"}
    why = (voice.get("why") or {}) if voice.get("state") == "not_listening" else {}
    if why.get("cause") in _NOT_LISTENING:
        raise MacFailure(
            f"«{copy_text('recordingCard.wakePhrase')}» is not listening ({'en-US' if language() == 'en' else 'es-ES'} on-device speech): {json.dumps(why, ensure_ascii=False)}"
        )
    state["seen"] = f"on-device speech ({language()}): {json.dumps(voice, ensure_ascii=False)}"
    return None


def _turn(state: dict) -> dict:
    """The assistant's turn in progress, or the last one: what the user said
    and Deimos's answer; the conversation itself is the hub's."""
    return (_value(state.get("assistant")) or {}).get("turn") or {}


def question_open(state: dict, _arg):
    value = _value(state.get("recording")) or {}
    question = value.get("question") or {}
    if question.get("origin") == "voice":
        return {"id": question.get("id")}
    trigger = value.get("voice_trigger") or {}
    if trigger.get("state") in ("abandoned", "not_listening"):
        raise MacFailure(f"no voice question opened: {json.dumps(trigger)}")
    state["seen"] = "no voice question opened"
    return None


def answer(state: dict, _arg):
    """The voice question once Deimos answered it or it failed. Until then it
    says what it saw: no question was heard, the question's stage, or an
    answer that has not come whole."""
    question = (_value(state.get("recording")) or {}).get("question") or {}
    if question.get("origin") != "voice":
        state["seen"] = (
            f"""no voice question: «{copy_text("recordingCard.wakePhrase")}» opened none"""
            if not question
            else f"a question of origin «{question.get('origin')}», not voice"
        )
        question_open(state, None)
        return None
    if question.get("stage") not in ("answered", "failed"):
        state["seen"] = f"the voice question is at «{question.get('stage')}»"
        return None
    result = {k: question.get(k) for k in ("id", "question", "origin", "stage", "cause")}
    if question["stage"] == "failed":
        result["answer"] = ""
        return result
    # The answer of the turn whose request id is the question's.
    message = _turn(state).get("answer") or {}
    if message.get("id") != f"{question.get('id')}:assistant" or not message.get("complete"):
        state["seen"] = (
            "the question is answered and its answer has not arrived whole yet "
            f"(turn {message.get('id')}, complete: {bool(message.get('complete'))})"
        )
        return None
    result["answer"] = message.get("text") or ""
    return result


# The note states after which nothing more comes without the user
# (orbit_core::notes::NoteState): written, given up, or no model on the hub.
FINAL_NOTE = ("ready", "failed", "not_configured")


def _row(state: dict, ident: str) -> dict | None:
    """The meeting `ident` in the latest read of the list of Notes
    (`notes_page`). A list the hub could not give is noted as what was seen,
    and the wait goes on: the hub may come back."""
    read = state.get("read") or {}
    if "refusal" in read:
        state["seen"] = f"the list of Notes could not be read: {json.dumps(read['refusal'])}"
        return None
    seen = f"recording {ident} is not in the list of Notes"
    for item in read.get("items") or []:
        if "hub_refusal" in item:
            seen = f"the hub did not give its part of the list: {json.dumps(item['hub_refusal'])}"
        elif item.get("recording_id") == ident:
            return item
    state["seen"] = seen
    return None


def saved(state: dict, ident):
    current = (_value(state.get("recording")) or {}).get("recording") or {}
    if current.get("id") == ident and current.get("phase") not in (None, "idle"):
        if current.get("phase") == "interrupted":
            raise MacFailure("the recording ended interrupted")
        state["seen"] = f"the recording is still at «{current.get('phase')}»"
        return None
    row = _row(state, ident)
    if row is None:
        return None
    local = row.get("local") or {}
    if row.get("capture") != "closed" or local.get("phase") == "recording":
        state["seen"] = (
            f"the list of Notes has it open (capture «{row.get('capture')}», "
            f"local «{local.get('phase')}»)"
        )
        return None
    return {k: row.get(k) for k in ("recording_id", "title", "duration_ms", "capture")}


def note(state: dict, ident):
    row = _row(state, ident)
    if row is None:
        return None
    headline = row.get("note") or {}
    if headline.get("state") in FINAL_NOTE:
        return {
            "phase": headline["state"],
            "short_summary": headline.get("short_summary"),
            "recording_id": row.get("recording_id"),
            "title": row.get("title"),
        }
    state["seen"] = f"transcript «{row.get('transcript')}», note «{headline.get('state', 'none')}»"
    return None


def chat(state: dict, text):
    assistant = _value(state.get("assistant")) or {}
    turn = assistant.get("turn") or {}
    user = turn.get("user") or {}
    if user.get("role") != "user" or user.get("text") != text:
        return None
    reply = turn.get("answer") or {}
    if assistant.get("turn_in_flight") or not reply.get("complete"):
        state["seen"] = "the turn is still in progress"
        return None
    return {"session": assistant.get("session_id"), "answer": reply.get("text") or ""}


def session(state: dict, ident):
    current = (_value(state.get("assistant")) or {}).get("session_id")
    state["seen"] = f"the open conversation is {current}"
    return {"session": current} if current == ident else None


def voice_note(state: dict, _arg):
    """The voice note's turn once Deimos answered it, or once a fact says no
    answer will come: its transcription failed, or the turn ended without a
    complete answer. The turn with a voice note is the test's: the turn
    before it is the Assistant's typed message, which has none."""
    assistant = _value(state.get("assistant")) or {}
    turn = assistant.get("turn") or {}
    user = turn.get("user") or {}
    voice = user.get("voice")
    if not isinstance(voice, dict):
        state["seen"] = "the turn in progress is not a voice note"
        return None
    reply = turn.get("answer") or {}
    result = {
        "role": user.get("role"),
        "transcript": voice.get("transcript"),
        "duration_ms": voice.get("duration_ms"),
        "audio_available": voice.get("audio_available", False),
        "failure": voice.get("failure"),
        "answer": reply.get("text") or "",
        "complete": bool(reply.get("complete")),
    }
    if result["failure"] is not None:
        return result
    if result["transcript"] is None:
        state["seen"] = "the voice note is being transcribed"
        return None
    if assistant.get("turn_in_flight"):
        state["seen"] = "Deimos has not finished answering"
        return None
    return result


def call_listening(state: dict, _arg):
    """The call once it listens: the daemon says `listening` only after the
    helper confirmed the microphone open with voice processing (CallReady).
    A call seen and then gone ended before it listened."""
    known = (_value(state.get("assistant")) or {}).get("call") or {}
    if known.get("state") != "present":
        state["seen"] = f"the call is «{known.get('state')}»"
        return None
    call = known.get("value")
    if call is None:
        if state.get("call_phase") is not None:
            raise MacFailure(
                f"the call ended without reaching «listening» (last phase «{state['call_phase']}»)"
            )
        state["seen"] = "there is no call"
        return None
    state["call_phase"] = call.get("phase")
    if call.get("phase") == "listening":
        return {k: call.get(k) for k in ("id", "phase", "started_at_ms", "expires_at_ms")}
    state["seen"] = f"the call is still at «{call.get('phase')}»"
    return None


# What each wait subscribes to, its predicate, and, for a fact only a read
# says, the read it repeats (on a change of the `notes` head and at least
# every READ_INTERVAL_S).
WAITS = {
    "recording-progress": (("recording",), progress, None),
    "audio-route": (("recording", "notices"), recovered, None),
    "paired": (("status",), paired, None),
    "recording": (("recording",), recording, None),
    "listening": (("recording",), listening, None),
    "question-open": (("recording",), question_open, None),
    "answer": (("recording", "assistant"), answer, None),
    "saved": (("recording", "notes"), saved, notes_list),
    "note": (("notes",), note, notes_list),
    "chat": (("assistant",), chat, None),
    "session": (("assistant",), session, None),
    "voice-note": (("assistant",), voice_note, None),
    "call-listening": (("assistant",), call_listening, None),
    "silence-question": (("recording", "notices"), silence_question, None),
    "silence-saved": (("recording", "notes"), saved, notes_list),
    **CONTROL_WAITS,
    **PANEL_WAITS,
}
