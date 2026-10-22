"""The Mac side of the real test: what runs on the test Mac, over SSH.

The orchestrator on the test server copies this file and the meeting script to
`~/.orbit-pruebas/harness/` on the Mac and calls one subcommand at a time; each
prints one JSON object and exits 0, or prints `{"error": …}` and exits 1.

    prepare --max S           checks, BlackHole as the default input, clean data, opens Orbit Pruebas
    pair URL CODE NAME
    wait WHAT [ARG] --max S   waits on a fact Orbit Pruebas publishes or answers on its socket
    read detail ID | read sessions
    say PART                  says one part of the script into BlackHole
    voice-note                a voice note to Deimos: voice_start, one phrase into BlackHole, voice_end
    command call_start|call_stop  starts or stops a call with Deimos
    restore --max S           closes Orbit Pruebas and puts the previous input back
    logs                      the report logs of Orbit Pruebas and the call's audio lines of daemon.log
    activate                  brings Orbit Pruebas to the front
    floating                  Orbit Pruebas's windows above the normal layer (the floating panel)
    engine-start --max S      starts CuaDriver, bounded to Orbit Pruebas, under its own identity
    engine-call TOOL JSON     one CuaDriver tool call
    engine-stop --max S       stops the CuaDriver this test started, by its pid

The clicks and the readings of the interface go through CuaDriver, the
standalone computer-use driver (cua-driver 0.28.2, /Applications/CuaDriver.app)
with its own macOS permissions, used since Orbit Pruebas's own engine refused
the test's readings in run 5. The test starts CuaDriver bounded by a capability
manifest that allows five tools on Orbit Pruebas alone, targets only Orbit
Pruebas's exact pid, and stops it.

It never touches the user's Orbit, its data or its Keychain: Orbit Pruebas has
its own bundle, data directory and Keychain service (orbit_bundle, feature
`pruebas`). It stops only the processes of the Orbit Pruebas bundle, found by
their exact executable path, never by name.

Runs with the Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import datetime
import json
import os
import plistlib
import re
import select
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path

BUNDLE_ID = "app.orbit.companion.pruebas"
# The last part of orbit_bundle::DATA_DIRECTORY with the feature `pruebas`
# (directories::ProjectDirs on macOS). contracts/constants.json registers it
# (PRUEBAS_DATA_DIRECTORY): its guard compares this copy, and orbit-core's
# pruebas test the Rust one.
DATA_DIRECTORY = "com.MarsGotta.OrbitPruebas"
SCRIPT = Path(__file__).resolve().with_name("meeting-script.txt")
# Lists a process's windows on every layer (CuaDriver lists layer 0 only).
WINDOWS = Path(__file__).resolve().with_name("windows.swift")
BLACKHOLE = "BlackHole 2ch"
# What CuaDriver may do for the test, and nothing more: find Orbit Pruebas's
# windows, read them, click, type and choose a value.
TOOLS = ("list_windows", "get_window_state", "click", "type_text", "press_key")
# Where the official installer puts CuaDriver's daemon; a pid is the test's
# only if its executable is in there.
CUA_APP = "/Applications/CuaDriver.app/"
VALIDITY = "3h"
IDLE = "1h"
# How CuaDriver words a refusal on permissions (authorization.rs,
# session_manifest.rs, the macOS permission gate).
PERMISSION_REFUSALS = (
    "Permission denied",
    "outside the capability manifest",
    "capability manifest denies",
    "permission",
)


class Paths:
    """Where everything lives, from the home directory (a temporary one in the dry run)."""

    def __init__(self, home: Path):
        self.app = home / "Applications" / "Orbit Pruebas.app"
        self.data = home / "Library" / "Application Support" / DATA_DIRECTORY
        self.socket = self.data / "orbit.sock"
        self.logs = self.data / "logs"
        self.own = home / ".orbit-pruebas"
        # File name kept from the earlier, Spanish harness: a run of it that
        # died left the input to put back here, and the next run restores it.
        self.previous_input = self.own / "entrada-anterior"
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
# Between two attempts to reach a socket that does not exist yet. Only
# spaces the attempts; the bound is the caller's.
SOCKET_RETRY_S = 0.2
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
DEFAULT_OWNER_NAME = "Yo"
MAX_PERSON_NAME = 128
# What the voice note says. Not a line of meeting-script.txt: that is the
# meeting, and check 1 counts every one of its words in the meeting's
# transcript. Deimos is asked for one word so the answer is short.
VOICE_NOTE = "Deimos, contesta sólo con la palabra recibido."
# The beats around the phrase. Not waits on a fact: no topic says when the
# microphone delivers its first frame after voice_start (voice_active turns
# true when the order is taken, before the helper opens the stream) or when
# the last frame said has reached the daemon before voice_end. These are the
# values of the probe that worked on the Mac on 2026-10-17, unmeasured
# beyond it. A beat too short shows as words missing from the transcript,
# which check 11 names; it never makes the check pass.
BEAT_BEFORE_SPEAKING_S = 0.5
BEAT_AFTER_SPEAKING_S = 0.7
# A call's length when it starts (`call_start`, 1..60 minutes). Two minutes
# cover the wait for «listening» (30 s, real_test.py bounds) plus the SSH margin
# of that wait and of the `call_stop` after it (SSH_MARGIN_S, 30 s each);
# `call_stop` ends it well before, and a call that outlived the step would
# end by itself, not stay open.
CALL_MINUTES = 2
# The commands the test sends besides `pair`, and their bodies
# (orbit_core::protocol::Command).
COMMANDS = {
    "voice_start": {"command": "voice_start"},
    "voice_end": {"command": "voice_end"},
    "call_start": {"command": "call_start", "duration_minutes": CALL_MINUTES},
    "call_stop": {"command": "call_stop"},
}
# The daemon.log events the test brings back (call.rs): the call's
# microphone as it opened and what it heard. Only numbers; daemon.log's
# other lines may name paths on the Mac and stay there.
CALL_EVENTS = ("call_audio_opened", "call_audio_heard")
# The months as the app's list of conversations writes them (es-ES, short
# month, as ICU does: «sept»; session-title.ts): in code, so the Mac's
# locale cannot change them.
MONTHS = (
    "ene",
    "feb",
    "mar",
    "abr",
    "may",
    "jun",
    "jul",
    "ago",
    "sept",
    "oct",
    "nov",
    "dic",
)


class MacFailure(Exception):
    """A fact that ends a wait or a step, with its cause."""


# ── The system: commands and processes ─────────────────────────────────────


class System:
    def run(self, *command: str) -> tuple[int, str, str]:
        """Runs a command to its end: exit code, stdout and stderr."""
        done = subprocess.run(list(command), capture_output=True, text=True, check=False)
        return done.returncode, done.stdout, done.stderr

    def run_checked(self, *command: str) -> str:
        code, output, errors = self.run(*command)
        if code != 0:
            raise MacFailure(f"«{command[0]}» exited with {code}: {errors.strip()[:300]}")
        return output

    def has_command(self, name: str) -> bool:
        return shutil.which(name) is not None

    def processes(self) -> list[tuple[int, str]]:
        output = self.run_checked("ps", "-axo", "pid=,comm=")
        rows = []
        for line in output.splitlines():
            parts = line.strip().split(None, 1)
            if len(parts) == 2 and parts[0].isdigit():
                rows.append((int(parts[0]), parts[1]))
        return rows

    def signal(self, pid: int, number: int) -> None:
        try:
            os.kill(pid, number)
        except ProcessLookupError:
            pass

    def wait_exit(self, pids: list[int], max_s: float) -> list[int]:
        """Waits on the exit of each process (kqueue NOTE_EXIT); returns the
        ones still alive at the bound."""
        queue = select.kqueue()
        alive = set()
        for pid in pids:
            event = select.kevent(
                pid,
                select.KQ_FILTER_PROC,
                select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                select.KQ_NOTE_EXIT,
            )
            try:
                queue.control([event], 0, 0)
                alive.add(pid)
            except ProcessLookupError:
                continue
        deadline = time.monotonic() + max_s
        while alive:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            for event in queue.control(None, len(alive), left):
                alive.discard(event.ident)
        queue.close()
        return sorted(alive)

    def now(self) -> float:
        return time.monotonic()

    def pause(self, seconds: float) -> None:
        time.sleep(seconds)

    def local_time(self, seconds: float) -> time.struct_time:
        """The Mac's local time, the clock the app names a conversation by."""
        return time.localtime(seconds)


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


