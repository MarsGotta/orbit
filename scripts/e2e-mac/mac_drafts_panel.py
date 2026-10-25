"""The Mac side of checks 17 and 18 of the real test: a draft typed in the
Assistant's composer and Orbit Pruebas quit right after it, with ⌘Q or with
the Dock's quit (the Apple Event «quit»), then opened again; and the system's
pointer moved over the floating panel and away from it, with the daemon's
word on the answer it holds.

`DraftsAndPanel` is mixed into mac.py's `Mac`: its methods use the Mac's
paths, system, socket and waits (`wait`), its CuaDriver calls
(`engine_call`) and its processes (`_ours`, `_app_pid`). Copied to the Mac
with mac.py; runs with the Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import json
import signal
import sqlite3
import time
from pathlib import Path

from mac_controls import MacFailure

# Lists a process's windows on every layer (CuaDriver lists layer 0 only).
WINDOWS = Path(__file__).resolve().with_name("windows.swift")
# Moves the system's pointer over a window, or beside it.
POINTER = Path(__file__).resolve().with_name("pointer.swift")
# How long a closed Orbit Pruebas is watched for a relaunch: the app's own
# relaunch, after its daemon ended, takes about a second.
RELAUNCH_WATCH_S = 5
# Between two attempts to reach a socket that does not exist yet, or two
# reads of the composer while a typed draft reaches it. Only spaces the
# attempts; the bound is the caller's.
SOCKET_RETRY_S = 0.2
# Every draft's key in the webview's local storage starts with `DRAFT_PREFIX`
# of state/local-drafts.ts; the composer's is its key (use-composer.ts). The
# app writes it on every change, never on a timer; the dry-run test checks both
# names against those files.
DRAFT_PREFIX = "orbit.draft."
DRAFT_KEY = DRAFT_PREFIX + "chat"
# ⌘Q, as a person presses it, through CuaDriver to Orbit Pruebas's pid; and
# the Apple Event «quit», what the Dock's «Salir» sends.
QUITS = ("cmd-q", "apple-event")


def _stored_text(value) -> str:
    """A value of WebKit's ItemTable: UTF-16 in a BLOB."""
    if isinstance(value, bytes):
        return value.decode("utf-16-le", errors="replace")
    return str(value)


class DraftsAndPanel:
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

    def launch(self, max_s: float) -> dict:
        """Opens Orbit Pruebas and waits until its daemon answers on its socket."""
        self.system.run_checked("open", str(self.paths.app))
        start = self.system.now()
        while not self.orbit.answers():
            if self.system.now() - start > max_s:
                raise MacFailure(
                    f"Orbit Pruebas's daemon did not open its socket within {max_s:.0f} s"
                )
            time.sleep(SOCKET_RETRY_S)
        return {"seconds": round(self.system.now() - start, 3)}

    def _draft_in_box(self, typing: dict, screen_s: float) -> None:
        """Waits until the window shows the draft just typed: only a draft on
        screen went through the composer's change, which keeps it."""
        read = {
            "pid": typing["pid"],
            "window_id": typing["window_id"],
            "include_accessibility_tree": True,
            "include_screenshot": False,
            # The draft is this run's own: only the composer's row matches.
            "query": typing["text"],
        }
        start = self.system.now()
        while True:
            result = self.engine_call("get_window_state", read)["result"]
            tree = result.get("tree_markdown") if isinstance(result, dict) else None
            if typing["text"] in (tree or ""):
                return
            if self.system.now() - start >= screen_s:
                raise MacFailure(
                    f"the composer did not show the draft typed within {screen_s:.0f} s; "
                    "Orbit Pruebas was not quit"
                )
            time.sleep(SOCKET_RETRY_S)

    def _quit(self, how: str, typing: dict) -> None:
        if how == "cmd-q":
            keys = {"key": "q", "modifiers": ["cmd"], "delivery_mode": "foreground"}
            self.engine_call(
                "press_key", {"pid": typing["pid"], "window_id": typing["window_id"], **keys}
            )
            return
        # Addressed by bundle identifier (its WebKit directory's name) and
        # asked only while it runs: telling a closed app to quit would open
        # it (scripts/release/install-on-mac.sh does the same).
        bundle = self.paths.webkit.name
        self.system.run_checked(
            "osascript",
            "-e",
            f'if application id "{bundle}" is running then tell application id "{bundle}" to quit',
        )

    def draft_and_quit(self, how: str, typing: str, screen_s: float, close_s: float) -> dict:
        """Types a draft in the composer (cua-driver `type_text` arguments),
        reads it back on screen and quits Orbit Pruebas `how` at once, in this
        one call, so nothing but that read stands between the draft and the
        quit. Then waits for every process of the bundle to end, watches for a
        relaunch, stops whatever came back, and reads whether WebKit's own
        storage kept the draft."""
        if how not in QUITS:
            raise MacFailure(f"unknown quit «{how}»")
        arguments = json.loads(typing)
        self.engine_call("type_text", arguments)
        typed = self.system.now()
        self._draft_in_box(arguments, screen_s)
        pids = self._ours()
        quit_at = self.system.now()
        self._quit(how, arguments)
        left = self.system.wait_exit(pids, close_s)
        if left:
            raise MacFailure(
                f"{len(left)} of the {len(pids)} processes of Orbit Pruebas still ran "
                f"{close_s:.0f} s after the quit ({how})"
            )
        self.system.pause(RELAUNCH_WATCH_S)
        back = self._ours()
        for pid in back:
            self.system.signal(pid, signal.SIGKILL)
        return {
            "quit": how,
            "in_box": True,
            "gap_s": round(quit_at - typed, 3),
            "closed": len(pids),
            "came_back": len(back),
            "stored": self.web_draft(arguments["text"]),
        }

    def _web_storages(self) -> list[Path]:
        """WebKit's local storage files of Orbit Pruebas, one per origin."""
        if not self.paths.webkit.is_dir():
            return []
        return sorted(self.paths.webkit.rglob("localstorage.sqlite3"))

    def clear_web_drafts(self) -> int:
        """Deletes every draft (DRAFT_PREFIX) from WebKit's storage of Orbit
        Pruebas, with the app closed, and returns how many: a draft an earlier
        run left would be in the composer when the run types its own message.
        Nothing else goes: the storage also keeps the first run's completion
        (`orbit.onboarding.done.local`), and without it the app opens on its
        welcome (run 20261025T141530Z). A file it cannot clean stops the run
        by name, since a draft left in it would."""
        removed = 0
        for path in self._web_storages():
            try:
                connection = sqlite3.connect(str(path))
                try:
                    with connection:
                        cursor = connection.execute(
                            "DELETE FROM ItemTable WHERE substr(key, 1, ?) = ?",
                            (len(DRAFT_PREFIX), DRAFT_PREFIX),
                        )
                    removed += cursor.rowcount
                finally:
                    connection.close()
            except sqlite3.Error as cause:
                raise MacFailure(f"could not remove the drafts from {path}: {cause}") from cause
        return removed

    def web_draft(self, text: str) -> bool | None:
        """Whether WebKit's storage of Orbit Pruebas holds `text` in the
        composer's draft, read from its files with the app closed. None when
        there is no such file to read: unknown, never «no»."""
        read, held = False, False
        for path in self._web_storages():
            try:
                connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True)
                try:
                    rows = connection.execute(
                        "SELECT value FROM ItemTable WHERE key = ?", (DRAFT_KEY,)
                    ).fetchall()
                finally:
                    connection.close()
            except sqlite3.Error:
                continue
            read = True
            held = held or any(text in _stored_text(value) for (value,) in rows)
        return held if read else None

    def panel_pointer(self, window_id: str, where: str, question: str, max_s: float) -> dict:
        """Moves the system's pointer `over` the panel or `away` from it
        (pointer.swift), and waits for what the daemon says of the answer
        `question`: held while the pointer is over it, gone once it left."""
        if where not in ("over", "away"):
            raise MacFailure(f"the pointer goes «over» or «away», not «{where}»")
        output = self.system.run_checked("swift", str(POINTER), str(window_id), where)
        lines = [line for line in output.splitlines() if line.strip()]
        moved = json.loads(lines[-1]) if lines else {}
        if moved.get("inside") is not (where == "over"):
            raise MacFailure(
                f"after moving it {where}, the pointer is not where it was sent: "
                f"{json.dumps(moved)}"
            )
        done = self.wait("pointer-held" if where == "over" else "answer-released", question, max_s)
        return {"pointer": moved, **done}


