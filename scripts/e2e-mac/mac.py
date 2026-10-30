"""The Mac side of the real test: what runs on the test Mac, over SSH.

The orchestrator on the test server copies this file, the modules it imports
(mac_bridge.py, mac_controls.py, mac_drafts_panel.py, mac_engine.py, mac_system.py and
mac_waits.py), the Swift scripts and the meeting script to
`~/.orbit-pruebas/harness/` on the Mac and calls one subcommand at a time;
each prints one JSON object and exits 0, or prints `{"error": …}` and
exits 1.

    prepare --max S           checks, BlackHole as the default input, clean data and drafts, opens Orbit Pruebas
    pair URL CODE NAME
    wait WHAT [ARG] --max S   waits on a fact Orbit Pruebas publishes or answers on its socket
    read detail ID | read sessions
    say PART                  says one part of the script into BlackHole (PARTS)
    remote-audio ACTION       check 25 preflight, fixture player, speech and cleanup
    voice-note                a voice note to Deimos: voice_start, one phrase into BlackHole, voice_end
    command call_start|call_stop  starts or stops a call with Deimos
    restore --max S           closes Orbit Pruebas and puts the previous input back
    launch --max S            opens Orbit Pruebas again and waits for its daemon
    draft-and-quit HOW JSON S --max S  types a draft, quits Orbit Pruebas (the Dock's quit)
    panel-pointer ID over|away Q --max S  moves the pointer over the panel or away from it
    logs                      the report logs of Orbit Pruebas and the call's audio lines of daemon.log
    activate                  brings Orbit Pruebas to the front
    floating                  Orbit Pruebas's windows above the normal layer (the floating panel)
    bridge-call JSON         one bounded request to the in-app bridge
    bridge-menus             the last menus built inside Orbit Pruebas (check 16 only)
    engine-start --max S      starts CuaDriver, bounded to Orbit Pruebas, under its own identity
    engine-call TOOL JSON     one CuaDriver tool call
    engine-stop --max S       stops the CuaDriver this test started, by its pid

The in-app bridge drives all web-view presses, typing and reads. CuaDriver
is started only for read-only macOS accessibility samples (check 21), under
its own identity and bounded to Orbit Pruebas's exact pid. Pointer motion
uses pointer.swift; the Dock's quit uses an Apple Event.

It never touches the user's Orbit, its data or its Keychain: Orbit Pruebas has
its own bundle, data directory and Keychain service (orbit_bundle, feature
`pruebas`). It stops only the processes of the Orbit Pruebas bundle, found by
their exact executable path, never by name.

Runs with the Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import datetime
import json
import plistlib
import re
import select
import shutil
import signal
import socket
import sys
import time
import uuid
from pathlib import Path

import audio_route
from harness_copy import language
from harness_copy import text as copy_text
from mac_bridge import TestBridge
from mac_controls import PARTS, MacFailure, script_parts
from mac_drafts_panel import RELAUNCH_WATCH_S, DraftsAndPanel
from mac_engine import CuaDriver
from mac_native import NativeChecks
from mac_remote_audio import RemoteAudio
from mac_system import System
from mac_waits import WAITS
from meeting_language import script_path, voice_note_phrase

BUNDLE_ID = "app.orbit.companion.pruebas"
# The last part of orbit_bundle::DATA_DIRECTORY with the feature `pruebas`
# (directories::ProjectDirs on macOS). contracts/constants.json registers it
# (PRUEBAS_DATA_DIRECTORY): its guard compares this copy, and orbit-core's
# pruebas test the Rust one.
DATA_DIRECTORY = "com.MarsGotta.OrbitPruebas"
SCRIPT = Path(__file__).resolve().with_name("meeting-script.txt")
BLACKHOLE = "BlackHole 2ch"
# CuaDriver only reads the app's macOS accessibility tree for check 21.
# No input tool is granted; the in-app bridge owns DOM actions.
TOOLS = ("get_window_state",)
VALIDITY = "3h"
IDLE = "1h"


class Paths:
    """Where everything lives, from the home directory (a temporary one in the dry run)."""

    def __init__(self, home: Path):
        self.app = home / "Applications" / "Orbit Pruebas.app"
        # The only sender of real keys (build-fixture-apps.sh): launched with
        # `open`, it holds its own Accessibility permission, never SSH's.
        self.keys_app = home / "Applications" / "Orbit Pruebas Teclas.app"
        # The silent far side of a meeting for check 25, signed the same way.
        self.remote_app = home / "Applications" / "Orbit Pruebas Remote.app"
        self.data = home / "Library" / "Application Support" / DATA_DIRECTORY
        self.socket = self.data / "orbit.sock"
        self.bridge = self.data / "orbit-test-bridge.sock"
        self.logs = self.data / "logs"
        self.own = home / ".orbit-pruebas"
        # File name kept from the earlier, Spanish harness: a run of it that
        # died left the input to put back here, and the next run restores it.
        self.previous_input = self.own / "entrada-anterior"
        # WebKit keeps the webview's local storage here, outside the data
        # directory, under the bundle identifier: the unsent drafts, the first
        # run's completion, the theme.
        self.webkit = home / "Library" / "WebKit" / BUNDLE_ID
        # CuaDriver's command line, from its official installer, and what the
        # test hands it: the manifest it is bounded by and the pid it started.
        self.cua = home / ".local" / "bin" / "cua-driver"
        self.manifest = self.own / "cua-manifest.yaml"
        self.cua_pid = self.own / "cua-driver.pid"
        # The helper latches the privacy stop in a file every Orbit on this
        # Mac shares (PrivacyLatch.swift); with it set, Orbit Pruebas starts
        # stopped.
        self.privacy_latch = (
            home / "Library" / "Application Support" / "Orbit" / "native-privacy-stop"
        )


CUA_ENGINE_RELATIVE_PATH = "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
MEMBERS = (
    "Contents/MacOS/Orbit",
    "Contents/MacOS/OrbitNative",
    "Contents/" + CUA_ENGINE_RELATIVE_PATH,
)
# Parked copies keep the installer's «Orbit.app.anterior-» prefix
# (orbit_bundle::PARKED_PREFIX).
OFFICIAL = re.compile(r"/Orbit\.app(\.anterior-[^/]+)?/Contents/MacOS/Orbit$")
# One request line to the daemon (crates/orbit-ipc, PROTOCOL_VERSION).
PROTOCOL = 1
# Between two reads of a wait whose fact only a read says (the meeting saved
# and listed, its note final: the `notes` topic is a head of revisions, not
# the list). A change of that head reads at once; this only covers a change
# the head does not name, and asks the hub at most once every two seconds,
# nothing next to the waits' bounds (minutes) or the hub's own pace (a
# transcription group every 20-30 s). It spaces reads; the bound is the wait's.
READ_INTERVAL_S = 2.0
# One line of the socket (crates/orbit-ipc, its 1 MiB frame): a line longer
# than this is not the daemon's.
MAX_LINE_BYTES = 1024 * 1024
# How the daemon names the user's microphone (orbit_core::notes::owner_name):
# the account's full name, or «Yo» when it has none or it is longer than the
# hub's MAX_PERSON_NAME_CHARS (contracts/constants.json guards the length).
MAX_PERSON_NAME = 128
# The beats around the phrase. Not waits on a fact: no topic says when the
# microphone delivers its first frame after voice_start (voice_active turns
# true when the order is taken, before the helper opens the stream) or when
# the last frame said has reached the daemon before voice_end. These are the
# values of the probe that worked on the Mac on 2026-10-17, unmeasured
# beyond it. A beat too short shows as words missing from the transcript,
# which check 11 names; it never makes the check pass.
BEAT_BEFORE_SPEAKING_S = 0.5
BEAT_AFTER_SPEAKING_S = 0.7
# Calls have no duration ceiling. The harness explicitly hangs up; an
# unanswered silence question is the fallback, using the Mac's own policy.
# The commands the test sends besides `pair`, and their bodies
# (orbit_core::protocol::Command).
COMMANDS = {
    "voice_start": {"command": "voice_start"},
    "voice_end": {"command": "voice_end"},
    "call_start": {"command": "call_start"},
    "call_stop": {"command": "call_stop"},
}
# The daemon.log events the test brings back (call.rs): the call's
# microphone as it opened and what it heard. Only numbers; daemon.log's
# other lines may name paths on the Mac and stay there.
CALL_EVENTS = ("call_audio_opened", "call_audio_heard")

# ── Orbit Pruebas's socket ──────────────────────────────────────────────────


class OrbitIpc:
    """One request per connection, one JSON object per line (crates/orbit-ipc)."""

    def __init__(self, path: Path):
        self.path = path

    def _connect(self, bound_s: float) -> socket.socket:
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.settimeout(bound_s)
        connection.connect(str(self.path))
        return connection

    @staticmethod
    def _request(body: dict) -> bytes:
        line = {"v": PROTOCOL, "id": f"real-test-{uuid.uuid4().hex[:12]}", "body": body}
        return (json.dumps(line, ensure_ascii=False) + "\n").encode()

    def answers(self) -> bool:
        try:
            self._connect(1).close()
            return True
        except OSError:
            return False

    def call(self, body: dict, bound_s: float = 30) -> dict:
        with self._connect(bound_s) as connection:
            connection.sendall(self._request(body))
            line = connection.makefile("rb").readline()
        if not line:
            raise MacFailure(
                f"the daemon closed the connection without answering «{body.get('command')}»"
            )
        return json.loads(line)

    def subscribe(self, topics: list[str], bound_s: float, beat_s: float | None = None):
        """Yields (topic, knowledge) for the current frames and every change,
        until the bound; then raises TimeoutError. With `beat_s`, also
        yields (None, None) every `beat_s` seconds, frames or not, so a wait
        can read what no frame says.

        Lines are cut from the socket's bytes here, never by a file object's
        readline under a timeout, whose buffer a timeout may leave
        inconsistent (Python's socket.makefile)."""
        deadline = time.monotonic() + bound_s
        beat = time.monotonic() + beat_s if beat_s is not None else None
        with self._connect(bound_s) as connection:
            connection.sendall(self._request({"command": "subscribe_topics", "topics": topics}))
            connection.setblocking(False)
            pending = b""
            first = True
            while True:
                now = time.monotonic()
                if now >= deadline:
                    raise TimeoutError
                if beat is not None and now >= beat:
                    beat = now + beat_s
                    yield None, None
                    continue
                if b"\n" not in pending:
                    wait = deadline - now if beat is None else min(deadline, beat) - now
                    ready, _, _ = select.select([connection], [], [], max(wait, 0))
                    if not ready:
                        continue
                    chunk = connection.recv(65536)
                    if not chunk:
                        raise MacFailure("the daemon closed the subscription")
                    pending += chunk
                    if b"\n" not in pending:
                        if len(pending) > MAX_LINE_BYTES:
                            raise MacFailure(
                                f"a line from the daemon exceeds {MAX_LINE_BYTES} bytes "
                                f"({len(pending)} unterminated)"
                            )
                        continue
                line, pending = pending.split(b"\n", 1)
                row = json.loads(line)
                if first:
                    first = False
                    if not row.get("ok"):
                        raise MacFailure(
                            f"the daemon refused the subscription: {json.dumps(row.get('refusal'))}"
                        )
                    continue
                data = (row.get("frame") or row).get("data") or {}
                if "topic" in data:
                    yield data["topic"], data.get("knowledge") or {}


def session_title(moment: time.struct_time, today: time.struct_time) -> str:
    """When a conversation began, as the app's list writes it from its start
    (session-title.ts `sessionWhen`), using the pass catalogue and the Mac clock."""
    clock = copy_text(f"sessionClock.{moment.tm_hour}", minute=f"{moment.tm_min:02d}")
    day = datetime.date(moment.tm_year, moment.tm_mon, moment.tm_mday)
    now = datetime.date(today.tm_year, today.tm_mon, today.tm_mday)
    if day == now:
        return copy_text("shellInline.todayAt", time=clock)
    if day == now - datetime.timedelta(days=1):
        return copy_text("shellInline.yesterdayAt", time=clock)
    key = f"sessionDate.{day.month}" if day.year == now.year else f"sessionDateYear.{day.month}"
    return copy_text(key, day=day.day, year=day.year, time=clock)


def owner_name(full: str | None) -> str:
    """How the daemon names the user's microphone from the account's full name
    (orbit_core::notes::owner_name, fed by `id -F`)."""
    name = (full or "").split(",")[0].strip()
    if (
        not name
        or len(name) > MAX_PERSON_NAME
        or any(ord(c) < 32 or 127 <= ord(c) < 160 for c in name)
    ):
        return copy_text("core.owner_name")
    return name


# ── The meeting script ──────────────────────────────────────────────────────


def voices(listing: str) -> list[str]:
    """Voices in the pass language, Spain or the United States first: «Mónica
    es_ES  # Hola…». Then those named by language and region, «Eddy (English
    (US))»: macOS has them in both languages, so both passes speak with the
    same voices, and it lists its novelty voices («Bad News») first."""
    region = "es_ES" if language() == "es" else "en_US"
    found = []
    for line in listing.splitlines():
        match = re.match(r"^(?P<name>.+?)\s+(?P<language>[a-z]{2}_[A-Z]{2})\s+#", line)
        if match and match["language"].startswith(language() + "_"):
            name = match["name"].strip()
            found.append(((match["language"] != region, not name.endswith("))")), name))
    return [name for _, name in sorted(found, key=lambda pair: pair[0])]


def required_voices(listing: str) -> list[str]:
    selected = voices(listing)
    minimum = 2 if language() == "en" else 1
    if len(selected) < minimum:
        raise MacFailure(
            f"macOS has {len(selected)} {copy_text('pass.language')} voices for say; "
            f"this pass needs {minimum}; install voices in System Settings"
        )
    return selected


# ── The steps ───────────────────────────────────────────────────────────────


class Mac(DraftsAndPanel, CuaDriver, NativeChecks, RemoteAudio):
    def __init__(
        self,
        paths: Paths,
        system: System | None = None,
        orbit: OrbitIpc | None = None,
        bridge: TestBridge | None = None,
    ):
        self.paths = paths
        self.system = system or System()
        self.orbit = orbit or OrbitIpc(paths.socket)
        self.bridge = bridge or TestBridge(paths.bridge, MAX_LINE_BYTES)

    def bridge_call(self, request: dict) -> dict:
        result = self.bridge.call(request)
        size = getattr(self.bridge, "last_reply_bytes", None)
        return {"result": result, "bytes": size}

    def bridge_menus(self) -> dict:
        return self.bridge.call({"command": "menus"})

    def _ours(self) -> list[int]:
        paths = {str(self.paths.app / member) for member in MEMBERS}
        return [pid for pid, command in self.system.processes() if command in paths]

    def _close_ours(self, max_s: float) -> dict:
        pids = self._ours()
        app = [
            pid
            for pid, command in self.system.processes()
            if command == str(self.paths.app / MEMBERS[0])
        ]
        for pid in app:
            # The app only: its daemon runs inside it and hears the SIGTERM,
            # closes in order and takes the helper and the engine with it;
            # the app follows.
            self.system.signal(pid, signal.SIGTERM)
        left = self.system.wait_exit(pids, max_s)
        for pid in left:
            self.system.signal(pid, signal.SIGKILL)
        # Orbit Pruebas must stay closed. On 2026-10-23 an orderly SIGTERM
        # relaunched it, and it ran all night against a test hub that was
        # gone; the step still said «ok». Whatever came back is stopped and
        # counted, so the step can say so.
        self.system.pause(RELAUNCH_WATCH_S)
        back = self._ours()
        for pid in back:
            self.system.signal(pid, signal.SIGKILL)
        return {"closed": len(pids), "forced": len(left), "came_back": len(back)}

    def _restore_input(self) -> str | None:
        if not self.paths.previous_input.exists():
            return None
        previous = self.paths.previous_input.read_text(encoding="utf-8").strip()
        # Real run of 2026-10-21: the input saved was a pair of headphones no
        # longer connected; SwitchAudioSource refused it and every later run
        # stopped while preparing. Such an input cannot be put back.
        inputs = self.system.run_checked("SwitchAudioSource", "-a", "-t", "input").splitlines()
        if previous not in [i.strip() for i in inputs]:
            self.paths.previous_input.unlink()
            return f"«{previous}» is no longer connected; the current input stays"
        self.system.run_checked("SwitchAudioSource", "-t", "input", "-s", previous)
        self.paths.previous_input.unlink()
        return previous

    def prepare(self, max_s: float) -> dict:
        paths = self.paths
        info = paths.app / "Contents" / "Info.plist"
        if not info.exists():
            raise MacFailure(f"Orbit Pruebas is not installed at {paths.app}")
        identifier = plistlib.loads(info.read_bytes()).get("CFBundleIdentifier")
        if identifier != BUNDLE_ID:
            raise MacFailure(f"{paths.app} is «{identifier}», not {BUNDLE_ID}")
        official = [command for _, command in self.system.processes() if OFFICIAL.search(command)]
        if official:
            raise MacFailure(
                "Orbit is open; quit it before the test (they share the helper's global "
                f"shortcuts): {official[0]}"
            )
        if paths.privacy_latch.exists():
            raise MacFailure(
                f"the privacy stop is latched ({paths.privacy_latch}); rearm Orbit first"
            )
        for command in ("SwitchAudioSource", "say", "open"):
            if not self.system.has_command(command):
                raise MacFailure(f"«{command}» is missing (brew install switchaudio-osx)")
        inputs = self.system.run_checked("SwitchAudioSource", "-a", "-t", "input").splitlines()
        if BLACKHOLE not in [i.strip() for i in inputs]:
            raise MacFailure(
                f"there is no «{BLACKHOLE}» input (brew install blackhole-2ch): {inputs}"
            )
        selected = required_voices(self.system.run_checked("say", "-v", "?"))
        script_parts(script_path().read_text(encoding="utf-8"))
        # What a run that died left running: its CuaDriver, its Orbit Pruebas.
        self.engine_stop(max_s)
        before = self._close_ours(max_s)
        # A run that could not restore the input left the original here.
        self._restore_input()
        paths.own.mkdir(mode=0o700, exist_ok=True)
        current = self.system.run_checked("SwitchAudioSource", "-c", "-t", "input").strip()
        paths.previous_input.write_text(current + "\n", encoding="utf-8")
        self.system.run_checked("SwitchAudioSource", "-t", "input", "-s", BLACKHOLE)
        # Its data, and the drafts in its webview's storage, which keeps the
        # first run's completion and the theme besides.
        if paths.data.exists():
            shutil.rmtree(paths.data)
        # The acceptance flow selects the pass language, regardless of macOS's
        # preferred languages. Only the isolated test identity's data is written.
        paths.data.mkdir(parents=True, mode=0o700, exist_ok=True)
        pass_language = language()
        presentation = paths.data / "presentation.toml"
        presentation.write_text(f'language = "{pass_language}"\n', encoding="utf-8")
        presentation.chmod(0o600)
        drafts = self.clear_web_drafts()
        # Both WebKit and the helper resolve regional preferences. Pin only
        # this isolated app's domain; never change the Mac's global languages.
        locale = "en-US" if pass_language == "en" else "es-ES"
        self.system.run_checked("defaults", "write", BUNDLE_ID, "AppleLanguages", "-array", locale)
        self.launch(max_s)
        macos = self.system.run_checked("sw_vers", "-productVersion").strip()
        # The name the microphone's turns must carry, taken where the daemon
        # takes it, never from the hub, which only repeats what the Mac sent.
        code, full, _ = self.system.run("id", "-F")
        return {
            "voices": selected[:2],
            "language": language(),
            "locale": locale,
            "previous_input": current,
            "macos": macos,
            "closed_before": before,
            "drafts_removed": drafts,
            "owner": owner_name(full if code == 0 else None),
        }

    def pair(self, url: str, code: str, name: str) -> dict:
        reply = self.orbit.call(
            {"command": "pair", "hub_url": url, "code": code, "device_name": name}
        )
        if not reply.get("ok"):
            raise MacFailure(
                f"pairing failed: {json.dumps(reply.get('refusal'), ensure_ascii=False)}"
            )
        return {"paired": True}

    def wait(self, what: str, arg, max_s: float) -> dict:
        """Waits on a fact: on the topic frames, and for a fact only a read
        says, on that read, repeated when the `notes` head moves and at least
        every READ_INTERVAL_S, up to the bound. A wait that times out says
        what it saw last."""
        topics, predicate, read = WAITS[what]
        start = self.system.now()
        state: dict = {}
        beat = READ_INTERVAL_S if read is not None else None
        try:
            for topic, known in self.orbit.subscribe(list(topics), max_s, beat):
                if topic is not None:
                    state[topic] = known
                if read is not None and (topic is None or topic == "notes" or "read" not in state):
                    state["read"] = self._read(read(arg))
                found = predicate(state, arg)
                if found is not None:
                    return {
                        "value": found,
                        "seconds": round(self.system.now() - start, 3),
                    }
        except TimeoutError:
            pass
        seen = f"; last seen: {state['seen']}" if state.get("seen") else ""
        raise MacFailure(f"«{what}» did not arrive within {max_s:.0f} s{seen}")

    def _read(self, read: dict) -> dict:
        """One read: its items, or the daemon's answer when it has none."""
        reply = self.orbit.call({"command": "read", "read": read})
        answer_ = reply.get("answer") or {}
        if answer_.get("state") != "present":
            return {"refusal": answer_ or reply.get("refusal")}
        return {"items": answer_["value"].get("items") or []}

    def read(self, what: str, ident: str | None = None) -> dict:
        if what == "detail":
            request = {"kind": "note_detail", "id": ident, "cursor": None}
        elif what == "sessions":
            request = {"kind": "sessions", "cursor": None}
        else:
            raise MacFailure(f"unknown read «{what}»")
        found = self._read(request)
        if "refusal" in found:
            raise MacFailure(
                f"the daemon has no «{what}»: "
                f"{json.dumps(found['refusal'], ensure_ascii=False)[:300]}"
            )
        if what == "sessions":
            # The app names a conversation by its start on this Mac's clock;
            # the test finds it in the list by that same name.
            for row in found["items"]:
                start = row.get("created_at_ms")
                if isinstance(start, int):
                    row["app_title"] = session_title(
                        self.system.local_time(start / 1000),
                        self.system.local_time(time.time()),
                    )
        return found

    def say(self, part: str) -> dict:
        number, lines_of = PARTS[part]
        lines = script_parts(script_path().read_text(encoding="utf-8"))[number][lines_of]
        selected = required_voices(self.system.run_checked("say", "-v", "?"))
        assigned = {
            "A": selected[0],
            "B": selected[1] if len(selected) > 1 else selected[0],
        }
        start = self.system.now()
        for voice, said in lines:
            self.system.run_checked("say", "-v", assigned[voice], "-a", BLACKHOLE, said)
        return {"lines": len(lines), "seconds": round(self.system.now() - start, 3)}

    def command(self, name: str) -> dict:
        """One of the test's commands to the daemon; a refusal is said with its
        cause."""
        if name not in COMMANDS:
            raise MacFailure(f"unknown command «{name}»")
        reply = self.orbit.call(COMMANDS[name])
        if not reply.get("ok"):
            cause = reply.get("refusal") or reply.get("error")
            raise MacFailure(
                f"the daemon refused «{name}»: {json.dumps(cause, ensure_ascii=False)[:300]}"
            )
        return {"command": name}

    def listening_choice(self) -> dict:
        """The same public settings order as Settings > AI, on this test hub."""
        choice = {"provider_id": "hub", "model": "whisper"}
        reply = self.orbit.call(
            {
                "command": "agent",
                "call": {
                    "call": "ai_function_save",
                    "function": "listening",
                    "choice": choice,
                },
            }
        )
        outcome = reply.get("outcome") or {}
        functions = (outcome.get("settings") or {}).get("functions", [])
        selected = next((row for row in functions if row.get("id") == "listening"), {})
        selected = {key: selected.get(key) for key in choice}
        if outcome.get("outcome") != "ai_settings" or selected != choice:
            raise MacFailure(
                f"hub listening choice not confirmed: expected {choice}, "
                f"selected {selected}, outcome {outcome.get('outcome')}, "
                f"refusal {outcome.get('refusal')}"
            )
        return choice

    def voice_note(self) -> dict:
        """A voice note, as a person records one: start it, say the phrase into
        BlackHole, end it. Ended even when the phrase could not be said."""
        selected = required_voices(self.system.run_checked("say", "-v", "?"))
        self.command("voice_start")
        start = self.system.now()
        try:
            self.system.pause(BEAT_BEFORE_SPEAKING_S)
            self.system.run_checked("say", "-v", selected[0], "-a", BLACKHOLE, voice_note_phrase())
            self.system.pause(BEAT_AFTER_SPEAKING_S)
        finally:
            self.command("voice_end")
        return {
            "phrase": voice_note_phrase(),
            "voice": selected[0],
            "seconds": round(self.system.now() - start, 3),
        }

    def restore(self, max_s: float) -> dict:
        # Every cleanup is attempted, especially restoration of the owner's
        # input, even if the fixture player or an earlier shutdown fails.
        try:
            remote = self.remote_audio("stop")
        finally:
            try:
                cua = self.engine_stop(max_s)
            finally:
                try:
                    closed = self._close_ours(max_s)
                finally:
                    previous = self._restore_input()
        return {
            "remote_audio": remote,
            "cua_driver": cua,
            "orbit_pruebas": closed,
            "restored_input": previous,
        }

    # ── CuaDriver: its manifest and its target (mac_engine.py runs it) ──

    def _app_pid(self) -> int:
        app = str(self.paths.app / MEMBERS[0])
        pids = [pid for pid, command in self.system.processes() if command == app]
        if len(pids) != 1:
            raise MacFailure(f"there are {len(pids)} processes of {app}, not one")
        return pids[0]

    def manifest(self, app_pid: int) -> str:
        """The capability manifest CuaDriver is started with: the read-only tool
        the accessibility check uses, on Orbit Pruebas only — its bundle identifier, its
        executable and the pid it runs with now — and nothing else: no other
        app, no display, no browser, no files, no processes to end
        (session_manifest.rs, cua-driver-rs-v0.28.2)."""
        executable = json.dumps(str(self.paths.app / MEMBERS[0]), ensure_ascii=False)
        tools = ", ".join(TOOLS)
        return (
            "# CuaDriver for Orbit's real test: Orbit Pruebas only, these tools only.\n"
            "version: 3\n"
            "# Existing validity ceiling; check 21 explicitly stops its short session.\n"
            f"expires_after: {VALIDITY}\n"
            "# Idle expiry is a fallback, never a substitute for the strict stop.\n"
            f"idle_timeout: {IDLE}\n"
            "resources:\n"
            "  apps:\n"
            f"    - bundle_id: {BUNDLE_ID}\n"
            "      windows: all\n"
            "      terminate: deny\n"
            f"    - executable: {executable}\n"
            "      windows: all\n"
            "      terminate: deny\n"
            "  desktop:\n"
            f"    applications: [{app_pid}]\n"
            "allow:\n"
            f"  tools: [{tools}]\n"
        )

    def switch_input(self, name=None) -> dict:
        return audio_route.switch_input(self, name)

    def activate(self) -> dict:
        """Brings Orbit Pruebas to the front and to the current Space.

        On the real Mac (run 8) its window opened on another Space, where
        CuaDriver resolves no accessibility window and returns an empty tree
        on purpose. `open` on a running app activates it, which moves the
        view to its window."""
        self.system.run_checked("open", str(self.paths.app))
        return {"activated": True}

    def logs(self) -> dict:
        texts = {}
        for name in (
            "report-daemon.log.1",
            "report-daemon.log",
            "report-app.log.1",
            "report-app.log",
        ):
            path = self.paths.logs / name
            if path.exists():
                texts[name] = path.read_text(encoding="utf-8", errors="replace")
        # The data directory is cleared before every run (prepare), so
        # these lines are this run's.
        call = []
        for name in ("daemon.log.1", "daemon.log"):
            path = self.paths.logs / name
            if not path.exists():
                continue
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                if isinstance(row, dict) and row.get("event") in CALL_EVENTS:
                    call.append(json.dumps(row, ensure_ascii=False) + "\n")
        return {"logs": texts, "call_audio": "".join(call)}


def main(argv: list[str], mac: Mac | None = None) -> int:
    from mac_cli import main as dispatch

    return dispatch(argv, mac or Mac(Paths(Path.home())))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
