"""The real test on the Mac: the orchestrator, launched from the test server.

    server/.venv/bin/python scripts/e2e-mac/real_test.py prepare
    server/.venv/bin/python scripts/e2e-mac/real_test.py run
    server/.venv/bin/python scripts/e2e-mac/real_test.py dry-run [--example NAME]

`prepare` creates, once, the test's Hermes token under
~/.orbit-pruebas/secretos. `run` is the real run: it starts the test profile
of Hermes and the test hub here, prepares the Mac over SSH (ORBIT_MAC_HOST,
ORBIT_MAC_SSH_KEY, as the release scripts), records a synthetic meeting in
Orbit Pruebas clicking and reading its interface through its in-app bridge,
with CuaDriver scoped to check 21's two read-only accessibility samples, with three lines of it muted
(«Silenciar»), sends Deimos a voice note and opens a call with it over Orbit
Pruebas's socket, throws a second recording away («Terminar sin guardar»),
deletes the first meeting («Borrar reunión»), quits Orbit Pruebas right
after typing a draft (the Dock's quit) and opens it again, runs the
checks and cleans up, also when something fails. During the meeting it also
moves the system's pointer over the floating panel that shows Deimos's
answer, and away from it. `dry-run` walks the same orchestrator
and the same checks on Linux, against the example states in
scripts/e2e-mac/dry-run-examples/, with the real test hub on loopback and
nothing else real: it proves the logic of the checks, never Orbit, and it is
not the real test.

Runbook: CONTRIBUTING.md, on the real test on the Mac.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid
from contextlib import contextmanager
from datetime import UTC, datetime
from pathlib import Path

DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORY))

import accessibility
import all_checks
import audio_route
import checks
import drafts_panel
import first_run
import hub_traces
import isolated_hub
import mac as mac_side
import mac_controls
import meeting_controls
import silence_recording
from interface import InAppBridge, Interface, StepFailed

ROOT = DIRECTORY.parents[1]
BASE = Path.home() / ".orbit-pruebas"
DEVICE_NAME = "Orbit Pruebas"
REMOTE = ".orbit-pruebas/harness"
# Every ssh of the test leaves with these, as the release scripts' do
# (scripts/release/lib.sh): no password prompt nobody can answer, a sleeping
# Mac found out in seconds, only the given key.
SSH = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "IdentitiesOnly=yes"]
# What an SSH call adds to the wait it carries: connecting (ConnectTimeout
# above) and starting python on the Mac.
SSH_MARGIN_S = 30


def _rust_constant(path: str, name: str) -> int:
    """A number read from the Rust source, so the bound follows the code."""
    text = (ROOT / path).read_text(encoding="utf-8")
    found = re.search(rf"const {name}: [^=]+= (?:Duration::from_secs\()?([0-9_]+)", text)
    if found is None:
        raise RuntimeError(f"{path} no longer declares {name}")
    return int(found.group(1).replace("_", ""))


def bounds() -> dict[str, tuple[float, str]]:
    """Every wait of the run: its bound and why it is that size.

    Each wait ends on the fact it waits for, or earlier on the fact that says
    it will not come; the bound only covers a fact that never arrives. Bounds
    marked «provisional» have no measurement behind them yet: the report of
    every run writes down how long each wait took, and the reviewer replaces
    them with what the measurements say (p99 and worst seen).
    """
    from orbit_hub import grouping, meeting_transcription
    from orbit_hub.processing.config import ProcessingConfiguration

    ready_s = _rust_constant("apps/desktop/src-tauri/src/daemon_thread.rs", "DAEMON_READY_WAIT")
    exit_s = _rust_constant("apps/desktop/src-tauri/src/daemon_thread.rs", "DAEMON_EXIT_WAIT")
    voice_s = _rust_constant("crates/orbit-daemon/src/hub.rs", "VOICE_HUB_HTTP_TIMEOUT")
    turn_s = 180  # Orbit does not bound a Hermes turn: the test's patience
    # The script lasts about two minutes, and the panel is held over a minute
    # in the middle of it (check 18); a third more for pauses.
    duration_s = 240
    groups = -(-duration_s // grouping.TARGET_SECONDS)
    # The hub asks for the note itself and the Mac reads it again when the
    # hub announces the change (`recording.changed`): no follow interval of
    # the Mac's adds to the wait.
    note_s = (
        groups * meeting_transcription.REQUEST_TIMEOUT_SECONDS
        + ProcessingConfiguration.model_fields["timeout_seconds"].default
    )
    return {
        "hermes": (60, "provisional: the start of `hermes serve` is not measured"),
        "hub": (35, "twenty times the worst hub start measured (1.73 s)"),
        "daemon": (
            ready_s + 15,
            (
                f"the app waits {ready_s} s for its daemon (DAEMON_READY_WAIT) plus what "
                "macOS takes to open it (provisional)"
            ),
        ),
        "paired": (30, "provisional: the daemon connecting to the test hub, not measured"),
        "cua-driver": (30, "provisional: the start of CuaDriver's daemon, not measured"),
        "accessibility-tree": (
            10,
            "provisional: WebKit builds its tree when macOS first asks, not measured",
        ),
        "call": (
            120,
            "cua-driver call stops waiting for its daemon at 120 s (daemon.rs, send_request)",
        ),
        "recording": (30, "provisional: the start of the microphone capture, not measured"),
        "listening": (300, "provisional: the first time, macOS downloads the Spanish speech model"),
        "answer": (
            180,
            "provisional: Orbit does not bound a Hermes turn; this is the test's patience",
        ),
        "saved": (
            120,
            (
                "provisional: sealing the recording and the Notes list taking it as closed, "
                "not measured"
            ),
        ),
        "note": (
            note_s,
            (
                f"what the system itself takes to give up: {groups} stretches of "
                f"{grouping.TARGET_SECONDS} s at {meeting_transcription.REQUEST_TIMEOUT_SECONDS} s "
                f"each and the note at "
                f"{ProcessingConfiguration.model_fields['timeout_seconds'].default:.0f} s"
            ),
        ),
        "screen": (10, "the interface receives the pushed changes; covers two engine reads"),
        "first-view": (
            20,
            (
                "provisional: the webview drawing its first view once the window is listed, "
                "not measured; the window itself appears in about a second"
            ),
        ),
        "chat": (
            turn_s,
            "provisional: Orbit does not bound a Hermes turn; this is the test's patience",
        ),
        "voice-note": (
            voice_s + turn_s,
            (
                f"the daemon gives a voice note's transcription {voice_s} s "
                f"(VOICE_HUB_HTTP_TIMEOUT) and the test gives the Hermes turn {turn_s} s "
                "(provisional, as «chat»); a failed transcription ends it earlier"
            ),
        ),
        "call-listening": (
            30,
            (
                "provisional: opening the microphone with voice processing, not measured; "
                "Orbit does not bound that opening"
            ),
        ),
        "session": (30, "provisional: opening a conversation stored in the hub, not measured"),
        "close": (
            exit_s + 15,
            f"the app gives its daemon {exit_s} s to exit (DAEMON_EXIT_WAIT), plus a margin",
        ),
        "stop": (
            30,
            "the same bound the production hub's unit gives to stop (TimeoutStopSec=30)",
        ),
        **meeting_controls.BOUNDS,
        **audio_route.BOUNDS,
        **drafts_panel.bounds(_rust_constant),
        **silence_recording.bounds(_rust_constant),
    }


# Queries for the bridge's semantic tree: matching rows and their ancestors.
# Unfiltered readings preserve hierarchy for transcript, list and draft checks.
READS: dict[str, str | None] = {
    # «Transcribiendo…», «Escribiendo la nota» and «Escribiendo el resumen…»
    # all contain it.
    "note-ready": "ribiendo",
    # The opened note's title is the field «Título de la reunión» in its
    # first-level heading (editable, it renames the meeting); the list beside
    # it names the note too, as a row, so a row is never taken for it.
    "note-from-notice": "Título de la reunión",
    "note-from-list": "Título de la reunión",
    "note-summary": "AXStaticText",
    "note-actions": "AXCheckBox",
    # Read whole: every turn's speaker is a button (it opens the speaker menu,
    # TranscriptSegment.tsx), not a static text, and no one query holds both
    # the times and texts and the speakers. A two-minute meeting is a few
    # dozen turns. The bridge refuses oversized replies with their byte counts.
    "note-transcript": None,
    "assistant-answered": "AXStaticText",
    "assistant-reopened": "AXStaticText",
    "final": "AXStaticText",
    # The call panel is a card of a handful of rows: read whole too.
    "panel": None,
    **meeting_controls.READS,
}


class RunStopped(Exception):
    """A step the rest of the run needed failed; the run goes to clean up."""


# ── The Mac, over SSH ───────────────────────────────────────────────────────


class RemoteMac:
    """mac.py on the Mac, one subcommand per SSH call."""

    def __init__(self, host: str, key: str):
        if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*(@[A-Za-z0-9_][A-Za-z0-9_.-]*)?", host):
            raise ValueError(f"invalid ORBIT_MAC_HOST: «{host}»")
        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", key) or not Path(key).is_file():
            raise ValueError(f"ORBIT_MAC_SSH_KEY missing or invalid: «{key}»")
        self.host = host
        self.options = [*SSH, "-i", key]

    def copy(self) -> None:
        subprocess.run(
            [
                "ssh",
                *self.options,
                self.host,
                f"mkdir -p -m 700 .orbit-pruebas && mkdir -p -m 700 {REMOTE}",
            ],
            check=True,
            capture_output=True,
            timeout=SSH_MARGIN_S,
        )
        subprocess.run(
            [
                "scp",
                "-q",
                *self.options,
                str(DIRECTORY / "mac.py"),
                str(DIRECTORY / "mac_controls.py"),
                str(DIRECTORY / "mac_bridge.py"),
                str(DIRECTORY / "mac_drafts_panel.py"),
                str(DIRECTORY / "mac_engine.py"),
                str(DIRECTORY / "mac_system.py"),
                str(DIRECTORY / "mac_waits.py"),
                str(DIRECTORY / "mac_silence.py"),
                str(DIRECTORY / "audio_route.py"),
                str(DIRECTORY / "meeting-script.txt"),
                str(DIRECTORY / "windows.swift"),
                str(DIRECTORY / "pointer.swift"),
                f"{self.host}:{REMOTE}/",
            ],
            check=True,
            capture_output=True,
            timeout=SSH_MARGIN_S,
        )

    def _call(self, bound_s: float, *arguments: str) -> dict:
        # A non-interactive SSH session has no /opt/homebrew/bin, where
        # SwitchAudioSource lives (scripts/release/mac-build.sh says the same).
        remote = (
            "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin "
            f"python3 {REMOTE}/mac.py " + " ".join(shlex.quote(a) for a in arguments)
        )
        try:
            done = subprocess.run(
                ["ssh", *self.options, self.host, remote],
                capture_output=True,
                text=True,
                timeout=bound_s + SSH_MARGIN_S,
                check=False,
            )
        except subprocess.TimeoutExpired:
            raise StepFailed(
                f"the Mac did not answer «{arguments[0]}» within {bound_s + SSH_MARGIN_S:.0f} s"
            ) from None
        line = done.stdout.strip().splitlines()[-1] if done.stdout.strip() else ""
        try:
            output = json.loads(line)
        except ValueError:
            raise StepFailed(
                f"«{arguments[0]}» on the Mac exited with {done.returncode}: "
                f"{done.stderr.strip()[:300]}"
            ) from None
        if done.returncode != 0 or "error" in output:
            raise StepFailed(f"«{arguments[0]}» on the Mac: {output.get('error')}")
        return output

    def prepare(self, max_s):
        return self._call(max_s, "prepare", "--max", str(max_s))

    def pair(self, url, code, name):
        return self._call(30, "pair", url, code, name)

    def wait(self, what, arg, max_s):
        return self._call(max_s, "wait", what, *([arg] if arg else []), "--max", str(max_s))

    def read(self, what, ident=None):
        return self._call(30, "read", what, *([ident] if ident else []))

    def switch_input(self, name=None):
        return self._call(30, "switch-input", *([name] if name else []))

    def say(self, part):
        # Each part is under a minute of speech at say's default rate.
        return self._call(240, "say", part)

    def restore(self, max_s):
        return self._call(max_s, "restore", "--max", str(max_s))

    def logs(self):
        return self._call(60, "logs")

    def activate(self):
        return self._call(30, "activate")

    def floating(self):
        # `swift` compiles the script first: a few seconds, far under 60.
        return self._call(60, "floating")

    def launch(self, max_s):
        return self._call(max_s, "launch", "--max", str(max_s))

    def draft_and_quit(self, how, typing, screen_s, close_s):
        # Typing, reading it back, the quit, the processes' end and the 5 s
        # relaunch watch (mac_drafts_panel.RELAUNCH_WATCH_S), all on the Mac.
        bound = 120 + screen_s + close_s + mac_side.RELAUNCH_WATCH_S
        return self._call(
            bound, "draft-and-quit", how, typing, str(screen_s), "--max", str(close_s)
        )

    def panel_pointer(self, window_id, where, question, max_s):
        # `swift` compiles pointer.swift first (as `floating`), then the wait.
        return self._call(
            60 + max_s, "panel-pointer", str(window_id), where, question, "--max", str(max_s)
        )

    def bridge_call(self, request):
        return self._call(12, "bridge-call", json.dumps(request, ensure_ascii=False))

    def bridge_menus(self):
        return self._call(12, "bridge-menus")

    def engine_start(self, max_s):
        return self._call(max_s, "engine-start", "--max", str(max_s))

    def engine_call(self, tool, arguments):
        # cua-driver call gives up on its daemon at 120 s (bounds «call»).
        return self._call(120, "engine-call", tool, json.dumps(arguments, ensure_ascii=False))

    def engine_stop(self, max_s):
        return self._call(max_s, "engine-stop", "--max", str(max_s))

    def voice_note(self):
        # One short phrase at say's default rate and two beats: seconds.
        return self._call(60, "voice-note")

    def command(self, name):
        return self._call(30, "command", name)


class InProcessMac:
    """mac.py's own steps, called in this process (the dry run)."""

    def __init__(self, mac: mac_side.Mac):
        self.mac = mac

    def copy(self) -> None:
        return None

    def __getattr__(self, name):
        method = getattr(self.mac, name)

        def call(*arguments):
            try:
                return method(*arguments)
            except mac_side.MacFailure as cause:
                raise StepFailed(str(cause)) from None

        return call