def _question(state: dict) -> dict | None:
    known = state.get("recording") or {}
    value = known.get("value") if known.get("state") == "present" else None
    return (value or {}).get("question")


def pointer_held(state: dict, ident):
    """The answer `ident` once the daemon says the pointer holds it
    (`held`): the app read the pointer over the panel and told it."""
    question = _question(state)
    if not question or question.get("id") != ident:
        raise MacFailure(f"the answer {ident} left the panel before the pointer held it")
    if question.get("held"):
        return {"held": True}
    state["seen"] = (
        "the answer is not held: the app did not tell the daemon the pointer is over the panel"
    )
    return None


def answer_released(state: dict, ident):
    """The answer `ident` once the daemon let it go."""
    question = _question(state)
    if not question or question.get("id") != ident:
        return {"gone": ident}
    state["seen"] = (
        f"the answer is still shown (held: {bool(question.get('held'))}, "
        f"released at {question.get('released_at_ms')})"
    )
    return None


# The waits these steps add to mac.py's WAITS, in its shape.
PANEL_WAITS = {
    "pointer-held": (("recording",), pointer_held, None),
    "answer-released": (("recording",), answer_released, None),
}


def _arguments(rest: list[str], count: int) -> tuple[list[str], float]:
    """The subcommand's `count` arguments and the bound after `--max`."""
    if len(rest) != count + 2 or rest[-2] != "--max":
        raise MacFailure(f"expected {count} arguments and --max S: {' '.join(rest)[:80]}")
    return rest[:count], float(rest[-1])


def _floating(mac, rest: list[str]) -> dict:
    if rest:
        raise MacFailure("floating takes no arguments")
    return mac.floating()


def _launch(mac, rest: list[str]) -> dict:
    _, max_s = _arguments(rest, 0)
    return mac.launch(max_s)


def _draft_and_quit(mac, rest: list[str]) -> dict:
    (how, typing, screen_s), close_s = _arguments(rest, 3)
    return mac.draft_and_quit(how, typing, float(screen_s), close_s)


def _panel_pointer(mac, rest: list[str]) -> dict:
    (window_id, where, question), max_s = _arguments(rest, 3)
    return mac.panel_pointer(window_id, where, question, max_s)


# mac.py's subcommands of these steps, each with its own arguments:
#   floating
#   launch --max S
#   draft-and-quit cmd-q|apple-event TYPE_TEXT_JSON SCREEN_S --max S
#   panel-pointer WINDOW_ID over|away QUESTION_ID --max S
COMMANDS = {
    "floating": _floating,
    "launch": _launch,
    "draft-and-quit": _draft_and_quit,
    "panel-pointer": _panel_pointer,
}