def _value(known: dict | None):
    if known and known.get("state") == "present":
        return known.get("value")
    return None


# ── What each wait waits for ────────────────────────────────────────────────
#
# A predicate looks at the latest knowledge of each topic and returns the
# result when the fact is there, None to keep waiting, or raises MacFailure
# when what it sees means the fact will not come.


def paired(state: dict, _arg):
    value = _value(state.get("status")) or {}
    if value.get("hub") == "revoked":
        raise MacFailure("the test hub revoked this Mac")
    return {"hub": "online"} if value.get("hub") == "online" else None


def recording(state: dict, _arg):
    current = (_value(state.get("recording")) or {}).get("recording") or {}
    if current.get("phase") == "recording":
        return {k: current.get(k) for k in ("id", "title", "audio", "started_at_ms")}
    return None


# Causes that do not pass while the recording goes on.
_NOT_LISTENING = (
    "macos_version",
    "locale_unsupported",
    "unavailable",
    "failed",
    "helper_without_phrase",
    "no_microphone",
    "privacy_stopped",
)


def listening(state: dict, _arg):
    voice = (_value(state.get("recording")) or {}).get("voice_trigger") or {}
    if voice.get("state") == "listening":
        return {"voice_trigger": "listening"}
    why = (voice.get("why") or {}) if voice.get("state") == "not_listening" else {}
    if why.get("cause") in _NOT_LISTENING:
        raise MacFailure(f"«Oye Deimos» is not listening: {json.dumps(why, ensure_ascii=False)}")
    return None


