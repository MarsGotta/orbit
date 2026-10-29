"""Real keys and audio/lifecycle observations for checks 22–24. Python 3.9."""

from __future__ import annotations

import json
from pathlib import Path

from mac_controls import MacFailure

KEYS = Path(__file__).resolve().with_name("keys.swift")
# Each is below the two-minute silence question; these are provisional
# scheduling/observation budgets, not product latency claims.
OBSERVE_S = 5
STATE_S = 10
# The 32-second process-exit patience covers the current 17-second daemon
# join plus AppKit teardown; the 60-second playback patience covers fixture
# synthesis/startup. Both are provisional harness bounds, never audio limits.


def progress(state, argument):
    expected = json.loads(argument)
    known = state.get("recording") or {}
    if known.get("state") != "present":
        return None
    recording = (known.get("value") or {}).get("recording") or {}
    if (
        recording.get("id") == expected["id"]
        and recording.get("phase") == "starting"
        and "sealed_chunks" not in expected
    ):
        return None
    if recording.get("id") != expected["id"] or recording.get("phase") != "recording":
        raise MacFailure("closing the window interrupted or replaced the recording")
    count = recording.get("sealed_chunks", 0)
    if count > expected.get("sealed_chunks", -1):
        return {"id": recording["id"], "sealed_chunks": count, "phase": "recording"}
    return None


class NativeChecks:
    def native_key(self, action, text=None):
        command = ["swift", str(KEYS), str(self._app_pid()), action]
        if text is not None:
            command.append(text)
        self.system.run_checked(*command)

    def audio_state(self, expected, max_s=STATE_S):
        start = self.system.now()
        while True:
            state = self.system.audio_state(self.paths.logs, self._app_pid())
            if state and all(state.get(k) is v for k, v in expected.items()):
                return state
            if self.system.now() - start >= max_s:
                raise MacFailure(f"helper audio state did not become {expected}: {state}")
            self.system.pause(0.1)

    def native(self, action, argument=None):
        if action == "edit":
            return self.native_edit(json.loads(argument))
        if action == "privacy-playback":
            return self.native_playback()
        if action == "privacy-stop":
            return self.native_privacy()
        if action == "stopped":
            with self.system.speaking(
                self.paths, "Privacy remains stopped during this invented speech."
            ):
                start = self.system.now()
                samples = []
                for _ in range(OBSERVE_S):
                    samples.append(
                        self.audio_state(
                            {
                                "privacy_stopped": True,
                                "microphone": False,
                                "system_audio": False,
                                "playing": False,
                            }
                        )
                    )
                    self.system.pause(1)
                return {"samples": samples, "seconds": self.system.now() - start}
        if action == "quit":
            return self.native_quit()
        if action == "lifecycle":
            return self.native_lifecycle(argument)
        if action == "rearmed":
            return self.audio_state(
                {
                    "privacy_stopped": False,
                    "microphone": False,
                    "system_audio": False,
                    "playing": False,
                }
            )
        raise MacFailure(f"unknown native acceptance action {action}")

    def native_playback(self):
        self.activate()
        with self.system.silent_output():
            try:
                reply = self.orbit.call(
                    {
                        "command": "speak",
                        "message_id": "privacy-fixture",
                        "text": "An invented privacy playback passage. " * 40,
                    }
                )
                if not reply.get("ok"):
                    raise MacFailure(f"playback refused: {json.dumps(reply)}")
                before = self.audio_state({"playing": True}, 60)
                self.native_key("privacy")
                after = self.audio_state(
                    {
                        "privacy_stopped": True,
                        "microphone": False,
                        "system_audio": False,
                        "playing": False,
                    }
                )
            finally:
                # A failed key must never make the pending fixture audible when
                # the output is restored. Stops are idempotent, input is not.
                try:
                    reply = self.orbit.call({"command": "speak_stop"})
                    if not reply.get("ok"):
                        raise MacFailure("fixture playback stop was refused")
                    self.audio_state({"playing": False})
                except (MacFailure, OSError, ValueError):
                    self._close_ours(32)
        return {"before": before, "after": after}

    def native_privacy(self):
        self.activate()
        with self.system.speaking(
            self.paths,
            "The invented privacy meeting continues while the brake is pressed.",
        ):
            before = self.audio_state({"microphone": True, "privacy_stopped": False})
            self.native_key("privacy")
            stopped = self.audio_state(
                {
                    "privacy_stopped": True,
                    "microphone": False,
                    "system_audio": False,
                    "playing": False,
                }
            )
        return {"before": before, "stopped": stopped}

    def native_quit(self):
        self.activate()
        pids = self._ours()
        helper = str(self.paths.app / "Contents/MacOS/OrbitNative")
        helpers = [pid for pid, path in self.system.processes() if path == helper]
        self.native_key("quit")
        left = self.system.wait_exit(pids, 32)
        self.system.pause(OBSERVE_S)
        return {
            "processes": len(pids),
            "helpers": len(helpers),
            "left": left,
            "came_back": len(self._ours()),
        }

    def native_lifecycle(self, ident):
        self.activate()
        with self.system.speaking(
            self.paths,
            "The invented lifecycle meeting keeps speaking through window close and application quit.",
        ) as speech:
            before = self.wait("recording-progress", json.dumps({"id": ident}), STATE_S)["value"]
            self.native_key("close")
            hidden = self.window_visibility(False)
            after = self.wait("recording-progress", json.dumps(before), STATE_S)["value"]
            self.native_key("dock")
            visible = self.window_visibility(True)
            speaking = speech.poll() is None
            quit_at = self.system.wall_ms()
            ended = self.native_quit()
        return {
            "before": before,
            "after": after,
            "hidden": hidden,
            "visible": visible,
            "speech_at_quit": speaking,
            "quit_at_ms": quit_at,
            "exit": ended,
        }

    def window_visibility(self, expected):
        start = self.system.now()
        while True:
            windows = self.bridge.call({"command": "windows"}).get("windows", [])
            main = next((w for w in windows if w.get("label") == "main"), {})
            if main.get("visible") is expected:
                return expected
            if self.system.now() - start >= STATE_S:
                raise MacFailure(f"main window visibility did not become {expected}")
            self.system.pause(0.1)

    def native_edit(self, typing):
        self.activate()
        typing["text"] = ""
        self.bridge.call(typing)  # Focus only; every edit below is a real key.
        values = {}
        self.native_key("text", "Maple budget")
        values["typed"] = self.field_value(typing, "Maple budget")
        self.native_key("undo")
        values["undo"] = self.field_value(typing, "")
        self.native_key("redo")
        values["redo"] = self.field_value(typing, "Maple budget")
        self.native_key("select-all")
        self.native_key("copy")
        self.native_key("end")
        self.native_key("paste")
        values["paste"] = self.field_value(typing, "Maple budgetMaple budget")
        # Clear only the test's clipboard content, never save/read the owner's.
        self.system.clear_clipboard()
        return values

    def field_value(self, typing, expected):
        start = self.system.now()
        while True:
            fields = self.bridge.call({"command": "read", "window": typing["window"]}).get(
                "elements", []
            )
            values = [
                r.get("text")
                for r in fields
                if r.get("role") == "textbox" and r.get("name") == typing["name"]
            ]
            if values == [expected]:
                return expected
            if self.system.now() - start >= STATE_S:
                raise MacFailure(f"native editor expected {expected!r}; observed {values!r}")
            self.system.pause(0.1)
