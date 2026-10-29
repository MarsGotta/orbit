"""The steps of checks 17 and 18 in the real test on the Mac.

18 runs inside the meeting, right after «Oye Deimos» has its answer in the
floating panel (check 7): the system's pointer goes over the panel, stays
there past the answer's own time, and leaves. 17 runs last, once every
other check has its evidence: it quits Orbit Pruebas with the Dock's quit
right after typing a draft in the Assistant's composer, and opens it again
on purpose. Panel/composer reads use the in-app bridge; actual pointer
motion remains pointer.swift, and the quit remains the Apple Event.

Each step works on the RealTest of real_test.py: its interface, its Mac, its
waits and its evidence.
"""

from __future__ import annotations

import json
import time
import uuid

from drafts_panel_checks import ANSWER_WHOLE, QUIT_NAMES, TEXTS

# Margins over the app's own times, with what they cover.
# Over the answer's time (ANSWER_SHOWN_MS): the daemon lets a question go
# on its tick, once a second, and the frame reaches the panel; 10 s is far
# past both, so an answer still shown then is held, not merely not gone yet.
HOLD_MARGIN_S = 10
# Over the grace after the pointer leaves (HOLD_GRACE_MS): the app reads the
# pointer every 250 ms (panel_hover.rs, POLL) and the daemon's tick is a
# second; provisional, not measured.
RELEASE_MARGIN_S = 3


def bounds(rust_constant) -> dict[str, tuple[float, str]]:
    """The waits these steps add to real_test.bounds, from the Rust source."""
    shown_s = rust_constant("crates/orbit-core/src/meeting_question.rs", "ANSWER_SHOWN_MS") / 1000
    grace_s = rust_constant("crates/orbit-core/src/meeting_question/hold.rs", "HOLD_GRACE_MS")
    grace_s /= 1000
    return {
        # The app's own times, read from the source: not waits, the checks'
        # yardsticks.
        "answer-shown": (shown_s, "ANSWER_SHOWN_MS: how long an answer stays by itself"),
        "hold-grace": (grace_s, "HOLD_GRACE_MS: how long it stays once the pointer left"),
        "pointer-held": (
            10,
            (
                "provisional: the app reads the pointer every 250 ms (panel_hover.rs) and "
                "tells the daemon, not measured"
            ),
        ),
        "answer-held": (
            shown_s + HOLD_MARGIN_S,
            (
                f"not a wait on a fact: how long after the answer the panel is read with the "
                f"pointer over it, its own {shown_s:.0f} s (ANSWER_SHOWN_MS) and "
                f"{HOLD_MARGIN_S} s more"
            ),
        ),
        "answer-released": (
            grace_s + RELEASE_MARGIN_S,
            (
                f"the grace after the pointer leaves, {grace_s:.0f} s (HOLD_GRACE_MS), and "
                f"{RELEASE_MARGIN_S} s for the app's reading and the daemon's tick (provisional)"
            ),
        ),
    }


def _record(test, what: str, done: dict) -> None:
    test.ev["waits"].append(
        {"wait": what, "max_s": test.bounds[what][0], "seconds": done.get("seconds")}
    )


def panel_holds_answer(test, panel, answered_at: float) -> None:
    """With the answer of «Oye Deimos» in the panel: the pointer over it
    holds it past its own time, and once the pointer leaves it goes within
    the grace. `answered_at` is when the run saw the answer whole."""
    ui = test.ui
    with test.step("the panel holds its answer under the pointer", (18,), critical=False):
        question = test.ev["question"]["id"]
        held_s = test.bounds["answer-held"][0]
        hold = test.ev["panel_hold"] = {
            "required_s": held_s,
            "shown_s": test.bounds["answer-shown"][0],
            "grace_s": test.bounds["hold-grace"][0],
            "release_bound_s": test.bounds["answer-released"][0],
        }
        held = test.bounds["pointer-held"][0]
        hold["over"] = test.mac.panel_pointer(panel.window_id, "over", question, held)
        _record(test, "pointer-held", hold["over"])
        time.sleep(max(0.0, answered_at + held_s - time.monotonic()))
        hold["held_for_s"] = round(time.monotonic() - answered_at, 3)
        ui.read(panel, "panel-held", None)
        released = test.bounds["answer-released"][0]
        hold["away"] = test.mac.panel_pointer(panel.window_id, "away", question, released)
        _record(test, "answer-released", hold["away"])
        # The daemon let it go; the panel follows its frame.
        test.measure(
            "screen",
            ui.read_until(
                panel,
                "panel-released",
                None,
                lambda text: ANSWER_WHOLE not in text,
                test.bounds["screen"][0],
            ),
        )


def drafts_survive_quitting(test) -> None:
    """A draft typed in the Assistant's composer, the Dock's quit at once,
    Orbit Pruebas opened again: the composer has it back and says
    «Recuperado…»."""
    with test.step("a draft survives the Dock's quit", (17,), critical=False):
        drafts = test.ev["drafts"] = []
        for how in QUIT_NAMES:
            # Invented text, unique to this quit: no earlier draft can be it.
            text = f"Borrador de prueba {uuid.uuid4().hex[:8]}: lo escrito justo antes de salir"
            row = {"quit": how, "text": text}
            drafts.append(row)
            test.ui.press(test.main, "Asistente", ("AXLink",))
            typing = test.ui.typing(test.main, "Mensaje para", text, contains=True)
            typed = json.dumps(typing, ensure_ascii=False)
            screen_s, close_s = test.bounds["screen"][0], test.bounds["close"][0]
            row.update(test.mac.draft_and_quit(how, typed, screen_s, close_s))
            _reopen(test, how, row)


def _reopen(test, how: str, row: dict) -> None:
    """Opens Orbit Pruebas again, on purpose, and reads the composer."""
    row["reopened"] = test.mac.launch(test.bounds["daemon"][0])
    row["previous_window_id"] = test.main.window_id
    test.main = test.open_main(f"home-after-{how}")
    row["reconnected"] = test.ui.reconnect()
    row["app_pid"] = test.ui.app_pid
    test.wait_mac("paired")
    test.ui.press(test.main, "Asistente", ("AXLink",))
    text = row["text"]

    def back(nodes) -> bool:
        said = [t for node in nodes for t in node.texts()]
        return any(text in t for t in said) and TEXTS["recovered"] in said

    test.measure(
        "screen",
        test.ui.read_nodes_until(
            test.main, f"draft-after-{how}", None, back, test.bounds["screen"][0]
        ),
    )
