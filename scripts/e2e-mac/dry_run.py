"""The dry run on Linux: the orchestrator and the checks against example
states, never against Orbit.

What is real: the orchestrator (real_test.py), the Mac-side steps and waits
(mac.py) with their predicates, the reading of the trees and the search for
what to click (interface.py, ax_tree.py), the checks, the report, the
reading of the test hub's database (hub_traces.py), and the test hub itself —
started from this tree on loopback with a Whisper and a note model that do
not exist and are never called, and stopped again.

What is replayed from scripts/e2e-mac/dry-run-examples/<example>/: the Mac's
system commands, the in-app bridge's semantic trees (screens/), menu reply
(bridge.json), and transitions of presses, typing, pointer motion and quits
(transitions.json); the daemon's socket and logs; CuaDriver's two independent
accessibility snapshots (accessibility-*.md); and what the hub keeps (hub.json).
The fake CuaDriver refuses every input tool. An example other than good holds
only the files that differ from good.

A dry run that passes says the logic of the test holds. It is not the real
test and does not count as one.
"""

from __future__ import annotations

import json
import plistlib
import socket
import tempfile
import time
from pathlib import Path

import isolated_hub
import mac as mac_side
import mac_engine
import real_test
from dry_bridge import MAIN_WINDOW, PANEL_WINDOW, PID, DryRunBridge, DryRunView, query_filter
from interface import StepFailed

__all__ = ["PID", "query_filter"]

EXAMPLES = Path(__file__).resolve().parent / "dry-run-examples"
CUA_PID = 4343
# cua-driver walks at most 2 000 nodes of a window (ax/tree.rs) and then says so.
MAX_NODES = 2000


class Example:
    def __init__(self, name: str):
        self.directories = (
            [EXAMPLES / name, EXAMPLES / "good"] if name != "good" else [EXAMPLES / "good"]
        )
        if not self.directories[0].is_dir():
            raise FileNotFoundError(f"no such example {self.directories[0]}")
        self.message = ""

    def path(self, relative: str) -> Path:
        for directory in self.directories:
            if (directory / relative).exists():
                return directory / relative
        raise FileNotFoundError(relative)

    def text(self, relative: str) -> str:
        return self.path(relative).read_text(encoding="utf-8").replace("{message}", self.message)

    def json(self, relative: str):
        return json.loads(self.text(relative))

    def base(self, relative: str):
        """«good»'s own `relative`, which the other examples change in part."""
        return json.loads((EXAMPLES / "good" / relative).read_text(encoding="utf-8"))

    def overrides(self, relative: str) -> dict:
        """The example's own overrides of `relative`, keyed like it."""
        own = self.directories[0] / relative
        if len(self.directories) == 1 or not own.exists():
            return {}
        return json.loads(own.read_text(encoding="utf-8"))


