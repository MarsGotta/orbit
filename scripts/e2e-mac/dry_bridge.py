"""Replayed web-view boundary for the Linux real-harness dry run."""

from __future__ import annotations

import json
import math
import time
from datetime import UTC, datetime

import ax_tree
from mac_controls import MacFailure

PID = 4242
MAIN_WINDOW = 11
PANEL_WINDOW = 12


def query_filter(markdown: str, query: str) -> str:
    lines = markdown.splitlines()
    kept = set()
    for position, line in enumerate(lines):
        if query.lower() in line.lower():
            kept.add(position)
            indent = len(line) - len(line.lstrip())
            for earlier in range(position - 1, -1, -1):
                other = len(lines[earlier]) - len(lines[earlier].lstrip())
                if other < indent:
                    kept.add(earlier)
                    indent = other
    return "\n".join(lines[i] for i in sorted(kept)) + "\n"


class DryRunView:
    def __init__(self, example, logs):
        self.example, self.logs = example, logs
        self.transitions = example.base("transitions.json")
        self.transitions.update(example.overrides("transitions.json"))
        self.screen, self.panel = "home", "panel"
        self.generation = 0

    @property
    def pid(self):
        return PID + self.generation - 1

    def window_id(self, label):
        return (PANEL_WINDOW if label == "panel" else MAIN_WINDOW) + 100 * (self.generation - 1)

    def follow(self, key):
        target = self.transitions.get(self.screen, {}).get(key)
        if isinstance(target, dict):
            for name, row in target.get("log", {}).items():
                self._log(name, row)
            target = target["to"]
        if target is not None:
            self.screen = target

    def _log(self, name, row):
        instant = datetime.fromtimestamp(math.ceil(time.time() * 1000) / 1000, UTC)
        row = dict(row, ts=instant.isoformat(timespec="milliseconds").replace("+00:00", "Z"))
        with open(self.logs / name, "a", encoding="utf-8") as log:
            log.write(json.dumps(row) + "\n")
        time.sleep(0.005)

    def tree(self, label):
        return self.example.text(f"screens/{self.panel if label == 'panel' else self.screen}.md")

    def pointer(self, window_id, where):
        target = self.transitions.get(self.panel, {}).get(f"pointer:{where}")
        if target is not None:
            self.panel = target
        inside = where == "over"
        return (
            0,
            json.dumps(
                {
                    "window_id": window_id,
                    "bounds": {"x": 1084.0, "y": 49.0, "width": 340.0, "height": 300.0},
                    "before": {"x": 700.0, "y": 450.0},
                    "after": {"x": 1254.0 if inside else 1044.0, "y": 199.0},
                    "warped": True,
                    "inside": inside,
                    "scale": 2.0,
                }
            )
            + "\n",
            "",
        )


def projected(markdown, generation=1):
    """Fixtures record semantic trees, independently of CuaDriver's read path."""
    import dataclasses

    lines, rows = [], []
    roles = {
        "AXButton": "button",
        "AXLink": "link",
        "AXTextArea": "textbox",
        "AXTextField": "textbox",
        "AXRadioButton": "radio",
        "AXTab": "tab",
        "AXPopUpButton": "combobox",
    }
    for at, node in enumerate(ax_tree.parse(markdown)):
        kind = roles.get(node.role)
        index = at + 1 if kind else None
        node = dataclasses.replace(node, index=index)
        prefix = "  " * node.depth + "- " + (f"[{index}] " if index else "") + node.role
        if node.title is not None:
            prefix += ' "' + node.title + '"'
        if node.value is not None:
            prefix += ' = "' + node.value + '"'
        if node.description is not None:
            prefix += " (" + node.description + ")"
        lines.append(prefix)
        if kind:
            rows.append(
                {
                    "index": index,
                    "role": kind,
                    "name": node.title or node.description or "",
                    "text": node.value or "",
                    "target": [42, generation, index],
                    "fixture_text": node.text(),
                }
            )
    return "\n".join(lines) + "\n", rows


class DryRunBridge:
    def __init__(self, example, view):
        self.example, self.view = example, view
        self.calls = []
        self.last_reply_bytes = None

    def call(self, request):
        self.calls.append(dict(request))
        reply = self._call(request)
        self.last_reply_bytes = len(json.dumps(reply, ensure_ascii=False).encode()) + 1
        return reply

    def _call(self, request):
        command = request["command"]
        if command == "menus":
            return self.example.json("bridge.json")
        if command == "windows":
            return {
                "ok": True,
                "pid": self.view.pid,
                "windows": [
                    {
                        "label": label,
                        "window_id": self.view.window_id(label),
                        "visible": True,
                        "frame": {"x": 0, "y": 0, "width": 800, "height": 600},
                    }
                    for label in ("main", "panel")
                ],
            }
        label = request["window"]
        markdown, elements = projected(self.view.tree(label), self.view.generation)
        if command == "read":
            if request.get("query"):
                markdown = query_filter(markdown, request["query"])
            if label == "main":
                self.view.follow("read")
            return {"ok": True, "tree_markdown": markdown, "elements": elements}
        matches = [
            row
            for row in elements
            if row["role"] == request["role"]
            and row["name"] == request["name"]
            and ("target" not in request or row["target"] == request["target"])
        ]
        if len(matches) != 1:
            raise MacFailure(f"test bridge match_count: {len(matches)} for {request['name']}")
        if command == "type":
            self.example.message = request["text"]
        self.view.follow(f"{command}:{matches[0]['fixture_text']}")
        return {"ok": True, "count": 1}