# ── The run ─────────────────────────────────────────────────────────────────


class RealTest:
    def __init__(
        self,
        mac,
        paths: isolated_hub.Paths,
        host: str,
        port: int,
        directory: Path,
        hermes: bool,
        providers: dict | None = None,
    ):
        self.mac = mac
        self.paths = paths
        self.host = host
        self.port = port
        self.directory = directory
        self.hermes = hermes
        self.providers = providers
        self.bounds = bounds()
        self.script = mac_side.script_parts(mac_side.SCRIPT.read_text(encoding="utf-8"))
        span = mac_controls.muted_span(self.script)
        self.ev: dict = {
            "steps": [],
            "waits": [],
            "trees": {},
            # What the recording hears: every line but those said with
            # «Silenciar» on, which check 13 looks for apart.
            "script": [
                said for part in self.script for _, said in part if said not in span["muted"]
            ],
            "muted_script": span,
            # Where a first run's welcome was skipped (first_run.py).
            "welcome_skipped": [],
        }
        database = hub_traces.database(paths.hub)
        # What the test hub keeps of a recording (the dry run replays it).
        self.hub_kept = lambda recording: hub_traces.kept(database, recording)
        self.processes: list[isolated_hub.Process] = []
        # The codes that are failures though they are silent on screen,
        # read from the one place that decides it (NoticeCode::is_failure).
        self.ev["failure_codes"] = sorted(
            checks.failure_codes(
                (ROOT / "crates" / "orbit-core" / "src" / "notice" / "mod.rs").read_text(
                    encoding="utf-8"
                )
            )
        )
        self.ui: Interface | None = None
        self.main = None
        self.cua_active = False

    @contextmanager
    def step(self, name: str, checks_: tuple[int, ...] = (), critical: bool = True):
        row = {
            "name": name,
            "start": time.time(),
            "checks": list(checks_),
            "ok": True,
        }
        self.ev["steps"].append(row)
        try:
            yield
        except (
            StepFailed,
            checks.Unchecked,
            RuntimeError,
            OSError,
            ValueError,
            KeyError,
            subprocess.SubprocessError,
        ) as cause:
            row["ok"] = False
            row["cause"] = str(cause) or type(cause).__name__
            if critical:
                raise RunStopped(name) from cause
        finally:
            row["end"] = time.time()

    def wait_mac(self, what: str, arg=None) -> dict:
        max_s, _ = self.bounds[what]
        row = {"wait": what, "max_s": max_s}
        self.ev["waits"].append(row)
        try:
            done = self.mac.wait(what, arg, max_s)
        except StepFailed as cause:
            row["failure"] = str(cause)
            raise
        row["seconds"] = done.get("seconds")
        return done["value"]

    def measure(self, what: str, seconds: float) -> None:
        self.ev["waits"].append(
            {"wait": what, "max_s": self.bounds[what][0], "seconds": round(seconds, 3)}
        )

    # ── steps ──

    def services(self) -> None:
        with self.step("stop what an earlier run left"):
            self.ev["leftovers"] = isolated_hub.stop_leftovers(self.paths, self.bounds["stop"][0])
        # The dry run has no Hermes: nothing there asks it anything.
        if self.hermes:
            with self.step("test Hermes", (7, 8), critical=False):
                # Its log stays out of the run's directory: it may carry what
                # the conversations said.
                process = isolated_hub.start_hermes(
                    self.paths, isolated_hub.HERMES_PORT, self.paths.base / "hermes.log"
                )
                self.processes.append(process)
                bound = self.bounds["hermes"][0]
                self.measure(
                    "hermes",
                    isolated_hub.wait_port("127.0.0.1", isolated_hub.HERMES_PORT, process, bound),
                )
        with self.step("test hub"):
            self.paths.config.write_text(
                isolated_hub.config_text(
                    self.paths, self.host, isolated_hub.HERMES_PORT, self.providers
                ),
                encoding="utf-8",
            )
            isolated_hub.init_store(self.paths)
            process = isolated_hub.start_hub(
                self.paths, self.host, self.port, self.directory / "hub.log"
            )
            self.processes.append(process)
            self.measure(
                "hub", isolated_hub.wait_health(self.hub_url, process, self.bounds["hub"][0])
            )

    @property
    def hub_url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def mac_ready(self) -> None:
        with self.step("prepare the Mac"):
            self.mac.copy()
            self.ev["mac"] = self.mac.prepare(self.bounds["daemon"][0])
        with self.step("pair Orbit Pruebas with the test hub"):
            code = isolated_hub.pairing_code(self.paths)
            self.mac.pair(self.hub_url, code, DEVICE_NAME)
            self.wait_mac("paired")

    def drive_interface(self) -> None:
        self.ui = Interface(InAppBridge(self.mac), self.directory / "trees")
        ui = self.ui
        with self.step("Orbit Pruebas's window"):
            self.main = self.open_main("home")

        with accessibility.session(self) as capture:
            capture("idle")
            with self.step("start recording from the interface", (1, 2, 3, 4, 5, 6, 7)):
                # «Grabar reunión» of the sidebar, in every view (Sidebar.tsx); the
                # sidebar comes first in the tree, before any view's own button.
                ui.press(self.main, "Grabar reunión", ("AXButton",), first=True)
                # Microphone only: the script plays into BlackHole, the default
                # input, and with system audio too every line would be recorded
                # twice.
                # «Qué audio grabar» is a list of radios, each labelled with its
                # explanation (AudioSourceField.tsx): pressed by its name.
                ui.press(self.main, "Solo mi micrófono", ("AXRadioButton",), contains=True)
                ui.press(self.main, "Empezar a grabar", ("AXButton",))
                recording = self.wait_mac("recording")
                self.ev["recording"] = dict(recording)
                if recording.get("audio") != "microphone_only":
                    raise StepFailed(
                        f"the recording started with audio «{recording.get('audio')}», not "
                        "«microphone_only»: «Qué audio grabar» did not change to «Solo mi micrófono»"
                    )

            capture("recording")

        with self.step(
            "read the recording's menu bar through the test bridge", (16,), critical=False
        ):
            self.ev["menus"] = self.mac.bridge_menus()

        with self.step("first half of the script"):
            self.mac.say("part1")

        panel = None
        with self.step("«Oye Deimos» and its answer in the panel", (7,), critical=False):
            try:
                self.wait_mac("listening")
            finally:
                # Said even if nobody listens: the meeting is the same meeting.
                self.mac.say("question")
            self.ev["question"] = self.wait_mac("answer")
            answered_at = time.monotonic()
            if self.ev["question"].get("stage") == "answered":
                # The panel hides 45 s after the answer (ANSWER_SHOWN_MS).
                panel = ui.panel(self.bounds["screen"][0])
                self.measure(
                    "screen",
                    ui.read_until(
                        panel,
                        "panel",
                        READS["panel"],
                        # Shown only with the answer whole (PanelAnswer.tsx).
                        lambda text: "Queda también en tu conversación" in text,
                        self.bounds["screen"][0],
                    ),
                )
        if panel is not None:
            drafts_panel.panel_holds_answer(self, panel, answered_at)

        # After «Oye Deimos»: the input change restarts its listening, and a
        # phrase said right after was revised and abandoned (run 20261028T131543Z).
        audio_route.exercise(self)

        meeting_controls.second_half(self)

        with self.step("end the recording from the interface", (1, 2, 3, 4, 5, 6)):
            # «Terminar» is always in sight on the sidebar's recording card
            # (RecordingCard.tsx; run 14 found the old pill's only under a mouse
            # hover): pressed by accessibility, as the keyboard would.
            ui.press(self.main, "Terminar", ("AXButton",), first=True)

        with self.step("the saved recording", (1, 2, 3, 4, 5, 6)):
            saved = self.wait_mac("saved", self.ev["recording"]["id"])
            self.ev["recording"].update(saved)

        identifier = self.ev["recording"]["id"]
        with self.step("the note", (1, 2, 3, 4, 5, 6), critical=False):
            self.ev["note_phase"] = self.wait_mac("note", identifier)
            self.ev["detail"] = self.mac.read("detail", identifier)["items"][0]

        # «Reunión guardada» opens nothing: the note is not written yet.
        # «Tu nota está lista» says when it is, with «Abrir», and goes away by
        # itself after ten seconds; a notice can disappear between reads, so
        # it is pressed as soon as the note is final, before anything else
        # waits.
        with self.step("«Abrir» from «Tu nota está lista»", (6,), critical=False):
            ui.press(self.main, "Abrir", ("AXLink", "AXButton"))
            # What the page shows once it opens.
            ui.read(self.main, "find the note just opened (whole window)", None)

        with self.step("the note on screen", (1, 2, 3, 4, 5, 6), critical=False):
            # Until the interface stops saying the note is on its way.
            self.measure(
                "screen",
                ui.read_until(
                    self.main,
                    "note-ready",
                    READS["note-ready"],
                    lambda text: (
                        not any(
                            t in text
                            for t in (
                                "Transcribiendo",
                                "Escribiendo la nota",
                                "Escribiendo el resumen",
                            )
                        )
                    ),
                    self.bounds["screen"][0],
                ),
            )

        with self.step("the note the notice opened", (6,), critical=False):
            title = checks._title(self.ev)
            self.measure(
                "screen",
                ui.read_until(
                    self.main,
                    "note-from-notice",
                    READS["note-from-notice"],
                    lambda text: title in text,
                    self.bounds["screen"][0],
                ),
            )
            # The filtered read says only that the title is missing; the whole
            # window says what is there instead.
            ui.read(self.main, "find the notice's note (whole window)", None)

        with self.step("open the note from the list of meetings (Reuniones)", (6,), critical=False):
            # The sidebar names the section «Reuniones», a link (Sidebar.tsx). An
            # open note's breadcrumb in the top bar is a second «Reuniones»
            # link; the sidebar comes first in the tree.
            ui.press(self.main, "Reuniones", ("AXLink",), first=True)
            # The list and the opened note name it by the meeting's title
            # (decided on 2026-10-17).
            title = checks._title(self.ev)
            if not title:
                raise StepFailed("the note has no title to find it by in the list")
            # Each meeting of the list is a link (MeetingsColumn.tsx).
            ui.press(self.main, title, ("AXLink", "AXButton"), contains=True, first=True)
            self.measure(
                "screen",
                ui.read_until(
                    self.main,
                    "note-from-list",
                    READS["note-from-list"],
                    lambda text: title in text,
                    self.bounds["screen"][0],
                ),
            )

        with self.step("the note's tabs", (1, 2, 3, 4, 5), critical=False):
            for tab, label in (
                ("Resumen", "note-summary"),
                ("Acciones", "note-actions"),
                ("Transcripción", "note-transcript"),
            ):
                # «Acciones · 3»: the open tasks are counted in the tab's name.
                ui.press(
                    self.main,
                    tab,
                    ("AXRadioButton", "AXTab", "AXButton"),
                    contains=True,
                    first=True,
                )
                ui.read(self.main, label, READS[label])

        with self.step("the Assistant", (8,), critical=False):
            # What is typed to the assistant, in the product's language.
            message = (
                f"Prueba de Orbit {uuid.uuid4().hex[:6]}: contesta sólo con la palabra recibido."
            )
            self.ev["chat"] = {"message": message}
            ui.press(self.main, "Asistente", ("AXLink",))
            # «Mensaje para Deimos»: the field names the conversation's agent.
            ui.type_into(self.main, "Mensaje para", message, contains=True)
            ui.press(self.main, "Enviar", ("AXButton",))
            done = self.wait_mac("chat", message)
            self.ev["chat"].update(done)
            answered = self.bounds["screen"][0]
            self.measure(
                "screen",
                ui.read_until(
                    self.main,
                    "assistant-answered",
                    READS["assistant-answered"],
                    lambda text: (
                        message in text and not checks._missing_words(done["answer"], text)
                    ),
                    answered,
                ),
            )
            # The app names a conversation by its start on the Mac's clock;
            # mac.py writes that same name as `app_title`.
            sessions = self.mac.read("sessions")["items"]
            earlier = next((s for s in sessions if s.get("id") == done["session"]), None)
            if earlier is None or not earlier.get("app_title"):
                raise StepFailed(
                    f"conversation {done['session']} is not among the {len(sessions)} "
                    "the hub lists, with its start date"
                )
            ui.press(self.main, "Nueva conversación", ("AXButton",), contains=True, first=True)
            # Each conversation of the list is a link named by its start («Hoy, 09:31»).
            ui.press(self.main, earlier["app_title"], ("AXLink",), contains=True, first=True)
            self.wait_mac("session", done["session"])
            self.measure(
                "screen",
                ui.read_until(
                    self.main,
                    "assistant-reopened",
                    READS["assistant-reopened"],
                    lambda text: message in text,
                    self.bounds["screen"][0],
                ),
            )

        # The voice note and the call, after the meeting has ended: nothing
        # records while they run.
        with self.step("a voice note to Deimos", (11,), critical=False):
            spoken = self.mac.voice_note()
            self.ev["voice_note"] = {
                "phrase": mac_side.VOICE_NOTE,
                "spoken": spoken,
                **self.wait_mac("voice-note"),
            }

        with self.step("a call with Deimos", (12,), critical=False):
            self.mac.command("call_start")
            try:
                self.ev["call"] = self.wait_mac("call-listening")
            finally:
                # Also when it never listened: no call stays open.
                self.mac.command("call_stop")

        with self.step("final screen", (9,), critical=False):
            ui.read(self.main, "final", READS["final"])

        # With every check of the first meeting's evidence collected.
        meeting_controls.discard_second_recording(self)
        meeting_controls.delete_first_meeting(self)
        silence_recording.record_without_answer(self)
        # Last: it quits Orbit Pruebas and opens it again.
        drafts_panel.drafts_survive_quitting(self)

    def open_main(self, label: str):
        """Orbit Pruebas's main window, brought to the front and read as
        `label`, past the welcome of a first run (first_run.py)."""
        ui = self.ui
        # The window appears after the app's first paint, a little after
        # its daemon answers; 20 s is far above the ~1 s it takes, and the
        # bridge offers no window event to wait on instead.
        self.mac.activate()
        deadline = time.monotonic() + 20
        while True:
            try:
                main = ui.windows()["main"]
                break
            except StepFailed:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.5)
        start = time.monotonic()
        first_view_s, screen_s = self.bounds["first-view"][0], self.bounds["screen"][0]
        if first_run.past_the_welcome(ui, main, label, first_view_s, screen_s):
            self.ev["welcome_skipped"].append(label)
        self.measure("first-view", time.monotonic() - start)
        return main

    def clean_up(self) -> None:
        if self.cua_active:
            with self.step("stop CuaDriver left by the accessibility check", critical=False):
                self.ev["cua"].update(self.mac.engine_stop(self.bounds["close"][0]))
                self.cua_active = False
        with self.step("close Orbit Pruebas and give the audio input back", critical=False):
            self.ev["restored"] = self.mac.restore(self.bounds["close"][0])
            came_back = self.ev["restored"]["orbit_pruebas"].get("came_back", 0)
            if came_back:
                raise StepFailed(
                    f"Orbit Pruebas came back after closing ({came_back} processes); "
                    "they were stopped"
                )
        with self.step("collect the Mac's logs", (10, 12), critical=False):
            collected = self.mac.logs()
            logs = collected["logs"]
            self.ev["logs"] = logs
            for name, text in logs.items():
                (self.directory / name).write_text(text, encoding="utf-8")
            self.ev["call_audio"] = collected["call_audio"]
            (self.directory / "daemon-call.log").write_text(
                collected["call_audio"], encoding="utf-8"
            )
        with self.step("stop the test hub and the test Hermes", critical=False):
            self.ev["stopped"] = [p.stop(self.bounds["stop"][0]) for p in reversed(self.processes)]
            log = self.directory / "hub.log"
            self.ev["hub_log"] = log.read_text(encoding="utf-8") if log.exists() else None

    def run(self) -> list[dict]:
        self.directory.mkdir(parents=True, exist_ok=True)
        (self.directory / "trees").mkdir(exist_ok=True)
        try:
            self.services()
            self.mac_ready()
            self.drive_interface()
        except RunStopped:
            pass
        except KeyboardInterrupt as cause:
            instant = time.time()
            self.ev["steps"].append(
                {
                    "name": "interrupted",
                    "start": instant,
                    "end": instant,
                    "checks": [],
                    "ok": False,
                    "cause": f"the test was interrupted ({cause or 'Ctrl-C'})",
                }
            )
        finally:
            self.clean_up()
        if self.ui is not None:
            self.ev["trees"] = self.ui.trees
            self.ev["reads"] = self.ui.reads
        results = all_checks.run_checks(self.ev)
        write_report(self.directory, self.ev, results)
        if not checks.passed(results) and self.paths.hub.exists():
            # The hub's data (the synthetic meeting's audio, its transcript, its
            # final pass) is wiped by the next run. A transcript that lost words
            # in 2 of 6 real runs (2026-10-20) could not be examined afterwards:
            # a failed run keeps what the hub kept, nothing more than invented
            # speech, in this run's own directory.
            shutil.copytree(self.paths.hub, self.directory / "hub-data")
        return results