class DryRunSystem(mac_side.System):
    """say, SwitchAudioSource, open, ps and cua-driver, answered like a Mac
    would. Never signals a process: the pids here are invented."""

    def __init__(self, example: Example, paths: mac_side.Paths):
        self.example = example
        self.paths = paths
        self.input = "Micrófono del MacBook Pro"
        self.commands: list[list[str]] = []
        self.app_open = False
        self.signals: list[tuple[int, int]] = []
        self.pauses: list[float] = []
        self.view = DryRunView(example, paths.logs)
        self.cua = DryRunCua(example, self.view)

    def _quit(self) -> None:
        """Orbit Pruebas ends, as on the Apple Event «quit»."""
        self.app_open = False

    def run(self, *command: str) -> tuple[int, str, str]:
        self.commands.append(list(command))
        if command[0] == str(self.paths.cua):
            return self.cua.command(list(command[1:]))
        if command[0] == "SwitchAudioSource":
            if "-a" in command:
                return 0, f"Micrófono del MacBook Pro\n{mac_side.BLACKHOLE}\n", ""
            if "-c" in command:
                return 0, self.input + "\n", ""
            self.input = command[-1]
            return 0, "", ""
        if command[:3] == ("say", "-v", "?"):
            return 0, self.example.text("voices.txt"), ""
        if command[:4] == ("open", "-n", "-g", "-a"):
            return self.cua.start(list(command))
        if command[0] == "open":
            # Orbit Pruebas «starts»: its logs are the example's, and on a
            # second start they are still the first one's (the data stays).
            if not self.app_open:
                self.view.generation += 1
            self.app_open = True
            self.paths.logs.mkdir(parents=True, exist_ok=True)
            for name in ("report-daemon.log", "report-app.log", "daemon.log"):
                if not (self.paths.logs / name).exists():
                    (self.paths.logs / name).write_text(
                        self.example.text(f"logs/{name}"), encoding="utf-8"
                    )
            return 0, "", ""
        if command[0] == "osascript":
            # The Apple Event «quit», what the Dock sends.
            self._quit()
            self.view.follow("apple-event:quit")
            return 0, "", ""
        if command[0] == "swift" and command[1].endswith("pointer.swift"):
            return self.view.pointer(int(command[2]), command[3])
        if command[0] == "sw_vers":
            return 0, "26.1\n", ""
        if list(command) == ["id", "-F"]:
            return 0, "Alina Pérez\n", ""
        if command[0] == "swift" and command[1].endswith("windows.swift"):
            # WindowServer's listing: the main window at layer 0 and, on
            # screen above it, the floating panel (layer 3, always on top).
            rows = [
                {"window_id": MAIN_WINDOW, "layer": 0, "is_on_screen": True},
                {"window_id": PANEL_WINDOW, "layer": 3, "is_on_screen": True},
            ]
            return 0, "".join(json.dumps(row) + "\n" for row in rows), ""
        if command[0] == "say":
            if self.cua.running:
                raise mac_side.MacFailure(
                    "CuaDriver was left running outside the accessibility step"
                )
            return 0, "", ""
        return 1, "", f"command the dry run does not expect: {command}"

    def has_command(self, name: str) -> bool:
        return True

    def pause(self, seconds: float) -> None:
        # Nothing plays here: the beat is noted, never slept.
        self.pauses.append(seconds)

    def local_time(self, seconds: float) -> time.struct_time:
        # The examples' Mac lives in UTC, so their titles do not depend on
        # the clock of the machine that runs the dry run.
        return time.gmtime(seconds)

    def processes(self) -> list[tuple[int, str]]:
        rows = []
        if self.app_open:
            rows.append((self.view.pid, str(self.paths.app / mac_side.MEMBERS[0])))
        if self.cua.running:
            rows.append((CUA_PID, mac_engine.CUA_APP + "Contents/MacOS/CuaDriver"))
        return rows

    def signal(self, pid: int, number: int) -> None:
        self.signals.append((pid, number))
        if pid == self.view.pid:
            self.app_open = False

    def wait_exit(self, pids: list[int], max_s: float) -> list[int]:
        if self.view.pid in pids:
            self.app_open = False
        return []


