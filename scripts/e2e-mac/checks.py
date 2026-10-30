"""The checks of the real test on the Mac, as pure functions.

The first twelve are here; those of the recording's controls («Silenciar»,
«Terminar sin guardar», «Borrar reunión», the menu bar) are in
meeting_control_checks.py, those of a draft across quitting and of the panel
under the pointer in drafts_panel_checks.py, and all_checks.py runs them all.
Each one reads the evidence the run collected — the accessibility trees of
what was on screen, what the daemon said over its socket, the report logs of
the Mac and the log of the test hub — and says
«pass», «fail» with its cause, «unchecked» when the run never got to it,
naming the step that stopped it, or «skipped» when the harness cannot see
what it checks at all, with that reason (skipped is not passed). No check is
ever softened to pass: one that fails on the Mac is an Orbit defect.
"""

from __future__ import annotations

import json
import re
from datetime import UTC, datetime

import ax_tree
import meeting_language
from harness_copy import Aliases, Messages
from harness_copy import text as copy_text
from legacy_ui import REMOVED, REMOVED_PATTERNS

NAMES = Messages(
    {
        1: "The transcript arrives in {pass.language}",
        2: "The times are relative to the start and do not exceed the duration",
        3: "The local microphone appears under its name",
        4: "No repeated or empty entries",
        5: "The note is written with a summary and actions, and «{noteScreen.steps.transcript.active}» goes away",
        6: "«{noteScreen.open}», from «{noteScreen.ready}», and the list of Notes open that note",
        7: "«{recordingCard.wakePhrase}» asks a question and the answer appears in the panel",
        8: "The Assistant answers and reopens an earlier conversation with its messages",
        9: "No problem notice in the top bar and none of the texts the interface removed",
        10: "No unexpected failure in the logs",
        11: "A voice note is transcribed and Deimos answers",
        12: "The call opens and listens",
    }
)
# What the run cannot see, said in every report.
NOT_COVERED = (
    (
        "that the call opens a turn with the synthetic voice: with voice processing, macOS "
        "lowers the volume of every other app, and the voice of `say` through BlackHole "
        "arrives at a tenth, under the threshold (measured on 2026-10-17). That the call "
        "hears a real voice was checked with a person's own voice that day."
    ),
)

# What a check of the transcript compares against the script. A word is a run
# of five or more letters: short words are the ones speech recognition most
# often drops or splits, and the content words of the script are longer.
WORD = re.compile(r"[a-zñ]{5,}")
# Share of the script's words that must come back. The audio is synthetic
# speech, read without noise straight into the recording; a recognizer working
# in the meeting language returns nearly all of it, and one that switched language (a
# failure seen in an earlier run) returns almost none. The margin is for names
# and numbers the recognizer writes differently («doce mil» as «12.000»).
MINIMUM_RECALL = 0.8


PILL = Aliases(
    {
        key: "problem." + key
        for key in (
            "orbitLost",
            "privacyStopped",
            "unpaired",
            "revoked",
            "offline",
            "missingPermissions",
        )
    }
)

# Report-log classes that mean something went wrong: «fixable» and the
# generic «failed». «working», «done» and «silent» are normal life.
FAILURE_CLASSES = ("failed", "fixable")
FAILURE_LEVELS = ("ERROR", "CRITICAL")


def result(number: int, status: str, detail: str, names: dict[int, str] = NAMES) -> dict:
    return {"number": number, "name": names[number], "status": status, "detail": detail}


class Unchecked(Exception):
    """The evidence a check needs was never collected."""


def passed(results: list[dict]) -> bool:
    """Whether the run passed: no check failed or was left unchecked. A
    skipped check is said in the report as such; it neither passes nor fails."""
    return all(r["status"] in ("pass", "skipped") for r in results)


def _tree(ev: dict, label: str) -> list[ax_tree.Node]:
    text = ev.get("trees", {}).get(label)
    if text is None:
        raise Unchecked(f"the screen «{label}» was never read")
    return ax_tree.parse(text)


