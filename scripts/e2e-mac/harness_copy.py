"""Checked exports of Orbit's UI/native catalogues, usable by Mac Python 3.9.

The app's tests evaluate its tables/formatters and compare these JSON files.
No Python label table exists. A scoped language also lets pure checks run over
saved evidence from either pass without changing the process's defaults.
"""

from __future__ import annotations

import json
import re
from collections.abc import Mapping
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path

DIRECTORY = Path(__file__).resolve().parent
LANGUAGE = ContextVar("harness_language", default="es")
UI = json.loads((DIRECTORY / "ui-copy.json").read_text(encoding="utf-8"))
NATIVE = json.loads((DIRECTORY / "native-copy.json").read_text(encoding="utf-8"))
TOKEN = re.compile(r"\{([a-zA-Z][\w.]+)\}")


def language() -> str:
    return LANGUAGE.get()


@contextmanager
def language_scope(value: str):
    if value not in ("es", "en"):
        raise ValueError(f"unsupported test language {value!r}; expected es or en")
    token = LANGUAGE.set(value)
    try:
        yield
    finally:
        LANGUAGE.reset(token)


def text(key: str, *, catalogue_language: str | None = None, **values) -> str:
    selected = catalogue_language or LANGUAGE.get()
    if key == "pass.language":
        return UI[selected]["notes.languages." + selected]
    if key.startswith("native."):
        value = NATIVE[selected][key.removeprefix("native.")]
    elif key.startswith("core."):
        value = NATIVE["core"][key.removeprefix("core.")]
    else:
        value = UI[selected][key]
    if not isinstance(value, str):
        raise KeyError(f"catalogue key {key} is not a text ({selected})")
    return value.format(**values) if values else value


def render(message: str) -> str:
    return TOKEN.sub(lambda match: text(match[1]), message)


class Aliases(Mapping):
    """Names local to a check refer to catalogue keys, never copied values."""

    def __init__(self, keys):
        self.keys_by_name = keys

    def __getitem__(self, name):
        key = self.keys_by_name[name]
        return text(key) if key is not None else None

    def __iter__(self):
        return iter(self.keys_by_name)

    def __len__(self):
        return len(self.keys_by_name)


class Messages(Aliases):
    """English operator diagnostics can quote labels from the active pass."""

    def __getitem__(self, name):
        return render(self.keys_by_name[name])


def validate_references(directory: Path = DIRECTORY) -> None:
    """Check literal keys even on refusal paths the good replay never visits."""
    import ast

    keys = set()
    for path in directory.glob("*.py"):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name):
                continue
            if (
                node.func.id in ("copy_text", "text")
                and node.args
                and isinstance(node.args[0], ast.Constant)
                and isinstance(node.args[0].value, str)
            ):
                keys.add(node.args[0].value)
            if node.func.id in ("Aliases", "Messages") and node.args:
                if not isinstance(node.args[0], ast.Dict):
                    continue
                for value in node.args[0].values:
                    if not isinstance(value, ast.Constant) or not isinstance(value.value, str):
                        continue
                    if node.func.id == "Aliases":
                        keys.add(value.value)
                    else:
                        keys.update(TOKEN.findall(value.value))
    for selected in ("es", "en"):
        for key in sorted(keys):
            text(key, catalogue_language=selected)
