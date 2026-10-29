"""Synthetic device boundaries for the native acceptance dry run only."""

from __future__ import annotations

import json
import time
from contextlib import contextmanager


class DryNative:
    def now(self):
        return time.monotonic() + sum(self.pauses)

    def wall_ms(self):
        return self.example.json("native.json")["quit_at_ms"]

    @contextmanager
    def speaking(self, paths, text):
        class Speech:
            def poll(self):
                return None

        yield Speech()

    def clear_clipboard(self):
        pass

    def audio_state(self, logs, pid=None):
        phase = getattr(self.example, "native_phase", "capture")
        return self.example.json("native.json")["audio"][phase]

    def native_key(self, action, text):
        fixture = self.example.json("native.json")
        if action == "privacy":
            self.example.native_phase = "stopped"
            self.view.screen = "native-stopped"
        elif action == "close":
            self.view.visible = fixture["closed_visible"]
        elif action == "dock":
            self.view.visible = fixture["dock_visible"]
        elif action == "quit":
            self._quit()
            self.view.screen = "native-home"
        elif action in ("text", "undo", "redo", "paste"):
            name = {"text": "typed", "undo": "undo", "redo": "redo", "paste": "paste"}[action]
            self.view.native_value = fixture["edit"][name]
        return 0, json.dumps({"posted": True}), ""