def _turn(state: dict) -> dict:
    """The assistant's turn in progress, or the last one: what the user said
    and Deimos's answer; the conversation itself is the hub's."""
    return (_value(state.get("assistant")) or {}).get("turn") or {}


def answer(state: dict, _arg):
    """The voice question once Deimos answered it or it failed. Until then it
    says what it saw: no question was heard, the question's stage, or an
    answer that has not come whole."""
    question = (_value(state.get("recording")) or {}).get("question") or {}
    if question.get("origin") != "voice":
        state["seen"] = (
            "no voice question: «Oye Deimos» opened none"
            if not question
            else f"a question of origin «{question.get('origin')}», not voice"
        )
        return None
    if question.get("stage") not in ("answered", "failed"):
        state["seen"] = f"the voice question is at «{question.get('stage')}»"
        return None
    result = {k: question.get(k) for k in ("id", "question", "origin", "stage", "cause")}
    if question["stage"] == "failed":
        result["answer"] = ""
        return result
    # The answer of the turn whose request id is the question's.
    message = _turn(state).get("answer") or {}
    if message.get("id") != f"{question.get('id')}:assistant" or not message.get("complete"):
        state["seen"] = (
            "the question is answered and its answer has not arrived whole yet "
            f"(turn {message.get('id')}, complete: {bool(message.get('complete'))})"
        )
        return None
    result["answer"] = message.get("text") or ""
    return result


# The note states after which nothing more comes without the user
# (orbit_core::notes::NoteState): written, given up, or no model on the hub.
FINAL_NOTE = ("ready", "failed", "not_configured")


