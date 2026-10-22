"""Imported Python copies of the shared bounds in contracts/constants.json."""

import importlib
import json
from collections.abc import Mapping
from pathlib import Path

from orbit_hub import device_bridge
from orbit_mcp import hub as translator_hub

REGISTRY = Path(__file__).resolve().parents[2] / "contracts/constants.json"


def values():
    return {entry["name"]: entry["value"] for entry in json.loads(REGISTRY.read_text())}


def as_json(value):
    """A Python constant as JSON reads it: a mapping is an object, a tuple a list."""
    if isinstance(value, Mapping):
        return {key: as_json(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [as_json(item) for item in value]
    return value


def test_python_copies_match_exported_limits():
    for entry in json.loads(REGISTRY.read_text()):
        for copy in entry["copies"]:
            if not copy["file"].startswith("server/"):
                continue
            module = copy["file"].removeprefix("server/").removesuffix(".py").replace("/", ".")
            actual = importlib.import_module(module)
            for name in copy["name"].split("."):
                if isinstance(actual, Mapping):
                    actual = actual[name]
                elif isinstance(actual, (tuple, list)):
                    actual = actual[int(name)]
                else:
                    actual = getattr(actual, name)
            assert copy["unit"] == entry["unit"], copy
            actual = as_json(actual)
            assert actual == entry["value"], (
                f"{copy['file']}:{copy['name']}={actual} {copy['unit']}; "
                f"esperado {entry['value']} {entry['unit']} ({entry['name']})"
            )


def test_bridge_deadlines_leave_room_for_the_next_boundary():
    expected = values()
    assert (
        expected["CAPTURE_UPLOAD_TIMEOUT_SECONDS"]
        < device_bridge.REQUEST_TIMEOUT_SECONDS
        < translator_hub.BRIDGE_TIMEOUT_SECONDS
    )