def _fact(ev: dict, key: str):
    value = ev.get(key)
    if value is None:
        raise Unchecked(f"«{key}» is missing")
    return value


def language_of(how: str | None) -> str | None:
    """The language on the note's header button: the part after the template."""
    parts = how.split(" · ") if how else []
    return parts[1] if len(parts) > 1 else None


def _transcript(ev: dict) -> tuple[list[str], list[ax_tree.Segment]]:
    before, segments = ax_tree.transcript(
        _tree(ev, "note-transcript"),
        copy_text("notes.transcriptHelp"),
        copy_text("shell.notice"),
    )
    return before, segments


def c1_language(ev: dict) -> tuple[bool, str]:
    before, segments = _transcript(ev)
    if not segments:
        return (
            False,
            f"""the {copy_text("noteScreen.transcriptTab")} tab shows no entry""",
        )
    # The header's button ends with the meeting's language.
    how = next((t for t in before if t.startswith(copy_text("noteScreen.howPrefix"))), None)
    language = language_of(how)
    if language != copy_text("notes.languages." + ev.get("language", "es")):
        return (
            False,
            f"the note does not show {copy_text('pass.language')}: «{how or 'none'}»",
        )
    script = " ".join(_fact(ev, "script"))
    said = " ".join(s.text for s in segments)
    back, of, recall = _words_back(script, said)
    if recall < MINIMUM_RECALL:
        # Where the words were lost tells a late start from a lost stretch
        # (2026-10-20: two runs began at 00:25 and one repeated itself).
        heard = set(WORD.findall(ax_tree.plain(said)))
        missing = [w for w in dict.fromkeys(WORD.findall(ax_tree.plain(script))) if w not in heard]
        return False, (
            f"only {back} of {of} words of the script come back "
            f"({recall:.0%}; minimum {MINIMUM_RECALL:.0%}); the first entry starts at "
            f"{segments[0].time}; {len(missing)} missing, the first: {', '.join(missing[:10])}"
        )
    return (
        True,
        f"{copy_text('pass.language')}; {back} of {of} words come back ({recall:.0%})",
    )


def _words_back(script: str, said: str) -> tuple[int, int, float]:
    """How many of the script's words (WORD) come back in what was
    transcribed, of how many, and their share."""
    expected = set(WORD.findall(ax_tree.plain(script)))
    heard = set(WORD.findall(ax_tree.plain(said)))
    back = len(expected & heard)
    return back, len(expected), back / len(expected) if expected else 0.0


def c2_times(ev: dict) -> tuple[bool, str]:
    _, segments = _transcript(ev)
    if not segments:
        return (
            False,
            f"""the {copy_text("noteScreen.transcriptTab")} tab shows no entry""",
        )
    duration_ms = int(_fact(ev, "recording")["duration_ms"])
    bad = []
    for number, segment in enumerate(segments, 1):
        value = ax_tree.seconds(segment.time)
        if value is None or value * 1000 > duration_ms:
            bad.append(f"entry {number}: «{segment.time}»")
    if bad:
        return False, (
            f"{len(bad)} of {len(segments)} times exceed the duration "
            f"({duration_ms // 1000} s) or cannot be read: " + ", ".join(bad[:5])
        )
    return True, f"{len(segments)} times between 0 and {duration_ms // 1000} s"


def c3_name(ev: dict) -> tuple[bool, str]:
    _, segments = _transcript(ev)
    if not segments:
        return (
            False,
            f"""the {copy_text("noteScreen.transcriptTab")} tab shows no entry""",
        )
    # The account's name as the daemon takes it (`id -F`, mac.py prepare),
    # never the hub's, which only repeats what the Mac sent it.
    owner = _fact(ev, "mac").get("owner") or ""
    # The recording is microphone only: every turn is the local microphone.
    others = [s.speaker for s in segments if s.speaker != owner]
    if others:
        return False, (
            f"{len(others)} of {len(segments)} entries of the local microphone do not carry "
            f"the name «{owner}»; observed names: {sorted(set(others))}"
        )
    return True, f"all {len(segments)} entries say «{owner}»"