class DryRunOrbit:
    """The daemon's socket: each subscription gets the next wait of the
    example. A wait that reads (the meeting saved, its note final) answers its
    reads, in order, from that wait's own `reads`; the last one stays. An
    example changes a wait with its frames (a list) or with
    `{"frames": …, "reads": …}`, or leaves it out with null."""

    def __init__(self, example: Example):
        self.example = example
        self.waits = example.base("ipc.json")["waits"]
        self.next = 0
        self.current: list | None = None

    def answers(self) -> bool:
        return True

    def _wait(self, wait: dict) -> dict:
        changed = self.example.overrides("ipc.json").get("waits", {}).get(wait["wait"])
        if isinstance(changed, list):
            changed = {"frames": changed}
        return {**wait, **(changed or {})}

    def call(self, body: dict, bound_s: float = 30) -> dict:
        if body["command"] == "pair" or body in mac_side.COMMANDS.values():
            return {"ok": True}
        if body["command"] == "read":
            kind = body["read"]["kind"]
            if self.current and kind == "notes_page":
                value = self.current.pop(0) if len(self.current) > 1 else self.current[0]
            else:
                reads = self.example.base("ipc.json")["reads"]
                reads.update(self.example.overrides("ipc.json").get("reads", {}))
                value = reads[kind]
            return {"ok": True, "answer": {"state": "present", "value": {"items": value}}}
        raise mac_side.MacFailure(f"command the dry run does not expect: {body['command']}")

    def subscribe(self, topics: list[str], bound_s: float, beat_s: float | None = None):
        # A wait the example sets to null is one its run never reaches.
        skipped = {
            k for k, v in self.example.overrides("ipc.json").get("waits", {}).items() if v is None
        }
        while self.next < len(self.waits) and self.waits[self.next]["wait"] in skipped:
            self.next += 1
        if self.next >= len(self.waits):
            raise TimeoutError
        wait = self._wait(self.waits[self.next])
        self.next += 1
        if sorted(wait["topics"]) != sorted(topics):
            raise mac_side.MacFailure(
                f"the dry run expected «{wait['wait']}» with {wait['topics']}, not {topics}"
            )
        if ("reads" in wait) != (beat_s is not None):
            raise mac_side.MacFailure(
                f"«{wait['wait']}»: the dry run and the wait disagree on whether it reads"
            )
        self.current = list(wait.get("reads") or [])
        try:
            text = json.dumps(wait["frames"])
            for frame in json.loads(text.replace("{message}", self.example.message)):
                yield frame["topic"], frame["knowledge"]
            # The beats of a reading wait: one per read still to answer.
            for _ in range(len(self.current)):
                yield None, None
        finally:
            self.current = None
        raise TimeoutError


# What macOS answered on a real run before WebKit had built its tree: only
# the window's own buttons.
WINDOW_CHROME_ONLY = """- [0] AXWindow "Orbit" [actions=[raise]]
  - [1] AXButton [actions=[press]]
  - [2] AXButton [actions=[press,zoomwindow,showmenu]]
  - [3] AXButton [actions=[press]]
"""


class DryRunCua:
    """CuaDriver lifecycle and read-only AX snapshots. It cannot drive the UI;
    cua.json can refuse an accessibility reading, or answer its first reads of
    each state with the window's own buttons only (chrome_only_reads)."""

    def __init__(self, example, view):
        self.example, self.view = example, view
        self.refusals = example.overrides("cua.json").get("refusals", {})
        self.chrome_only = example.overrides("cua.json").get("chrome_only_reads", 0)
        self.reads = {}
        self.running = False
        self.mode = example.overrides("cua.json").get("mode", "bounded")
        self.serve = None
        self.stopped = False

    def start(self, command: list[str]) -> tuple[int, str, str]:
        self.serve = command
        self.running = True
        return 0, "", ""

    def command(self, arguments: list[str]) -> tuple[int, str, str]:
        if arguments == ["status"]:
            if not self.running:
                return 1, "", "Cua Driver daemon is not running"
            manifest = "true" if self.mode == "bounded" else "false"
            status = (
                "Cua Driver daemon is running\n"
                "  socket: /Users/dryrun/Library/Caches/cua-driver/cua-driver.sock\n"
                f"  pid: {CUA_PID}\n"
                f"  permission mode: {self.mode} (launch)\n"
                f"  capability manifest: configured={manifest}, "
                f"approved_at_startup={manifest}, valid={manifest}\n"
            )
            return 0, status, ""
        if arguments[:1] == ["--expected-pid"] and arguments[2:] == ["stop"]:
            if int(arguments[1]) == CUA_PID:
                self.running = False
                self.stopped = True
            return 0, "", ""
        if arguments[:1] == ["call"] and len(arguments) == 3:
            if not self.running:
                return 1, "", "Cua Driver daemon is not running"
            tool, data = arguments[1], json.loads(arguments[2])
            if tool in self.refusals:
                return 1, "", self.refusals[tool]
            try:
                return 0, json.dumps(self.call(tool, data), indent=2) + "\n", ""
            except StepFailed as cause:
                return 1, "", str(cause)
        return 1, "", f"cua-driver command the dry run does not expect: {arguments}"

    def call(self, tool, arguments):
        if tool != "get_window_state" or arguments.get("pid") != self.view.pid:
            raise StepFailed(f"CuaDriver only permits the read-only accessibility check: {tool}")
        phase = "idle" if self.view.screen in ("home", "briefing") else "recording"
        self.reads[phase] = self.reads.get(phase, 0) + 1
        if self.reads[phase] <= self.chrome_only:
            return {"tree_markdown": WINDOW_CHROME_ONLY}
        tree = self.example.text(f"accessibility-{phase}.md")
        lines = tree.splitlines()
        if len(lines) > MAX_NODES:
            tree = "\n".join(lines[:MAX_NODES]) + f"\nAX tree truncated at {MAX_NODES} nodes\n"
        return {"tree_markdown": tree}


