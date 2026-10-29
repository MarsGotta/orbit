"""Orbit Pruebas's DOM driver over its owner-local test bridge.

Every read is saved as the bridge returned it. Actions carry the role, exact
accessible name and DOM target of the selected row; the view revalidates them
before clicking/typing. Lost receipts are never retried. CuaDriver belongs only
to the separate read-only macOS accessibility check.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path

import ax_tree

BUNDLE_ID = "app.orbit.companion.pruebas"
PANEL_TITLE = "Orbit · llamada"


class StepFailed(Exception):
    """A step could not be done; the text says why, with numbers."""


@dataclass(frozen=True)
class Window:
    pid: int
    window_id: int
    title: str
    label: str = "main"
    visible: bool = True


def candidates(nodes, text, roles, contains):
    found = []
    exact = ax_tree.find(nodes, text, roles, contains)
    for at, node in enumerate(nodes):
        if node.index is None or (roles and node.role not in roles):
            continue
        end = at + 1
        while end < len(nodes) and nodes[end].depth > node.depth:
            end += 1
        # A link can carry live metadata beside its visible label. Its guarded
        # press still uses the full accessible name returned by the bridge.
        labels = [value for child in nodes[at + 1 : end] for value in child.texts()]
        if node in exact or any(text in value if contains else text == value for value in labels):
            found.append(node)
    return found


class Interface:
    def __init__(self, bridge, artifacts: Path, app_pid: int = 0):
        self.bridge = bridge
        self.artifacts = artifacts
        self.app_pid = app_pid
        self.reads: list[dict] = []
        self.trees: dict[str, str] = {}
        self._read_count = 0
        self._elements: dict[int, dict] = {}

    def windows(self) -> dict[str, Window]:
        reply = self.bridge.call({"command": "windows"})
        self.app_pid = reply.get("pid", self.app_pid)
        found = {}
        for row in reply.get("windows", []):
            label = row.get("label")
            if label in ("main", "panel"):
                found[label] = Window(
                    self.app_pid,
                    int(row["window_id"]),
                    "Orbit" if label == "main" else PANEL_TITLE,
                    label,
                    row["visible"],
                )
        if "main" not in found:
            raise StepFailed(f"test bridge sees {len(found)} app windows, none labelled main")
        return found

    def reconnect(self) -> dict:
        # No cached connection or pid survives: windows is a new socket request
        # to the new process, and every DOM target must come from a fresh read.
        self._elements.clear()
        main = self.windows()["main"]
        return {"app_pid": main.pid, "window_id": main.window_id}

    def panel(self, bound_s: float) -> Window:
        deadline = time.monotonic() + bound_s
        while True:
            found = self.windows()
            if "panel" in found and found["panel"].visible:
                return found["panel"]
            if time.monotonic() >= deadline:
                raise StepFailed(f"test bridge has no panel within {bound_s:.0f} s")
            time.sleep(0.5)

    def read(self, window: Window, label: str, query: str | None) -> list[ax_tree.Node]:
        request = {"command": "read", "window": window.label}
        if query is not None:
            request["query"] = query
        reply = self.bridge.call(request)
        markdown = reply.get("tree_markdown")
        if not isinstance(markdown, str):
            raise StepFailed(f"test bridge returned no tree for «{label}»")
        # Defensive privacy boundary, also used by the separate AX collector.
        markdown = ax_tree.without_menu_bar(markdown)
        self._elements = {row["index"]: row for row in reply.get("elements", []) if "index" in row}
        size = getattr(self.bridge, "last_reply_bytes", None)
        self.reads.append({"label": label, "query": query, "bytes": size, "source": "bridge"})
        self._read_count += 1
        name = f"{self._read_count:03d}-{label.replace('/', '-').replace(' ', '-')}.txt"
        (self.artifacts / name).write_text(markdown, encoding="utf-8")
        if not label.startswith("find "):
            self.trees[label] = markdown
        return ax_tree.parse(markdown)

    def _one(
        self, window: Window, text: str, roles: tuple[str, ...], contains: bool, first: bool = False
    ) -> ax_tree.Node:
        deadline = time.monotonic() + 10
        while True:
            nodes = self.read(window, f"find {text}", text)
            found = candidates(nodes, text, roles, contains)
            if found or time.monotonic() >= deadline:
                break
            time.sleep(0.5)
        if first and found:
            return found[0]
        if len(found) != 1:
            raise StepFailed(
                f"«{text}» ({', '.join(roles) or 'any role'}): {len(found)} "
                "pressable elements instead of one"
            )
        return found[0]

    def _request(self, command, window, text, roles, contains=False, first=False) -> dict:
        node = self._one(window, text, roles, contains, first)
        element = self._elements.get(node.index)
        if not element or "target" not in element:
            raise StepFailed(f"test bridge row {node.index} has no guarded DOM target for «{text}»")
        return {
            "command": command,
            "window": window.label,
            "role": element["role"],
            "name": element["name"],
            "target": element["target"],
        }

    def press(
        self,
        window: Window,
        text: str,
        roles: tuple[str, ...] = (),
        contains: bool = False,
        first: bool = False,
    ) -> None:
        self.bridge.call(self._request("press", window, text, roles, contains, first))

    def typing(self, window: Window, field: str, text: str, contains: bool = False) -> dict:
        return {
            **self._request("type", window, field, ("AXTextArea", "AXTextField"), contains),
            "text": text,
        }

    def type_into(self, window: Window, field: str, text: str, contains: bool = False) -> None:
        self.bridge.call(self.typing(window, field, text, contains))

    def read_until(
        self, window: Window, label: str, query: str | None, condition, bound_s: float
    ) -> float:
        return self.read_nodes_until(
            window, label, query, lambda nodes: condition(ax_tree.all_text(nodes)), bound_s
        )

    def read_nodes_until(
        self, window: Window, label: str, query: str | None, condition, bound_s: float
    ) -> float:
        start = time.monotonic()
        while True:
            nodes = self.read(window, label, query)
            if condition(nodes) or time.monotonic() - start > bound_s:
                return time.monotonic() - start
            time.sleep(1)


class InAppBridge:
    """One Mac-side client call, opening the current process's socket afresh."""

    def __init__(self, mac):
        self.mac = mac
        self.last_reply_bytes = None

    def call(self, request: dict) -> dict:
        done = self.mac.bridge_call(request)
        self.last_reply_bytes = done.get("bytes")
        return done["result"]
