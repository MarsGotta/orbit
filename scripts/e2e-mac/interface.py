"""Orbit Pruebas's interface, clicked and read as a person would, through
CuaDriver.

CuaDriver is the standalone computer-use driver (cua-driver 0.28.2, the
version Orbit bundles), installed in /Applications/CuaDriver.app with its own
Accessibility and Screen Recording grants. The test drives it from the test server
over SSH (`mac.py engine-call`), bounded by a capability manifest to five
tools on Orbit Pruebas alone. Orbit Pruebas's own engine was the first design
(the test as Deimos through the hub's bridge); after run 5, where it refused
every reading as «operations that target its own authorization process», the
harness moved to CuaDriver. Only Orbit Pruebas's exact pid is ever targeted:
cua-driver refuses a pid that is its own.

The vocabulary is cua-driver's: `list_windows`, `get_window_state`, `click`,
and `type_text`, an element addressed by the `element_index` of
the latest tree of its window.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path

import ax_tree

BUNDLE_ID = "app.orbit.companion.pruebas"
PANEL_TITLE = "Orbit · llamada"
# How cua-driver says it stopped walking a tree (ax/tree.rs): at most 2 000
# nodes by default (DEFAULT_MAX_ELEMENTS). A tree cut short is never read as
# a whole one.
TRUNCATED = "AX tree truncated at"
# Attempts to choose an option in a native <select> before failing by name;
# each one is read back (real runs 23 and 24 needed a second).
CHOOSE_ATTEMPTS = 3
# The open menu is not in the tree (run 13): the keys wait this beat for
# it. Not a deadline: the read-back decides whether the choice took.
MENU_BEAT_S = 0.3


class StepFailed(Exception):
    """A step could not be done; the text says why, with numbers."""


@dataclass(frozen=True)
class Window:
    pid: int
    window_id: int
    title: str


class Interface:
    def __init__(self, bridge, artifacts: Path, app_pid: int):
        self.bridge = bridge
        self.snapshot_id: str | None = None
        self.artifacts = artifacts
        # The pid of Orbit Pruebas's app, found by its exact executable path:
        # only its windows are ever targeted.
        self.app_pid = app_pid
        self.reads: list[dict] = []
        self.trees: dict[str, str] = {}
        self._read_count = 0
        self.truncated: str | None = None

    def windows(self) -> dict[str, Window]:
        """Orbit Pruebas's main window («main»), among the windows of its own
        pid and of no other process. The floating panel is never here:
        CuaDriver lists layer-0 windows only (see `panel`)."""
        # Not `on_screen_only`: on the real Mac (run 7) the main window was
        # listed with is_on_screen false and on another Space, and the pid
        # filter already keeps the answer to this app's few windows.
        structure = self.bridge.action("list_windows", {"pid": self.app_pid})
        rows = [w for w in structure.get("windows", []) if w.get("pid") == self.app_pid]
        found: dict[str, Window] = {}
        for row in rows:
            title = row.get("title") or ""
            if title == "Orbit":
                found.setdefault("main", Window(int(row["pid"]), int(row["window_id"]), title))
        if "main" not in found:
            raise StepFailed(
                f"CuaDriver does not see Orbit Pruebas's window (pid {self.app_pid}): "
                f"{len(rows)} of its windows, none titled «Orbit»"
            )
        return found

    def panel(self, floating, bound_s: float) -> Window:
        """The floating panel («Orbit · llamada»), once it is on screen.

        CuaDriver's `list_windows` lists layer-0 windows only, and the panel
        is always on top: it was never listed, so runs 1 to 17 said there was
        no window while the panel was on screen (probed on the Mac on
        2026-10-17). `floating` asks WindowServer for the app's windows above
        layer 0 that are on screen; the one whose accessibility window is
        titled «Orbit · llamada» is the panel, read by its window id as any
        other window."""
        deadline = time.monotonic() + bound_s
        seen: list[int] = []
        while True:
            for row in floating().get("windows", []):
                if row.get("pid") != self.app_pid:
                    continue
                window = Window(self.app_pid, int(row["window_id"]), PANEL_TITLE)
                structure = self.bridge.action(
                    "get_window_state",
                    {
                        "pid": window.pid,
                        "window_id": window.window_id,
                        "include_accessibility_tree": True,
                        "include_screenshot": False,
                    },
                )
                first = (structure.get("tree_markdown") or "").lstrip().split("\n", 1)[0]
                if f'AXWindow "{PANEL_TITLE}"' in first:
                    return window
                seen.append(window.window_id)
            if time.monotonic() >= deadline:
                raise StepFailed(
                    f"with the answer given, no floating window of Orbit Pruebas is "
                    f"«{PANEL_TITLE}» within {bound_s:.0f} s (floating windows seen: "
                    f"{sorted(set(seen))})"
                )
            time.sleep(0.5)

    def read(self, window: Window, label: str, query: str | None) -> list[ax_tree.Node]:
        """The rows of one window that match `query`, and their ancestors.

        Every reading of the main window is filtered (cua-driver's `query`):
        a check reads only what it needs, the answer stays small, and fewer
        nodes leave the window. Only the call panel, a card of a handful of
        rows, is read whole (`query=None`). Each answer's size goes to the
        report. CuaDriver walks at most 2 000 nodes of a window; a reading it
        cut short fails by name.
        """
        arguments = {
            "pid": window.pid,
            "window_id": window.window_id,
            "include_accessibility_tree": True,
            "include_screenshot": False,
        }
        if query is not None:
            arguments["query"] = query
        structure = self.bridge.action("get_window_state", arguments)
        size = getattr(self.bridge, "last_reply_bytes", None)
        self.reads.append({"label": label, "query": query, "bytes": size})
        # A click by element_index needs the snapshot it came from
        # (cua-driver 0.28.2 click: «element_index … requires the matching
        # snapshot_id»); each read replaces the window's snapshot.
        self.snapshot_id = structure.get("snapshot_id")
        if structure.get("degraded"):
            # CuaDriver returns an empty tree on purpose when it cannot tie the
            # window to its accessibility element (another Space, for one).
            reason = str(structure.get("degraded_reason") or "")[:200]
            raise StepFailed(f"CuaDriver could not read «{label}»: {reason}")
        markdown = structure.get("tree_markdown")
        if not isinstance(markdown, str):
            raise StepFailed(f"the engine returned no tree for «{label}»: {sorted(structure)}")
        markdown = ax_tree.without_menu_bar(markdown)
        self._read_count += 1
        name = f"{self._read_count:03d}-{label.replace('/', '-').replace(' ', '-')}.txt"
        (self.artifacts / name).write_text(markdown, encoding="utf-8")
        cut = next((line for line in markdown.splitlines() if TRUNCATED in line), None)
        self.truncated = cut.strip()[:160] if cut is not None else None
        if not label.startswith("find "):
            # Evidence read short would be evidence of nothing. A search may
            # still find its target in what was walked (the indices stay valid).
            if self.truncated is not None:
                raise StepFailed(f"tree of {label} truncated: {self.truncated}")
            self.trees[label] = markdown
        return ax_tree.parse(markdown)

    def _one(
        self,
        window: Window,
        text: str,
        roles: tuple[str, ...],
        contains: bool,
        first: bool = False,
    ) -> ax_tree.Node:
        # A screen the previous click opened renders a moment later (the
        # route change, then its own read). Wait for the element instead of
        # failing on the first frame: 10 s is far above a render and is the
        # same bound as the interface showing a change (bounds «screen»).
        deadline = time.monotonic() + 10
        while True:
            nodes = self.read(window, f"find {text}", query=text)
            candidates = ax_tree.find(nodes, text, roles, contains)
            if candidates or self.truncated is not None or time.monotonic() >= deadline:
                break
            time.sleep(0.5)
        if not candidates and self.truncated is None:
            # cua-driver's `query` filters the rendered lines, so a label that
            # holds a line break («R» then «Reunión…», «+» then «Nueva
            # conversación») matches only on its second line and loses its
            # node (real run 16). One whole read of the window finds it.
            nodes = self.read(window, f"find {text} (whole window)", query=None)
            candidates = ax_tree.find(nodes, text, roles, contains)
        if not candidates and self.truncated is not None:
            raise StepFailed(f"«{text}» is not in the tree, truncated: {self.truncated}")
        if first and candidates:
            # The main view comes before the context panel in the tree.
            return candidates[0]
        if len(candidates) != 1:
            seen = [f"[{n.index}] {n.role} «{n.text()}»" for n in candidates[:5]]
            screen = ""
            if not candidates:
                # What the screen says instead, so the failure shows where the
                # app really is (Orbit Pruebas's own, synthetic content).
                texts = [
                    n.text()
                    for n in self.read(window, f"screen without {text}", "AXStaticText")
                    if n.text()
                ]
                screen = f"; the screen says {texts[:12]}"
            raise StepFailed(
                f"«{text}» ({', '.join(roles) or 'any role'}): {len(candidates)} pressable "
                f"elements instead of one {seen}{screen}"
            )
        return candidates[0]

    def press(
        self,
        window: Window,
        text: str,
        roles: tuple[str, ...] = (),
        contains: bool = False,
        button: str | None = None,
        first: bool = False,
    ) -> None:
        node = self._one(window, text, roles, contains, first)
        arguments = {
            "pid": window.pid,
            "window_id": window.window_id,
            "element_index": node.index,
            "snapshot_id": self.snapshot_id,
        }
        if button is not None:
            arguments["button"] = button
        self.bridge.action("click", arguments)

    def read_until(
        self, window: Window, label: str, query: str | None, condition, bound_s: float
    ) -> float:
        """Reads the window until `condition(text)` holds or the bound passes.

        The interface gets the daemon's changes pushed, but the engine reads
        the screen on its own schedule: waiting on what the screen shows,
        rather than reading once, keeps the result from depending on which of
        the two came first. The last reading is the evidence either way.
        Returns the seconds it took."""
        start = time.monotonic()
        while True:
            text = ax_tree.all_text(self.read(window, label, query))
            if condition(text) or time.monotonic() - start > bound_s:
                return time.monotonic() - start
            time.sleep(1)

    def type_into(self, window: Window, field: str, text: str, contains: bool = False) -> None:
        node = self._one(window, field, ("AXTextArea", "AXTextField"), contains)
        # In the foreground, as a person types: a background insert into web
        # content is never confirmed by the engine (cua-driver type_text).
        self.bridge.action(
            "type_text",
            {
                "pid": window.pid,
                "window_id": window.window_id,
                "element_index": node.index,
                "snapshot_id": self.snapshot_id,
                "text": text,
                "delivery_mode": "foreground",
            },
        )

    def choose(self, window: Window, field: str, option: str, options: tuple[str, ...]) -> None:
        """Chooses `option` in a native <select>, as a person does with the
        keyboard: open its menu, move to the option, press Return, and read
        the field back.

        Setting the popup's AXValue (set_value) changes what it shows but
        fires no change event in WebKit (real run 12), and the open menu is
        not part of the window's accessibility tree (run 13), so its items
        cannot be pressed. `options` is the menu's order, from the app.

        The menu opening cannot be observed, so the keys can reach the popup
        before its menu is up: right after a new build was installed (real
        runs 23 and 24) the field stayed on «Sistema». Each attempt reads the
        field back, and the choice is tried again, `CHOOSE_ATTEMPTS` times at
        most, before it fails by name."""
        current = ""
        for _ in range(CHOOSE_ATTEMPTS):
            node = self._one(window, field, ("AXPopUpButton", "AXComboBox"), False)
            current = node.value or ""
            if current == option:
                return
            if current not in options or option not in options:
                raise StepFailed(f"«{field}» shows «{current}», which is not in {list(options)}")
            steps = options.index(option) - options.index(current)
            self.bridge.action(
                "click",
                {
                    "pid": window.pid,
                    "window_id": window.window_id,
                    "element_index": node.index,
                    "snapshot_id": self.snapshot_id,
                },
            )
            # A beat for the menu to open: it has no element to wait on, and
            # the read-back below is what decides.
            time.sleep(MENU_BEAT_S)
            keys = ["down" if steps > 0 else "up"] * abs(steps) + ["return"]
            for key in keys:
                # Foreground: an open native menu only takes genuine key events.
                self.bridge.action(
                    "press_key",
                    {
                        "pid": window.pid,
                        "window_id": window.window_id,
                        "key": key,
                        "delivery_mode": "foreground",
                    },
                )
        node = self._one(window, field, ("AXPopUpButton", "AXComboBox"), False)
        if (node.value or "") != option:
            raise StepFailed(
                f"«{field}» is still on «{node.value or current}» after {CHOOSE_ATTEMPTS} "
                f"attempts to choose «{option}» with the keyboard"
            )


class CuaBridge:
    """CuaDriver on the Mac, one `cua-driver call` per action (mac.py)."""

    def __init__(self, mac):
        self.mac = mac
        self.last_reply_bytes: int | None = None

    def action(self, tool: str, arguments: dict) -> dict:
        done = self.mac.engine_call(tool, arguments)
        self.last_reply_bytes = done.get("bytes")
        result = done.get("result")
        return result if isinstance(result, dict) else {}