def c4_repeated(ev: dict) -> tuple[bool, str]:
    _, segments = _transcript(ev)
    if not segments:
        return (
            False,
            f"""the {copy_text("noteScreen.transcriptTab")} tab shows no entry""",
        )
    empty = [n for n, s in enumerate(segments, 1) if not s.text.strip()]
    seen: dict[str, int] = {}
    repeated = []
    for number, segment in enumerate(segments, 1):
        key = ax_tree.plain(segment.text).strip()
        if not key:
            continue
        if key in seen:
            repeated.append(f"{seen[key]} and {number}")
        else:
            seen[key] = number
    if empty or repeated:
        return False, (
            f"of {len(segments)} entries, {len(empty)} empty ({empty[:5]}) and "
            f"{len(repeated)} repeated ({repeated[:5]})"
        )
    return True, f"{len(segments)} entries, none empty or repeated"


def _note(ev: dict) -> dict:
    """The note as the user sees it, from the meeting's document."""
    note = _fact(ev, "detail").get("note") or {}
    if note.get("state") != "ready" or not note.get("note"):
        state = {k: note.get(k) for k in ("state", "attempts", "error")}
        raise ValueError(f"the daemon has no written note: {json.dumps(state)[:200]}")
    return note["note"]


def _title(ev: dict) -> str:
    """The meeting's title: the list of Notes and the opened note both name
    the note by it (decided on 2026-10-17), never by the title the note was
    written with."""
    return _fact(ev, "detail").get("title") or ""


def c5_note(ev: dict) -> tuple[bool, str]:
    phase = _fact(ev, "note_phase")
    if phase.get("phase") != "ready":
        return False, f"the note never became ready: {json.dumps(phase)}"
    try:
        note = _note(ev)
    except ValueError as cause:
        return False, str(cause)
    sections = [s for s in note.get("sections") or [] if (s.get("markdown") or "").strip()]
    actions = note.get("action_items") or []
    if not sections or not actions:
        return (
            False,
            f"the note has {len(sections)} sections with text and {len(actions)} actions",
        )
    summary = ax_tree.all_text(_tree(ev, "note-summary"))
    if copy_text("noteScreen.summaryLater") in summary or any(
        _missing_words(s["markdown"], summary) for s in sections
    ):
        return (
            False,
            f"the {copy_text('noteScreen.summaryTab')} tab does not show the note's {len(sections)} sections",
        )
    tasks = ax_tree.all_text(_tree(ev, "note-actions"))
    shown = [a for a in actions if a["text"] in tasks]
    if not shown:
        return (
            False,
            f"the {copy_text('noteScreen.actionsTab')} tab shows none of the {len(actions)} actions",
        )
    after = ("note-ready", "note-summary", "note-actions", "note-transcript")
    still = [
        label
        for label in after
        if ax_tree.plain(copy_text("noteScreen.steps.transcript.active")).rstrip(".…")
        in ax_tree.plain(ev.get("trees", {}).get(label, ""))
    ]
    if still:
        return (
            False,
            f"with the note ready, «{copy_text('noteScreen.steps.transcript.active')}» is still on screen ({', '.join(still)})",
        )
    return True, (
        f"{len(sections)} sections and {len(actions)} actions ({len(shown)} shown); "
        f"""«{copy_text("noteScreen.steps.transcript.active")}» is gone"""
    )


