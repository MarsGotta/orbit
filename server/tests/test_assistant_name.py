"""The hub selects and validates the name before advertising it to a Mac."""

from dataclasses import replace

import pytest
from conftest import device
from support_app import client

from orbit_hub.config import Settings


def config_file(tmp_path, name_line=""):
    path = tmp_path / "orbit.toml"
    path.write_text(
        name_line
        + """data_dir = "/fixture/data"
key_file = "/fixture/key"
hermes_url = "https://hermes.example.test"
hermes_credential_file = "/fixture/token"
[meeting_transcription]
endpoint = "https://transcriber.example.test"
model = "fixture-model"
"""
    )
    return path


def test_name_comes_from_toml_or_environment_and_defaults_to_deimos(tmp_path, monkeypatch):
    monkeypatch.delenv("ORBIT_HUB_ASSISTANT_NAME", raising=False)
    assert getattr(Settings.load(config_file(tmp_path)), "assistant_name", None) == "Deimos"
    path = config_file(tmp_path, 'assistant_name = "Atlas"\n')
    assert Settings.load(path).assistant_name == "Atlas"
    monkeypatch.setenv("ORBIT_HUB_ASSISTANT_NAME", "Átlas Azul")
    assert Settings.load(path).assistant_name == "Átlas Azul"
    monkeypatch.setenv("ORBIT_HUB_ASSISTANT_NAME", "")
    with pytest.raises(ValueError, match="assistant_name is empty: 0 characters; maximum 48"):
        Settings.load(path)


@pytest.mark.parametrize(
    "name,cause,count",
    [
        ("", "is empty", 0),
        ("  ", "is empty", 2),
        ("A" * 49, "is too long", 49),
        ("At\nlas", "contains control characters", 6),
        ("At\x7flas", "contains control characters", 6),
    ],
)
def test_invalid_name_refuses_startup_with_cause_count_and_ceiling(tmp_path, name, cause, count):
    import json

    path = config_file(tmp_path, f"assistant_name = {json.dumps(name)}\n")
    with pytest.raises(
        ValueError, match=rf"assistant_name {cause}: {count} characters; maximum 48"
    ):
        Settings.load(path)


@pytest.mark.parametrize("name", [None, "Atlas", "Á" * 48])
def test_ready_advertises_the_configured_name(environment, name):
    settings, store, adapter = environment
    if name is not None:
        settings = replace(settings, assistant_name=name)
    paired = device(store)
    with (
        client((settings, store, adapter)) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws,
    ):
        ready = ws.receive_json()
        assert ready["event"] == "ready"
        assert ready["data"].get("assistant_name") == (name or "Deimos")