def write_report(directory: Path, ev: dict, results: list[dict]) -> None:
    """report.json for machines, report.txt for people. No content but the
    synthetic meeting's, which is invented."""
    data = {
        "results": results,
        "steps": ev["steps"],
        "waits": ev["waits"],
        "mac": ev.get("mac"),
        "recording": ev.get("recording"),
        "stopped": ev.get("stopped"),
        "restored": ev.get("restored"),
        "reads": ev.get("reads"),
        "cua": ev.get("cua"),
        "accessibility": ev.get("accessibility"),
        "discard": ev.get("discard"),
        "delete": ev.get("delete"),
        "panel_hold": ev.get("panel_hold"),
        "silence": ev.get("silence"),
        "audio_route": ev.get("audio_route"),
        "drafts": ev.get("drafts"),
        "welcome_skipped": ev.get("welcome_skipped"),
    }
    (directory / "report.json").write_text(
        json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    lines = [f"Real test on the Mac · {directory.name}", ""]
    for r in results:
        lines.append(f"{r['number']:>2}. [{r['status']}] {r['name']}")
        lines.append(f"    {r['detail']}")
        for row in r.get("log", []):
            lines.append(f"    log {row['log']}: code {row['code']}, op {row['op']}")
    if ev.get("welcome_skipped"):
        where = ", ".join(ev["welcome_skipped"])
        skipped = f"Orbit Pruebas {first_run.REPORTED} ({where}); «{first_run.SKIP}» skipped it."
        lines += ["", skipped]
    lines += ["", "Not covered:"]
    lines += [f"  {text}" for text in all_checks.NOT_COVERED]
    lines += ["", "Steps:"]
    for s in ev["steps"]:
        status = "ok" if s.get("ok", True) else f"FAILED: {s.get('cause')}"
        lines.append(f"  {s['name']}: {status} ({s.get('end', s['start']) - s['start']:.1f} s)")
    lines += ["", "Bridge reads (bytes of the in-app reply):"]
    for read in ev.get("reads") or []:
        lines.append(
            f"  {read['label']} [{read['query'] or 'whole window'}]: "
            f"{read['bytes'] if read['bytes'] is not None else '—'} bytes"
        )
    lines += ["", "Waits (what each took and its bound):"]
    for w in ev["waits"]:
        lines.append(
            f"  {w['wait']}: {w.get('seconds', '—')} s of {w['max_s']:.0f} s {w.get('failure', '')}"
        )
    (directory / "report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


# ── Entry points ────────────────────────────────────────────────────────────


def run_directory(base: Path) -> Path:
    return base / "runs" / datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")


def tailnet_ip() -> str:
    done = subprocess.run(
        ["tailscale", "ip", "-4"], capture_output=True, text=True, timeout=15, check=True
    )
    address = done.stdout.split()[0]
    if isolated_hub.ipaddress.ip_address(address) not in isolated_hub.TAILNET:
        raise RuntimeError(f"`tailscale ip -4` gave {address}, outside the tailnet")
    return address


def run_real() -> int:
    paths = isolated_hub.Paths(BASE)
    isolated_hub.config_text(paths, "127.0.0.1", isolated_hub.HERMES_PORT)
    isolated_hub.required_environment("ORBIT_TEST_HERMES_TOKEN_ENV")
    for secret in (paths.hermes_token,):
        if not secret.exists() or secret.stat().st_mode & 0o077:
            raise SystemExit(
                f"{secret} with mode 0600 is missing: run «real_test.py prepare» first"
            )
    try:
        mac = RemoteMac(
            os.environ.get("ORBIT_MAC_HOST", ""), os.environ.get("ORBIT_MAC_SSH_KEY", "")
        )
        host = tailnet_ip()
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as cause:
        raise SystemExit(f"cannot start: {cause}") from None
    for address, port in (
        (host, isolated_hub.HUB_PORT),
        ("127.0.0.1", isolated_hub.HERMES_PORT),
    ):
        if not isolated_hub.port_free(address, port):
            raise SystemExit(
                f"port {address}:{port} is in use; the test does not touch what uses it"
            )
    directory = run_directory(BASE)
    test = RealTest(mac, paths, host, isolated_hub.HUB_PORT, directory, hermes=True)

    # Closing the terminal or a plain `kill` still cleans up: the run goes to
    # its `finally` as with Ctrl-C.
    def interrupt(number, _frame):
        raise KeyboardInterrupt(f"signal {number}")

    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGHUP, interrupt)
    results = test.run()
    print((directory / "report.txt").read_text(encoding="utf-8"))
    return 0 if checks.passed(results) else 1


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Orbit's real test on the Mac")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("prepare", help="creates, once, the test's keys in ~/.orbit-pruebas")
    commands.add_parser("run", help="the real test, against the Mac")
    dry_run_parser = commands.add_parser(
        "dry-run", help="a dry run on Linux against example states; not the test"
    )
    dry_run_parser.add_argument("--example", default="good")
    dry_run_parser.add_argument("--directory", type=Path)
    options = parser.parse_args(argv)
    if options.command == "prepare":
        created = isolated_hub.prepare_secrets(isolated_hub.Paths(BASE))
        print("Created: " + (", ".join(created) if created else "nothing; they already existed"))
        return 0
    if options.command == "run":
        return run_real()
    import dry_run

    results, directory = dry_run.run(options.example, options.directory)
    print((directory / "report.txt").read_text(encoding="utf-8"))
    return 0 if checks.passed(results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