class DryRunHub:
    """What the test hub keeps of a recording. The real reading runs first,
    against the dry run's own hub, which never has a recording; the answer is
    then the example's hub.json, in order for each recording, the last one
    staying."""

    def __init__(self, example: Example, read):
        answers = example.base("hub.json")
        answers.update(example.overrides("hub.json"))
        self.answers = {k: list(v) for k, v in answers.items()}
        self.read = read

    def __call__(self, recording: str) -> dict:
        real = self.read(recording)
        if real["rows"] or real["deleted"]:
            raise StepFailed(f"the dry run's hub keeps something of {recording}: {real}")
        answers = self.answers.get(recording) or [real]
        return answers.pop(0) if len(answers) > 1 else answers[0]


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def run(name: str, directory: Path | None = None) -> tuple[list[dict], Path]:
    example = Example(name)
    root = Path(tempfile.mkdtemp(prefix="orbit-dry-run-")) if directory is None else directory
    # The Mac's home, with Orbit Pruebas «installed».
    home = root / "mac"
    mac_paths = mac_side.Paths(home)
    (mac_paths.app / "Contents").mkdir(parents=True)
    (mac_paths.app / "Contents" / "Info.plist").write_bytes(
        plistlib.dumps({"CFBundleIdentifier": mac_side.BUNDLE_ID})
    )
    system = DryRunSystem(example, mac_paths)
    mac = real_test.InProcessMac(
        mac_side.Mac(mac_paths, system, DryRunOrbit(example), DryRunBridge(example, system.view))
    )
    paths = isolated_hub.Paths(root / "orbit-pruebas")
    isolated_hub.prepare_secrets(paths)
    key = paths.secrets / "fake-processing.key"
    isolated_hub.write_private(key, b"dry-run")
    # Nothing listens on port 9 (discard), and «.invalid» never resolves: the
    # dry run's hub has no recordings, so neither is ever called.
    providers = {
        "whisper": "http://127.0.0.1:9",
        "processing": "https://processing.invalid/v1/chat/completions",
        "processing_key": str(key),
    }
    run_directory = root / "run"

    test = real_test.RealTest(
        mac,
        paths,
        "127.0.0.1",
        free_port(),
        run_directory,
        hermes=False,
        providers=providers,
    )
    # A replayed screen never changes: reading it again could not help. Nor
    # does a replayed hub.
    test.bounds["screen"] = (0, "in the dry run the screens never change by themselves")
    for wait in ("hub-chunks", "hub-deleted", "silence-hub"):
        test.bounds[wait] = (0, "in the dry run the test hub's answers are replayed")
    # Nor does a replayed panel change with time: it is read at once.
    test.bounds["answer-held"] = (0, "in the dry run the panel never changes by itself")
    test.bounds["accessibility-tree"] = (2, "cua.json may hold back the first reads")
    test.hub_kept = DryRunHub(example, test.hub_kept)
    return test.run(), run_directory
