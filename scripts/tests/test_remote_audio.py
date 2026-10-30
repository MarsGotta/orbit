"""Check 25 decisions use synthetic routes and transcripts, never host audio."""

import copy
import json
import sys
import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "e2e-mac"))

import all_checks


class RemoteAudioTests(unittest.TestCase):
    def routes(self):
        return {
            "devices": [
                {
                    "name": "BlackHole 2ch",
                    "uid": "BlackHole2ch_UID",
                    "virtual": True,
                    "alive": True,
                },
                {
                    "name": "BlackHole 16ch",
                    "uid": "BlackHole16ch_UID",
                    "virtual": True,
                    "alive": True,
                },
            ],
            "unsafe_readers": 0,
            "unsafe_aggregates": 0,
        }

    def test_only_direct_independent_virtual_sinks_are_safe(self):
        import mac_remote_audio

        mac_remote_audio.require_silent(self.routes())
        for field in ("virtual", "alive", "uid"):
            with self.subTest(field=field):
                routes = self.routes()
                routes["devices"][1][field] = False
                with self.assertRaisesRegex(mac_remote_audio.MacFailure, "BlackHole 16ch"):
                    mac_remote_audio.require_silent(routes)
        for field in ("unsafe_readers", "unsafe_aggregates"):
            routes = self.routes()
            routes[field] = 1
            with self.assertRaisesRegex(mac_remote_audio.MacFailure, "monitor|aggregate"):
                mac_remote_audio.require_silent(routes)
        routes = self.routes()
        routes["devices"].pop()
        with self.assertRaisesRegex(mac_remote_audio.MacFailure, "missing.*BlackHole 16ch"):
            mac_remote_audio.require_silent(routes)
        with self.assertRaises(mac_remote_audio.MacFailure):
            mac_remote_audio.require_silent({"devices": routes["devices"]})

    def test_check_requires_each_phrase_on_only_its_own_stored_side(self):
        import remote_audio

        phrases = remote_audio.phrases("en")
        evidence = {
            "recording": {
                "id": "cedar",
                "audio": {"application": {"application_pid": 4242}},
            },
            "player": {"pid": 4242},
            "route": self.routes(),
            "muted": {"microphone_muted": True},
            "unmuted": {"microphone_muted": False},
            "muted_ui": True,
            "restored": True,
            "stored": {
                "closed": True,
                "complete": True,
                "tracks": {
                    "mic_local": {
                        "frames": 100,
                        "chunks": 1,
                        "sha256": "a" * 64,
                        "nonzero": True,
                    },
                    "remote_app": {
                        "frames": 100,
                        "chunks": 1,
                        "sha256": "b" * 64,
                        "nonzero": True,
                    },
                },
                "segments": [
                    {"track": "mic_local", "text": phrases["mic"]},
                    {
                        "track": "remote_app",
                        "text": phrases["remote"] + ". " + phrases["remote-muted"],
                    },
                ],
            },
        }
        ev = {"language": "en", "remote_audio": evidence}
        self.assertIn(25, all_checks.CHECKS)
        check = all_checks.CHECKS[25]
        self.assertTrue(check(ev)[0])
        for mutation in (
            "swap",
            "leak",
            "lost-remote",
            "duplicate",
            "silence",
            "unrestored",
        ):
            with self.subTest(mutation=mutation):
                broken = copy.deepcopy(ev)
                row = broken["remote_audio"]
                segments = row["stored"]["segments"]
                if mutation == "swap":
                    segments[0]["track"], segments[1]["track"] = (
                        "remote_app",
                        "mic_local",
                    )
                elif mutation == "leak":
                    segments[1]["text"] += " " + phrases["mic-muted"]
                elif mutation == "lost-remote":
                    segments[1]["text"] = phrases["remote"]
                elif mutation == "duplicate":
                    row["stored"]["tracks"]["remote_app"]["sha256"] = "a" * 64
                elif mutation == "silence":
                    row["stored"]["tracks"]["remote_app"]["nonzero"] = False
                else:
                    row["restored"] = False
                self.assertFalse(check(broken)[0])

    def test_missing_sink_stops_before_any_synthesis_or_app_launch(self):
        import mac

        route = self.routes()
        route["devices"].pop()

        class System(mac.System):
            def __init__(self):
                self.commands = []

            def run(self, *command):
                self.commands.append(command)
                if command[0] == "SwitchAudioSource":
                    return 0, "\n".join(d["name"] for d in route["devices"]), ""
                if "--probe" in command:
                    return 0, json.dumps(route), ""
                return 0, "", ""

            def processes(self):
                return []

            def wait_exit(self, pids, max_s):
                return []

        with tempfile.TemporaryDirectory() as directory:
            system = System()
            device = mac.Mac(mac.Paths(Path(directory)), system)
            with self.assertRaisesRegex(mac.MacFailure, "missing BlackHole 16ch"):
                device.remote_audio("prepare")
            self.assertFalse(any(c[0] in ("say", "open") or "-s" in c for c in system.commands))

    def test_cleanup_signals_only_the_exact_fixture_executable(self):
        import mac

        with tempfile.TemporaryDirectory() as directory:
            device = mac.Mac(mac.Paths(Path(directory)))
            executable = str(device.remote_executable)

            class System(mac.System):
                def __init__(self):
                    self.signalled = []

                def processes(self):
                    return [
                        (1001, executable),
                        (1002, executable + "-other"),
                        (1003, "/owner/RemoteSpeaker"),
                    ]

                def signal(self, pid, number):
                    self.signalled.append(pid)

                def wait_exit(self, pids, max_s):
                    return []

            system = device.system = System()
            self.assertEqual(device.remote_audio("stop"), {"restored": True})
            self.assertEqual(system.signalled, [1001])

    def test_failed_remote_speech_stops_the_player_and_finishes_the_fixture(self):
        import remote_audio
        from interface import StepFailed

        calls = []

        class Mac:
            def remote_audio(self, phase):
                calls.append(phase)
                if phase == "prepare":
                    return {"pid": 4242, "route": {}}
                if phase == "mic":
                    raise StepFailed("fixture speech refused")
                return {"restored": True}

        class UI:
            def press(self, window, name, roles, **kwargs):
                calls.append(name)

            def read_until(self, *args):
                return 0

        class Test:
            def __init__(self):
                self.ev = {"remote_preflight": {}}
                self.mac, self.ui, self.main = Mac(), UI(), "main"
                self.bounds = {"screen": (0, "fixture")}

            @contextmanager
            def step(self, *args, **kwargs):
                yield

            def wait_mac(self, what, arg=None):
                if what == "recording":
                    return {
                        "id": "cedar",
                        "audio": {"application": {"application_pid": 4242}},
                    }
                return {}

            def measure(self, *args):
                pass

        test = Test()
        with self.assertRaisesRegex(StepFailed, "fixture speech refused"):
            remote_audio.exercise(test)
        self.assertIn("stop", calls)
        self.assertIn(remote_audio.copy_text("meeting.finish"), calls)
        self.assertTrue(test.ev["remote_audio"]["restored"])

    def test_failed_player_stop_still_restores_the_owners_input(self):
        import mac

        called = []

        class Device(mac.Mac):
            def remote_audio(self, action):
                raise mac.MacFailure("remote speaker did not stop")

            def engine_stop(self, max_s):
                called.append("engine")
                return {}

            def _close_ours(self, max_s):
                called.append("app")
                return {}

            def _restore_input(self):
                called.append("input")
                return "Fixture microphone"

        device = Device(mac.Paths(Path("/synthetic-unused")))
        with self.assertRaisesRegex(mac.MacFailure, "remote speaker did not stop"):
            device.restore(1)
        self.assertEqual(called, ["engine", "app", "input"])


if __name__ == "__main__":
    unittest.main()