def _row(state: dict, ident: str) -> dict | None:
    """The meeting `ident` in the latest read of the list of Notes
    (`notes_page`). A list the hub could not give is noted as what was seen,
    and the wait goes on: the hub may come back."""
    read = state.get("read") or {}
    if "refusal" in read:
        state["seen"] = f"the list of Notes could not be read: {json.dumps(read['refusal'])}"
        return None
    seen = f"recording {ident} is not in the list of Notes"
    for item in read.get("items") or []:
        if "hub_refusal" in item:
            seen = f"the hub did not give its part of the list: {json.dumps(item['hub_refusal'])}"
        elif item.get("recording_id") == ident:
            return item
    state["seen"] = seen
    return None


def saved(state: dict, ident):
    current = (_value(state.get("recording")) or {}).get("recording") or {}
    if current.get("id") == ident and current.get("phase") not in (None, "idle"):
        if current.get("phase") == "interrupted":
            raise MacFailure("the recording ended interrupted")
        state["seen"] = f"the recording is still at «{current.get('phase')}»"
        return None
    row = _row(state, ident)
    if row is None:
        return None
    local = row.get("local") or {}
    if row.get("capture") != "closed" or local.get("phase") == "recording":
        state["seen"] = (
            f"the list of Notes has it open (capture «{row.get('capture')}», "
            f"local «{local.get('phase')}»)"
        )
        return None
    return {k: row.get(k) for k in ("recording_id", "title", "duration_ms", "capture")}


def note(state: dict, ident):
    row = _row(state, ident)
    if row is None:
        return None
    headline = row.get("note") or {}
    if headline.get("state") in FINAL_NOTE:
        return {
            "phase": headline["state"],
            "short_summary": headline.get("short_summary"),
            "recording_id": row.get("recording_id"),
            "title": row.get("title"),
        }
    state["seen"] = f"transcript «{row.get('transcript')}», note «{headline.get('state', 'none')}»"
    return None


def chat(state: dict, text):
    assistant = _value(state.get("assistant")) or {}
    turn = assistant.get("turn") or {}
    user = turn.get("user") or {}
    if user.get("role") != "user" or user.get("text") != text:
        return None
    reply = turn.get("answer") or {}
    if assistant.get("turn_in_flight") or not reply.get("complete"):
        state["seen"] = "the turn is still in progress"
        return None
    return {"session": assistant.get("session_id"), "answer": reply.get("text") or ""}


def session(state: dict, ident):
    current = (_value(state.get("assistant")) or {}).get("session_id")
    state["seen"] = f"the open conversation is {current}"
    return {"session": current} if current == ident else None


def voice_note(state: dict, _arg):
    """The voice note's turn once Deimos answered it, or once a fact says no
    answer will come: its transcription failed, or the turn ended without a
    complete answer. The turn with a voice note is the test's: the turn
    before it is the Assistant's typed message, which has none."""
    assistant = _value(state.get("assistant")) or {}
    turn = assistant.get("turn") or {}
    user = turn.get("user") or {}
    voice = user.get("voice")
    if not isinstance(voice, dict):
        state["seen"] = "the turn in progress is not a voice note"
        return None
    reply = turn.get("answer") or {}
    result = {
        "role": user.get("role"),
        "transcript": voice.get("transcript"),
        "duration_ms": voice.get("duration_ms"),
        "audio_available": voice.get("audio_available", False),
        "failure": voice.get("failure"),
        "answer": reply.get("text") or "",
        "complete": bool(reply.get("complete")),
    }
    if result["failure"] is not None:
        return result
    if result["transcript"] is None:
        state["seen"] = "the voice note is being transcribed"
        return None
    if assistant.get("turn_in_flight"):
        state["seen"] = "Deimos has not finished answering"
        return None
    return result


