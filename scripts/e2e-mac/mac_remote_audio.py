"""Silent, app-owned remote speech for check 25. Mac-side Python 3.9."""

from __future__ import annotations

import json
import signal

from mac_controls import MacFailure

PLAYER = "Orbit Pruebas Remote"
PLAYER_ID = "app.orbit.pruebas.remote"
SINKS = {"BlackHole 2ch": "BlackHole2ch_UID", "BlackHole 16ch": "BlackHole16ch_UID"}
# Provisional budgets for local compilation/launch and one short phrase. These
# bound missing receipts, not audio duration; replace with Mac measurements.
READY_S = 15
PLAY_S = 30


def phrases(language):
    if language == "en":
        return {
            "mic": "Cedar shelves cost seven hundred",
            "remote": "Maple windows open on Thursday",
            "mic-muted": "Cedar umbrellas are purple",
            "remote-muted": "Maple gardens need watering",
        }
    # «Roble», not «Arce»: the recognizer once heard «ar se», and a split
    # content word fails the check like a missing one.
    return {
        "mic": "Los estantes de Cedro cuestan setecientos",
        "remote": "Las ventanas de Roble abren el jueves",
        "mic-muted": "Los paraguas de Cedro son morados",
        "remote-muted": "Los jardines de Roble necesitan riego",
    }


def require_silent(route):
    for name, uid in SINKS.items():
        rows = [d for d in route.get("devices", []) if d.get("name") == name]
        if not rows:
            raise MacFailure(f"missing {name} input/output; check 25 installs no driver")
        if len(rows) != 1:
            raise MacFailure(f"{name} resolves to {len(rows)} devices; expected exactly 1")
        if rows[0].get("uid") != uid:
            raise MacFailure(f"{name} has an unexpected device UID")
        if rows[0].get("virtual") is not True:
            raise MacFailure(f"{name} is not a direct virtual device")
        if rows[0].get("alive") is not True:
            raise MacFailure(f"{name} is not alive")
    if route.get("unsafe_readers") != 0:
        raise MacFailure("audio monitoring is active or unknown: another process holds an input")
    if route.get("unsafe_aggregates") != 0:
        raise MacFailure("an aggregate/multi-output route contains a fixture sink, or is unknown")


class RemoteAudio:
    @property
    def remote_directory(self):
        return self.paths.own / "remote-audio"

    @property
    def remote_executable(self):
        return self.paths.remote_app / "Contents" / "MacOS" / "RemoteSpeaker"

    def _remote_readers(self):
        from mac import MEMBERS

        allowed = {str(self.paths.app / member) for member in MEMBERS[:2]}
        return ",".join(str(pid) for pid, command in self.system.processes() if command in allowed)

    def _remote_probe(self):
        # Only the isolated app/helper may read audio during this trial. No
        # process names, device listings or unrelated pids enter the evidence.
        route = json.loads(
            self.system.run_checked(str(self.remote_executable), "--probe", self._remote_readers())
        )
        require_silent(route)
        return route

    def _remote_answer(self, name, max_s):
        path = self.remote_directory / f"{name}.json"
        failure = self.remote_directory / "failure.json"
        start = self.system.now()
        while not path.exists():
            if failure.exists():
                cause = json.loads(failure.read_text(encoding="utf-8")).get("error")
                raise MacFailure(f"remote speaker stopped: {cause}")
            if self.system.now() - start >= max_s:
                raise MacFailure(f"remote speaker has no {name} receipt within {max_s} s")
            self.system.pause(0.1)
        answer = json.loads(path.read_text(encoding="utf-8"))
        if answer.get("error"):
            raise MacFailure(f"remote speaker: {answer['error']}")
        return answer

    def remote_audio(self, action):
        from harness_copy import language
        from mac import required_voices

        if action == "stop":
            # Exact executable identity, including recovery after an interrupted
            # SSH call. Never trust a stale pid file and never signal by pattern.
            pids = [p for p, c in self.system.processes() if c == str(self.remote_executable)]
            if not pids:
                return {"restored": True}
            for pid in pids:
                self.system.signal(pid, signal.SIGTERM)
            left = self.system.wait_exit(pids, READY_S)
            for pid in left:
                self.system.signal(pid, signal.SIGKILL)
            if self.system.wait_exit(left, READY_S):
                raise MacFailure("remote speaker did not stop; audio restoration incomplete")
            return {"restored": True}
        if action == "preflight":
            self._remote_build()
            return self._remote_probe()
        if action == "prepare":
            return self._remote_prepare()
        if action not in phrases(language()):
            raise MacFailure(f"unknown remote audio phase: {action}")
        route = self._remote_probe()
        current = self.system.run_checked("SwitchAudioSource", "-c", "-t", "input").strip()
        if current != "BlackHole 2ch":
            raise MacFailure("check 25 microphone is not BlackHole 2ch; no speech started")
        if action.startswith("mic"):
            voice = required_voices(self.system.run_checked("say", "-v", "?"))[0]
            self.system.run_checked(
                "say", "-v", voice, "-a", "BlackHole 2ch", phrases(language())[action]
            )
        else:
            (self.remote_directory / f"{action}-done.json").unlink(missing_ok=True)
            request = self.remote_directory / "request.tmp"
            request.write_text(json.dumps({"phase": action}), encoding="utf-8")
            request.replace(self.remote_directory / "request.json")
            answer = self._remote_answer(action + "-done", PLAY_S)
            if (
                answer.get("played") is not True
                or answer.get("output_uid") != SINKS["BlackHole 16ch"]
            ):
                raise MacFailure("remote playback receipt did not confirm the silent output")
        return {"route": route, "phase": action}

    def _remote_build(self):
        # Cheap, named failure before compiling a player or synthesizing speech.
        for direction in ("input", "output"):
            listing = self.system.run_checked("SwitchAudioSource", "-a", "-t", direction)
            available = {name.strip() for name in listing.splitlines()}
            for name in SINKS:
                if name not in available:
                    raise MacFailure(f"missing {name} {direction}; check 25 installs no driver")
        directory = self.remote_directory
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.remote_audio("stop")
        # Built and signed with Orbit Pruebas's identity by build-fixture-apps.sh
        # (with every build-test-app.sh): nothing is compiled or signed here.
        if not self.remote_executable.exists():
            raise MacFailure(
                f"{self.paths.remote_app} is missing: build Orbit Pruebas with "
                "scripts/e2e-mac/build-test-app.sh, which builds it too"
            )

    def _remote_prepare(self):
        from harness_copy import language
        from mac import required_voices

        self._remote_build()
        directory = self.remote_directory
        route = self._remote_probe()  # Before synthesis, opening the app or speaking.
        voice = required_voices(self.system.run_checked("say", "-v", "?"))[-1]
        for phase, phrase in phrases(language()).items():
            if phase.startswith("remote"):
                self.system.run_checked(
                    "say", "-v", voice, "-o", str(directory / f"{phase}.aiff"), phrase
                )
        for name in ("ready", "remote-done", "remote-muted-done", "request", "failure"):
            (directory / f"{name}.json").unlink(missing_ok=True)
        self.system.run_checked(
            "open",
            "-n",
            "-g",
            "-a",
            str(self.paths.remote_app),
            "--args",
            str(directory),
            self._remote_readers(),
            str(PLAY_S),
        )
        return {**self._remote_answer("ready", READY_S), "route": route}