def c6_open_note(ev: dict) -> tuple[bool, str]:
    title = _title(ev)
    if not title:
        return False, "the note has no title to recognise it by"
    bad = []
    for label, origin in (
        ("note-from-notice", "the notice"),
        ("note-from-list", "the list"),
    ):
        # The note's own title field, not the row that names it in the list.
        fields = [
            n
            for n in _tree(ev, label)
            if n.role == "AXTextField" and copy_text("noteScreen.titleLabel") in n.texts()
        ]
        if not any(node.value == title for node in fields):
            bad.append(origin)
    if bad:
        return False, f"from {' and '.join(bad)} the note «{title}» does not open"
    return True, f"from the notice and from the list «{title}» opens"


def c7_wake_phrase(ev: dict) -> tuple[bool, str]:
    question = _fact(ev, "question")
    if question.get("origin") != "voice":
        return (
            False,
            f"the question did not come from the voice: origin «{question.get('origin')}»",
        )
    if question.get("stage") != "answered":
        return False, (
            f"the question ended «{question.get('stage')}» "
            f"({json.dumps(question.get('cause'), ensure_ascii=False)})"
        )
    if ax_tree.plain(meeting_language.question()) not in ax_tree.plain(
        question.get("question") or ""
    ):
        return (
            False,
            f"the question heard is not the script's: «{question.get('question')}»",
        )
    panel = ax_tree.all_text(_tree(ev, "panel"))
    if copy_text("floatingPanel.alsoInConversation") not in panel:
        return False, "the panel does not show the answer as whole"
    for failure in (
        copy_text("meetingQuestion.failed"),
        copy_text("meetingQuestion.notSent"),
        copy_text("meetingQuestion.thinking").rstrip(".…"),
    ):
        if failure in panel:
            return False, f"the panel says «{failure}»"
    missing = _missing_words(question.get("answer") or "", panel)
    if missing is None:
        return False, "Deimos answered without text"
    if missing:
        return False, f"the panel does not show the answer: {missing} missing"
    return True, "question heard, answer in the panel"


def _missing_words(answer: str, screen: str) -> list[str] | None:
    """The three longest words of `answer` that `screen` does not show.

    The screen renders Markdown, so the answer is matched by its words rather
    than as one string."""
    words = sorted(set(WORD.findall(ax_tree.plain(answer))), key=lambda w: (-len(w), w))
    if not words:
        return None
    shown = ax_tree.plain(screen)
    return [w for w in words[:3] if w not in shown]


def c8_assistant(ev: dict) -> tuple[bool, str]:
    chat = _fact(ev, "chat")
    message, answer = chat.get("message") or "", chat.get("answer") or ""
    for label, moment in (
        ("assistant-answered", "on answering"),
        ("assistant-reopened", "on opening the earlier conversation"),
    ):
        screen = ax_tree.all_text(_tree(ev, label))
        if message not in screen:
            return False, f"{moment}, the Assistant does not show the message sent"
        missing = _missing_words(answer, screen)
        if missing is None:
            return False, "Deimos answered without text"
        if missing:
            return (
                False,
                f"{moment}, the Assistant does not show the answer: {missing} missing",
            )
    return True, "answer received; the earlier conversation opens with its messages"


def c9_no_noise(ev: dict) -> tuple[bool, str]:
    trees = ev.get("trees", {})
    if "final" not in trees:
        raise Unchecked("the screen «final» was never read")
    seen = []
    for label, text in trees.items():
        screen = ax_tree.all_text(ax_tree.parse(text))
        for removed in (*REMOVED, *PILL.values()):
            if removed in screen:
                seen.append(f"«{removed}» in {label}")
        permission = re.escape(copy_text("problem.missingPermission")).replace(
            re.escape("{name}"), r"[^\n]+"
        )
        if re.search(permission, screen):
            seen.append(f"permission notice in {label}")
        for pattern in REMOVED_PATTERNS:
            if (found := pattern.search(screen)) is not None:
                seen.append(f"«{found.group(0)}» in {label}")
    if seen:
        return False, "; ".join(seen[:8])
    return True, f"{len(trees)} screens read with no problem notice and no removed text"


