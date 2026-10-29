"""Orbit Pruebas's main window, past the welcome of a first run.

The app shows its first run («Paso 1 de 5 · Bienvenida») while the webview's
storage lacks `orbit.onboarding.done.local` and the daemon's view says
`onboarding_done` false (AppOnboarding.tsx). «Configurarlo más tarde» leaves
it: it stores that flag and sends `onboarding_done` (OnboardingView.tsx), so
the next openings start on the main window. `prepare` keeps the flag (it
removes only the drafts); a Mac that never finished a first run, or that lost
the flag, opens on the welcome once.

Run 20261025T141530Z: the window was read once, before the webview drew the
welcome, so the welcome was taken for the main window. The bridge is read until
it shows one or the other.
"""

from __future__ import annotations

import time

import ax_tree
from harness_copy import text as copy_text
from interface import StepFailed

# The first run's way out, above every step but the last (onboarding.ts
# `skipAll`, OnboardingFrame.tsx); the dry-run test checks it is the app's.
# The sidebar's button, in every view of the main window (shell.ts `record`).
# What the report says, once, when a run skipped the welcome.
REPORTED = "opened on its first run's welcome"
# Between two reads of a window still drawing its first view. Only spaces the
# reads; the bound is the caller's.
READ_INTERVAL_S = 0.5


def _shows(ui, main, label: str, text: str) -> bool:
    return bool(ax_tree.find(ui.read(main, label, text), text, ("AXButton",)))


def past_the_welcome(ui, main, label: str, first_view_s: float, screen_s: float) -> bool:
    """Reads the main window until it shows the main view or the first run,
    within `first_view_s`. On the first run, presses «Configurarlo más
    tarde» once and waits up to `screen_s` for the main view. Returns whether
    it skipped the welcome."""
    deadline = time.monotonic() + first_view_s
    while True:
        if _shows(ui, main, f"{label} first run", copy_text("onboarding.skipAll")):
            ui.press(main, copy_text("onboarding.skipAll"), ("AXButton",))
            after = f"{label} after the welcome"
            ui.read_until(
                main,
                after,
                copy_text("shell.record"),
                lambda text: copy_text("shell.record") in text,
                screen_s,
            )
            if copy_text("shell.record") not in ui.trees.get(after, ""):
                raise StepFailed(
                    f"«{copy_text('onboarding.skipAll')}» did not lead to the main window («{copy_text('shell.record')}») within {screen_s:.0f} s"
                )
            return True
        if _shows(ui, main, label, copy_text("shell.record")):
            return False
        if time.monotonic() >= deadline:
            raise StepFailed(
                f"the window showed neither the main view («{copy_text('shell.record')}») nor the first run "
                f"(«{copy_text('onboarding.skipAll')}») within {first_view_s:.0f} s"
            )
        time.sleep(READ_INTERVAL_S)