def call_listening(state: dict, _arg):
    """The call once it listens: the daemon says `listening` only after the
    helper confirmed the microphone open with voice processing (CallReady).
    A call seen and then gone ended before it listened."""
    known = (_value(state.get("assistant")) or {}).get("call") or {}
    if known.get("state") != "present":
        state["seen"] = f"the call is «{known.get('state')}»"
        return None
    call = known.get("value")
    if call is None:
        if state.get("call_phase") is not None:
            raise MacFailure(
                f"the call ended without reaching «listening» (last phase «{state['call_phase']}»)"
            )
        state["seen"] = "there is no call"
        return None
    state["call_phase"] = call.get("phase")
    if call.get("phase") == "listening":
        return {k: call.get(k) for k in ("id", "phase", "started_at_ms", "expires_at_ms")}
    state["seen"] = f"the call is still at «{call.get('phase')}»"
    return None


def _notes_list(_arg) -> dict:
    return {"kind": "notes_page", "cursor": None}


# What each wait subscribes to, its predicate, and, for a fact only a read
# says, the read it repeats (on a change of the `notes` head and at least
# every READ_INTERVAL_S).
WAITS = {
    "paired": (("status",), paired, None),
    "recording": (("recording",), recording, None),
    "listening": (("recording",), listening, None),
    "answer": (("recording", "assistant"), answer, None),
    "saved": (("recording", "notes"), saved, _notes_list),
    "note": (("notes",), note, _notes_list),
    "chat": (("assistant",), chat, None),
    "session": (("assistant",), session, None),
    "voice-note": (("assistant",), voice_note, None),
    "call-listening": (("assistant",), call_listening, None),
}


def session_title(moment: time.struct_time, today: time.struct_time) -> str:
    """When a conversation began, as the app's list writes it from its start
    (session-title.ts `sessionWhen`), on the Mac's clock: «Hoy, 09:53»,
    «Ayer, 16:00», «18 oct, 18:02», «15 oct 2025, 03:46»."""
    clock = f"{moment.tm_hour:02d}:{moment.tm_min:02d}"
    day = datetime.date(moment.tm_year, moment.tm_mon, moment.tm_mday)
    now = datetime.date(today.tm_year, today.tm_mon, today.tm_mday)
    if day == now:
        return f"Hoy, {clock}"
    if day == now - datetime.timedelta(days=1):
        return f"Ayer, {clock}"
    year = "" if day.year == now.year else f" {day.year}"
    return f"{day.day} {MONTHS[day.month - 1]}{year}, {clock}"


def owner_name(full: str | None) -> str:
    """How the daemon names the user's microphone from the account's full name
    (orbit_core::notes::owner_name, fed by `id -F`)."""
    name = (full or "").split(",")[0].strip()
    if (
        not name
        or len(name) > MAX_PERSON_NAME
        or any(ord(c) < 32 or 127 <= ord(c) < 160 for c in name)
    ):
        return DEFAULT_OWNER_NAME
    return name


# ── The meeting script ──────────────────────────────────────────────────────


def script_parts(text: str) -> list[list[tuple[str, str]]]:
    """The three parts, each a list of (voice, line); «---» separates them."""
    parts: list[list[tuple[str, str]]] = [[]]
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line == "---":
            parts.append([])
            continue
        voice, separator, said = line.partition(":")
        if not separator or voice.strip() not in ("A", "B") or not said.strip():
            raise MacFailure(f"script line without «A:» or «B:»: {line[:60]}")
        parts[-1].append((voice.strip(), said.strip()))
    if len(parts) != 3 or not all(parts):
        raise MacFailure(
            f"the script must have three parts separated by «---» and has {len(parts)}"
        )
    return parts


PARTS = {"part1": 0, "question": 1, "part2": 2}


def spanish_voices(listing: str) -> list[str]:
    """Spanish voices from `say -v '?'`, Spain first: «Mónica  es_ES  # Hola…»."""
    found = []
    for line in listing.splitlines():
        match = re.match(r"^(?P<name>.+?)\s+(?P<language>[a-z]{2}_[A-Z]{2})\s+#", line)
        if match and match["language"].startswith("es_"):
            found.append((match["language"] != "es_ES", match["name"].strip()))
    return [name for _, name in sorted(found, key=lambda pair: pair[0])]