def c11_voice_note(ev: dict) -> tuple[bool, str]:
    note = _fact(ev, "voice_note")
    if note.get("failure") is not None:
        return False, (
            "the voice note was not transcribed: "
            f"{json.dumps(note['failure'], ensure_ascii=False)[:200]}"
        )
    if note.get("role") != "user":
        return (
            False,
            f"the voice note's turn is not the user's: role «{note.get('role')}»",
        )
    # The phrase opens with the assistant's name, which is not what this
    # check measures: in English Whisper writes «Deimos» as «Demos» (real
    # run 20261029T215955Z). Only the words after it are counted.
    phrase = note.get("phrase") or ""
    addressed = re.match(r"\s*Deimos\s*,\s*", phrase)
    back, of, recall = _words_back(
        phrase[addressed.end() :] if addressed else phrase, note.get("transcript") or ""
    )
    if recall < MINIMUM_RECALL:
        return False, (
            f"the voice note's transcript brings only {back} of {of} words of the phrase "
            f"({recall:.0%}; minimum {MINIMUM_RECALL:.0%}): «{note.get('transcript')}»"
        )
    if not note.get("complete") or not (note.get("answer") or "").strip():
        return False, (
            f"Deimos did not answer the voice note (complete answer: {bool(note.get('complete'))}, "
            f"{len(note.get('answer') or '')} characters)"
        )
    seconds = (note.get("duration_ms") or 0) / 1000
    return True, (
        f"voice note of {seconds:.1f} s; {back} of {of} words come back ({recall:.0%}); "
        "Deimos answered"
    )


def _call_audio(ev: dict) -> list[dict]:
    """The call's lines of daemon.log (mac.py logs), parsed."""
    rows = []
    for line in (ev.get("call_audio") or "").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict):
            rows.append(row)
    return rows


def c12_call(ev: dict) -> tuple[bool, str]:
    _fact(ev, "call")
    if "logs" not in ev or "call_audio" not in ev:
        raise Unchecked("the Mac's logs were not collected")
    # The call's own failure; check 10 looks at every other one.
    starts = [f for f in failure_lines(ev) if (f.get("data") or {}).get("step") == "audio_start"]
    if starts:
        return False, "audio_start failed: " + "; ".join(
            f"code {(f.get('data') or {}).get('code')} "
            f"({(f.get('data') or {}).get('cause')}, {(f.get('data') or {}).get('status')}), "
            f"op {f['op']}"
            for f in starts[:3]
        )
    rows = _call_audio(ev)
    opened = [r.get("fields") or {} for r in rows if r.get("event") == "call_audio_opened"]
    heard = [r.get("fields") or {} for r in rows if r.get("event") == "call_audio_heard"]
    if not opened:
        return False, "daemon.log has no call_audio_opened: the microphone did not open"
    if not heard:
        return (
            False,
            "daemon.log has no call_audio_heard: the call did not say what it heard",
        )
    last_heard = heard[-1]
    frames = str(last_heard.get("frames"))
    if not frames.isdigit() or int(frames) == 0:
        return False, f"the call got to listen and heard {frames} frames"
    last_opened = opened[-1]
    return True, (
        f"listens with the microphone open ({last_opened.get('input_channels')} channels at "
        f"{last_opened.get('sample_rate')} Hz) and heard {frames} frames (loudest RMS "
        f"{last_heard.get('loudest_rms')}, threshold {last_heard.get('speech_rms_threshold')})"
    )


def failure_codes(notice_rs: str) -> set[str]:
    """The codes `NoticeCode::is_failure` names besides the `failed` class:
    failures that are silent on screen (crates/orbit-core/src/notice/mod.rs). Read
    from the source so the test follows the one place that decides it."""
    start = notice_rs.index("pub const fn is_failure(self) -> bool {")
    body = notice_rs[start : notice_rs.index("\n    }\n", start)]
    names = re.findall(r"Self::([A-Z]\w+)", body)
    return {re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower() for name in names}


