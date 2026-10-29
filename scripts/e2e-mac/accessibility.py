"""Check 21: one read-only macOS AX sample per state, scoped to recording start.

The sidebar replaces Record with its recording card, so two states are needed.
No CuaDriver input or bridge projection can establish this accessibility fact.
"""

from __future__ import annotations

import time
from contextlib import contextmanager

import ax_tree
from checks import _fact
from interface import StepFailed

NAMES = {21: "macOS sees the recording buttons and sidebar links by name"}
NOT_COVERED = ()
LINKS = ("Hoy", "Calendario", "Reuniones", "Notas", "Asistente", "Ajustes")
# Between macOS accessibility reads while WebKit builds its tree; each read is
# a CuaDriver call over SSH, so a shorter pause only adds calls.
AX_RETRY_S = 0.5


def _named(nodes, role, label):
    return any(
        node.role == role
        and any(text == label or text.startswith(label + " ") for text in node.texts())
        for node in nodes
    )


def c21_accessibility(ev: dict) -> tuple[bool, str]:
    sample = _fact(ev, "accessibility")
    for phase, button in (("idle", "Grabar reunión"), ("recording", "Terminar")):
        tree = (sample.get("trees") or {}).get(phase)
        if not tree:
            return False, f"no macOS accessibility tree for {phase}"
        if "AX tree truncated at" in tree:
            return False, f"macOS accessibility tree for {phase} was truncated"
        nodes = ax_tree.parse(tree)
        missing = [label for label in LINKS if not _named(nodes, "AXLink", label)]
        if not _named(nodes, "AXButton", button):
            missing.append(button)
        if missing:
            return False, f"macOS accessibility in {phase}: missing names {missing}"
    return (
        True,
        "macOS names the sidebar links, «Grabar reunión» before recording and «Terminar» during it",
    )


@contextmanager
def session(test):
    """Only this check starts CuaDriver; always use its strict stop afterwards."""
    confirmed = False
    sample = test.ev["accessibility"] = {"trees": {}}

    def capture(phase):
        with test.step(f"macOS accessibility: {phase}", (21,), critical=False):
            if not confirmed:
                raise StepFailed("read-only CuaDriver startup was not confirmed")
            main = test.main
            button = "Grabar reunión" if phase == "idle" else "Terminar"
            label = f"accessibility-{phase}-ready"
            bound = test.bounds["screen"][0]
            test.ui.read_nodes_until(
                main,
                label,
                button,
                lambda nodes: bool(ax_tree.find(nodes, button, ("AXButton",))),
                bound,
            )
            if not ax_tree.find(ax_tree.parse(test.ui.trees[label]), button, ("AXButton",)):
                raise StepFailed(
                    f"the bridge did not show «{button}» within {bound} s; AX was not sampled"
                )
            # WebKit builds its accessibility tree when a client first asks:
            # on the real run of 2026-10-29 the first read held only the
            # window's own buttons. Read again until macOS shows the button.
            started, reads = time.monotonic(), 0
            tree_bound = test.bounds["accessibility-tree"][0]
            while True:
                reads += 1
                done = test.mac.engine_call(
                    "get_window_state",
                    {
                        "pid": main.pid,
                        "window_id": main.window_id,
                        "include_accessibility_tree": True,
                        "include_screenshot": False,
                    },
                )
                result = done["result"]
                if result.get("degraded") or not isinstance(result.get("tree_markdown"), str):
                    raise StepFailed(
                        "CuaDriver could not read macOS accessibility: "
                        f"{result.get('degraded_reason')}"
                    )
                tree = ax_tree.without_menu_bar(result["tree_markdown"])
                shown = ax_tree.find(ax_tree.parse(tree), button, ("AXButton",))
                if shown or time.monotonic() - started > tree_bound:
                    break
                time.sleep(AX_RETRY_S)
            sample["trees"][phase] = tree
            sample.setdefault("reads", {})[phase] = reads
            (test.directory / "trees" / f"accessibility-{phase}.txt").write_text(
                tree, encoding="utf-8"
            )

    test.ev["cua"] = {"sessions": 1}
    test.cua_active = True
    try:
        with test.step("start read-only accessibility check", (21,), critical=False):
            test.ev["cua"].update(test.mac.engine_start(test.bounds["cua-driver"][0]))
            confirmed = True
        yield capture
    finally:
        # Startup can refuse after writing the owned pid (for example, the
        # driver did not accept bounded mode). Stop that process here too.
        with test.step("stop read-only accessibility check", (21,), critical=False):
            test.ev["cua"].update(test.mac.engine_stop(test.bounds["close"][0]))
            test.cua_active = False


CHECKS = {21: c21_accessibility}