# ── The steps ───────────────────────────────────────────────────────────────


class Mac:
    def __init__(self, paths: Paths, system: System | None = None, orbit: OrbitIpc | None = None):
        self.paths = paths
        self.system = system or System()
        self.orbit = orbit or OrbitIpc(paths.socket)

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
        return {"closed": len(pids), "forced": len(left)}

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
        voices = spanish_voices(self.system.run_checked("say", "-v", "?"))
        if not voices:
            raise MacFailure("macOS has no Spanish voice for «say»")
        script_parts(SCRIPT.read_text(encoding="utf-8"))
        # What a run that died left running: its CuaDriver, its Orbit Pruebas.
        self.engine_stop(max_s)
        before = self._close_ours(max_s)
        # A run that could not restore the input left the original here.
        self._restore_input()
        paths.own.mkdir(mode=0o700, exist_ok=True)
        current = self.system.run_checked("SwitchAudioSource", "-c", "-t", "input").strip()
        paths.previous_input.write_text(current + "\n", encoding="utf-8")
        self.system.run_checked("SwitchAudioSource", "-t", "input", "-s", BLACKHOLE)
        if paths.data.exists():
            shutil.rmtree(paths.data)
        self.system.run_checked("open", str(paths.app))
        start = self.system.now()
        while not self.orbit.answers():
            if self.system.now() - start > max_s:
                raise MacFailure(
                    f"Orbit Pruebas's daemon did not open its socket within {max_s:.0f} s"
                )
            time.sleep(SOCKET_RETRY_S)
        macos = self.system.run_checked("sw_vers", "-productVersion").strip()
        # The name the microphone's turns must carry, taken where the daemon
        # takes it, never from the hub, which only repeats what the Mac sent.
        code, full, _ = self.system.run("id", "-F")
        return {
            "voices": voices[:2],
            "previous_input": current,
            "macos": macos,
            "closed_before": before,
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
                    return {"value": found, "seconds": round(self.system.now() - start, 3)}
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
                        self.system.local_time(start / 1000), self.system.local_time(time.time())
                    )
        return found

    def say(self, part: str) -> dict:
        lines = script_parts(SCRIPT.read_text(encoding="utf-8"))[PARTS[part]]
        voices = spanish_voices(self.system.run_checked("say", "-v", "?"))
        if not voices:
            raise MacFailure("macOS has no Spanish voice for «say»")
        assigned = {"A": voices[0], "B": voices[1] if len(voices) > 1 else voices[0]}
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

    def voice_note(self) -> dict:
        """A voice note, as a person records one: start it, say the phrase into
        BlackHole, end it. Ended even when the phrase could not be said."""
        voices = spanish_voices(self.system.run_checked("say", "-v", "?"))
        if not voices:
            raise MacFailure("macOS has no Spanish voice for «say»")
        self.command("voice_start")
        start = self.system.now()
        try:
            self.system.pause(BEAT_BEFORE_SPEAKING_S)
            self.system.run_checked("say", "-v", voices[0], "-a", BLACKHOLE, VOICE_NOTE)
            self.system.pause(BEAT_AFTER_SPEAKING_S)
        finally:
            self.command("voice_end")
        return {
            "phrase": VOICE_NOTE,
            "voice": voices[0],
            "seconds": round(self.system.now() - start, 3),
        }

    def restore(self, max_s: float) -> dict:
        cua = self.engine_stop(max_s)
        closed = self._close_ours(max_s)
        return {
            "cua_driver": cua,
            "orbit_pruebas": closed,
            "restored_input": self._restore_input(),
        }

    # ── CuaDriver ──

    def _app_pid(self) -> int:
        app = str(self.paths.app / MEMBERS[0])
        pids = [pid for pid, command in self.system.processes() if command == app]
        if len(pids) != 1:
            raise MacFailure(f"there are {len(pids)} processes of {app}, not one")
        return pids[0]

    def _cua_status(self) -> str | None:
        """`cua-driver status`, or None when no CuaDriver daemon is running."""
        code, output, _ = self.system.run(str(self.paths.cua), "status")
        return output if code == 0 else None

    def manifest(self, app_pid: int) -> str:
        """The capability manifest CuaDriver is started with: the five tools
        the test uses, on Orbit Pruebas only — its bundle identifier, its
        executable and the pid it runs with now — and nothing else: no other
        app, no display, no browser, no files, no processes to end
        (session_manifest.rs, cua-driver-rs-v0.28.2)."""
        executable = json.dumps(str(self.paths.app / MEMBERS[0]), ensure_ascii=False)
        tools = ", ".join(TOOLS)
        return (
            "# CuaDriver for Orbit's real test: Orbit Pruebas only, these tools only.\n"
            "version: 3\n"
            "# The longest run is under three hours (the note waits up to 52 min).\n"
            f"expires_after: {VALIDITY}\n"
            "# The longest silence between two calls is the wait for the note.\n"
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

    def activate(self) -> dict:
        """Brings Orbit Pruebas to the front and to the current Space.

        On the real Mac (run 8) its window opened on another Space, where
        CuaDriver resolves no accessibility window and returns an empty tree
        on purpose. `open` on a running app activates it, which moves the
        view to its window."""
        self.system.run_checked("open", str(self.paths.app))
        return {"activated": True}

    def floating(self) -> dict:
        """Orbit Pruebas's windows on screen above the normal layer: the
        floating panel. CuaDriver's `list_windows` lists layer-0 windows only
        (cua-driver 0.28.2, list_windows.rs), so the panel, always on top,
        never appears there; WindowServer lists it (windows.swift)."""
        app_pid = self._app_pid()
        rows = [
            json.loads(line)
            for line in self.system.run_checked("swift", str(WINDOWS), str(app_pid)).splitlines()
            if line.strip()
        ]
        return {
            "windows": [
                {"pid": app_pid, **row}
                for row in rows
                if row.get("layer", 0) > 0 and row.get("is_on_screen")
            ]
        }

    def engine_start(self, max_s: float) -> dict:
        """Starts CuaDriver's daemon under its own macOS identity (LaunchServices,
        so its own Accessibility and Screen Recording grants apply), bounded by
        the manifest, and checks that it took it."""
        if self._cua_status() is not None:
            raise MacFailure(
                "a CuaDriver is already running; the test does not touch it: stop it with "
                "«cua-driver stop» and launch the test again"
            )
        app_pid = self._app_pid()
        self.paths.own.mkdir(mode=0o700, exist_ok=True)
        descriptor = os.open(self.paths.manifest, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as file:
            file.write(self.manifest(app_pid))
        self.system.run_checked(
            "open", "-n", "-g", "-a", "CuaDriver", "--args", "serve",
            "--permission-mode", "bounded",
            "--capability-manifest", str(self.paths.manifest),
            "--approve-capability-manifest",
        )  # fmt: skip
        start = self.system.now()
        while True:
            status = self._cua_status()
            pid = re.search(r"^\s*pid: (\d+)$", status or "", re.MULTILINE)
            if pid is not None:
                break
            if self.system.now() - start > max_s:
                raise MacFailure(f"CuaDriver did not answer within {max_s:.0f} s of starting it")
            time.sleep(SOCKET_RETRY_S)
        self.paths.cua_pid.write_text(pid.group(1) + "\n", encoding="utf-8")
        mode = re.search(r"permission mode: (\w+)", status or "")
        manifest = "capability manifest: configured=true, approved_at_startup=true, valid=true"
        if mode is None or mode.group(1) != "bounded" or manifest not in (status or ""):
            raise MacFailure(
                "CuaDriver did not start in bounded mode with its manifest: "
                + " | ".join(line.strip() for line in (status or "").splitlines()[1:])
            )
        return {"pid": int(pid.group(1)), "app_pid": app_pid}

    def engine_call(self, tool: str, arguments: dict) -> dict:
        """One tool call. A refusal is said by its name: the driver refusing its
        own authorization process, or a permission the manifest or macOS does
        not give, never a bare failure."""
        code, output, errors = self.system.run(
            str(self.paths.cua), "call", tool, json.dumps(arguments, ensure_ascii=False)
        )
        if code != 0:
            cause = errors.strip()[:500]
            if "own authorization process" in cause:
                raise MacFailure(
                    f"CuaDriver refuses to act on its own authorization process "
                    f"(pid {arguments.get('pid')}) in {tool}: {cause}"
                )
            if any(mark in cause for mark in PERMISSION_REFUSALS):
                raise MacFailure(f"CuaDriver refused {tool} for permissions: {cause}")
            raise MacFailure(f"CuaDriver failed in {tool} (code {code}): {cause}")
        try:
            result = json.loads(output)
        except ValueError:
            result = {"text": output.strip()}
        return {"result": result, "bytes": len(output.encode())}

    def engine_stop(self, max_s: float) -> dict:
        """Stops the CuaDriver this test started: by the pid it wrote down,
        and only if that pid is still CuaDriver's."""
        if not self.paths.cua_pid.exists():
            return {"stopped": True, "pid": None}
        pid = int(self.paths.cua_pid.read_text(encoding="utf-8").strip())
        ours = any(
            p == pid and command.startswith(CUA_APP) for p, command in self.system.processes()
        )
        if ours:
            self.system.run(str(self.paths.cua), "--expected-pid", str(pid), "stop")
            if self.system.wait_exit([pid], max_s):
                self.system.signal(pid, signal.SIGTERM)
        self.paths.cua_pid.unlink()
        return {"stopped": True, "pid": pid}

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


def _bound(arguments: list[str]) -> tuple[list[str], float]:
    if "--max" not in arguments:
        raise MacFailure("--max is missing")
    position = arguments.index("--max")
    return arguments[:position] + arguments[position + 2 :], float(arguments[position + 1])


def main(argv: list[str], mac: Mac | None = None) -> int:
    mac = mac or Mac(Paths(Path.home()))
    try:
        if not argv:
            raise MacFailure("the subcommand is missing")
        command, rest = argv[0], argv[1:]
        if command == "prepare":
            _, max_s = _bound(rest)
            output = mac.prepare(max_s)
        elif command == "pair" and len(rest) == 3:
            output = mac.pair(*rest)
        elif command == "wait":
            rest, max_s = _bound(rest)
            output = mac.wait(rest[0], rest[1] if len(rest) > 1 else None, max_s)
        elif command == "read" and rest:
            output = mac.read(rest[0], rest[1] if len(rest) > 1 else None)
        elif command == "say" and len(rest) == 1 and rest[0] in PARTS:
            output = mac.say(rest[0])
        elif command == "voice-note" and not rest:
            output = mac.voice_note()
        elif command == "command" and len(rest) == 1 and rest[0] in ("call_start", "call_stop"):
            output = mac.command(rest[0])
        elif command == "restore":
            _, max_s = _bound(rest)
            output = mac.restore(max_s)
        elif command == "logs":
            output = mac.logs()
        elif command == "activate":
            output = mac.activate()
        elif command == "floating":
            output = mac.floating()
        elif command == "engine-start":
            _, max_s = _bound(rest)
            output = mac.engine_start(max_s)
        elif command == "engine-call" and len(rest) == 2:
            output = mac.engine_call(rest[0], json.loads(rest[1]))
        elif command == "engine-stop":
            _, max_s = _bound(rest)
            output = mac.engine_stop(max_s)
        else:
            raise MacFailure(f"unknown subcommand: {' '.join(argv)[:80]}")
    except (MacFailure, OSError, ValueError, KeyError) as cause:
        print(json.dumps({"error": str(cause) or type(cause).__name__}, ensure_ascii=False))
        return 1
    print(json.dumps(output, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