def failure_lines(ev: dict) -> list[dict]:
    """Every failure the logs recorded, with when, where, code and op."""
    silent = set(ev.get("failure_codes") or ())
    failures = []
    for name, text in sorted(ev.get("logs", {}).items()):
        for line in text.splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                failures.append({"log": name, "code": "unreadable_line", "op": None, "ts": None})
                continue
            if row.get("class") in FAILURE_CLASSES or row.get("code") in silent:
                failures.append(
                    {
                        "log": name,
                        "code": row.get("code"),
                        "op": row.get("op"),
                        "class": row.get("class"),
                        "data": row.get("data"),
                        "ts": _instant(row.get("ts"), utc=True),
                    }
                )
    for line in (ev.get("hub_log") or "").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            # uvicorn's own lines are plain text; only the hub's are JSON.
            if "Traceback" in line or line.startswith("ERROR"):
                failures.append({"log": "hub", "code": "unformatted", "op": None, "ts": None})
            continue
        if row.get("level") in FAILURE_LEVELS:
            failures.append(
                {
                    "log": "hub",
                    "code": row.get("code") or row.get("event"),
                    "op": row.get("op"),
                    "event": row.get("event"),
                    "ts": _instant(row.get("ts"), utc=False),
                }
            )
    return failures


def _instant(text: object, utc: bool) -> float | None:
    """Seconds since the epoch. The Mac writes UTC with a «Z»; the hub writes
    the local time of the machine that runs it, which is this one."""
    if not isinstance(text, str):
        return None
    try:
        moment = datetime.fromisoformat(text)
    except ValueError:
        return None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=UTC) if utc else moment.astimezone()
    return moment.timestamp()


# What the real privacy shortcut (check 22) causes by design, from the moment
# it is pressed: commands refused because Orbit is stopped, the stop kept
# across the relaunch, the recording it cut, and that recording's pending work
# at the final close (real run 20261030T115848Z).
BRAKE_STEP = "real privacy brake during recording"
AFTER_BRAKE = {"privacy_stop_kept", "recording_closed_after_cut", "pending_work_keep_open"}


def _caused_by_the_brake(failure: dict, ev: dict) -> bool:
    pressed = next((s["start"] for s in ev.get("steps", []) if s["name"] == BRAKE_STEP), None)
    if pressed is None or failure.get("ts") is None or failure["ts"] < pressed:
        return False
    if failure.get("code") == "helper_command_failed":
        return (failure.get("data") or {}).get("code") == "privacy_stopped"
    return failure.get("code") in AFTER_BRAKE


def c10_no_failures(ev: dict) -> tuple[bool, str]:
    if "logs" not in ev:
        raise Unchecked("the Mac's logs were not collected")
    failures = [f for f in failure_lines(ev) if not _caused_by_the_brake(f, ev)]
    if failures:
        return False, "; ".join(_describe(f, ev) for f in failures[:10])
    names = ", ".join(sorted(ev["logs"])) + (", hub" if ev.get("hub_log") is not None else "")
    return True, f"no failure in {names}"


def _describe(failure: dict, ev: dict) -> str:
    step = step_at(ev, failure.get("ts"))
    where = f" during «{step}»" if step else ""
    return f"{failure['log']}: code {failure['code']}, op {failure['op']}{where}"


def step_at(ev: dict, instant: float | None) -> str | None:
    if instant is None:
        return None
    for step in ev.get("steps", []):
        if step["start"] <= instant <= step.get("end", float("inf")):
            return step["name"]
    return None


CHECKS = {
    1: c1_language,
    2: c2_times,
    3: c3_name,
    4: c4_repeated,
    5: c5_note,
    6: c6_open_note,
    7: c7_wake_phrase,
    8: c8_assistant,
    9: c9_no_noise,
    10: c10_no_failures,
    11: c11_voice_note,
    12: c12_call,
}
