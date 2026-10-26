"""Every hub-v1 wire variant is shared by both language consumers."""

import json
from pathlib import Path
from typing import get_args

import pytest

from orbit_hub.protocol import HUB_PROTOCOL, PARAMETERS, Frame

ROOT = Path(__file__).resolve().parents[2] / "contracts" / "fixtures" / "hub"
REQUESTS = {
    "chat.send",
    "chat.observe",
    "chat.interrupt",
    "approval.respond",
    "clarify.respond",
    "ping",
    "device.reply",
}


def names(folder):
    files = list(folder.iterdir())
    assert all(file.suffix == ".json" for file in files)
    return {file.stem for file in files}


def test_fixtures_use_the_registered_hub_protocol():
    for folder in (ROOT / "requests", ROOT / "events"):
        for path in folder.glob("*.json"):
            assert json.loads(path.read_text())["v"] == HUB_PROTOCOL


def test_request_fixture_inventory_matches_hub_consumer():
    from orbit_hub.ws_frames import METHODS

    assert set(METHODS) == REQUESTS
    assert names(ROOT / "requests") == REQUESTS
    assert set(get_args(Frame.model_fields["method"].annotation)) == REQUESTS
    assert set(PARAMETERS) == REQUESTS


@pytest.mark.parametrize("name", sorted(REQUESTS))
def test_request_fixture_is_accepted_by_hub(name):
    raw = json.loads((ROOT / "requests" / f"{name}.json").read_text())
    frame = Frame.model_validate(raw)
    assert frame.method == name
    PARAMETERS[name].model_validate(frame.params)
