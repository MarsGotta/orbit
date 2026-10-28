"""Adding vendor agents needs no operator tool tables or provider connection."""

import json
import sys
from dataclasses import replace
from pathlib import Path

import claude_agent_sdk
import pytest
from conftest import device
from fastapi.testclient import TestClient

from orbit_hub.app import create_app
from orbit_hub.assistant.agents import Agents
from orbit_hub.assistant.roster import Profile, Roster
from orbit_hub.errors import HubError


@pytest.fixture
def shipped_codex(tmp_path, monkeypatch):
    release = tmp_path / "release"
    path = release / "bin" / "codex"
    path.parent.mkdir(parents=True)
    path.write_text("#!/bin/sh\nexit 99\n")  # Adding must never run it or sign in.
    path.chmod(0o755)
    monkeypatch.setattr(sys, "prefix", str(release / ".venv"))
    return path


def test_no_tool_tables_add_both_services_by_absolute_path(environment, shipped_codex):
    settings, store, _ = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    paths = {
        "claude-code": Path(claude_agent_sdk.__file__).parent / "_bundled" / "claude",
        "codex": shipped_codex,
    }
    with TestClient(create_app(settings, store=store), base_url="https://localhost") as http:
        for service, name in (("claude-code", "Aster"), ("codex", "Lyra")):
            answer = http.post(
                "/v1/agents",
                headers=headers,
                json={
                    "service": service,
                    "name": name,
                    "description": "",
                    "color": "c1",
                    "access": dict.fromkeys(
                        ("screen", "control", "calendar", "meetings", "files"), False
                    ),
                },
            )
            assert answer.status_code == 201, answer.json()
    # A restart still resolves defaults for the stored agents.
    agents = Agents.from_settings(settings, roster=Roster(store))
    for agent in list(agents.by_id.values())[1:]:
        assert agent.settings.command == (str(paths[agent.kind]),)
        assert agent.backend.mcp_server()["args"] == ["-m", "orbit_mcp.cli"]
        assert agent.settings.orbit_mcp_config == settings.data_dir / "assistant-mcp.toml"


def test_generated_mcp_config_uses_actual_origin_and_private_persistent_keys(environment):
    from orbit_hub.assistant.local_mcp import prepare
    from orbit_mcp.config import Config

    settings, _, _ = environment
    prepared = prepare(settings, "http://127.0.0.1:19431")
    path = prepared.assistant_tool_settings()["codex"].orbit_mcp_config
    config = Config.load(path)
    assert config.hub_url == "http://127.0.0.1:19431"
    assert config.devices == {}
    assert config.meetings is not None
    assert path.stat().st_mode & 0o777 == 0o600
    for bridge, credentials in (
        (prepared.device_bridge, config.device_bridge),
        (prepared.meetings_bridge, config.meetings),
    ):
        key = Path(bridge.credential_file)
        assert key.stat().st_mode & 0o777 == 0o600
        assert key.read_bytes() == credentials.key
    again = prepare(settings, "http://127.0.0.1:19432")
    assert again.device_bridge == prepared.device_bridge
    assert Config.load(path).device_bridge.key == config.device_bridge.key
    assert Config.load(path).hub_url.endswith(":19432")


def test_existing_bridge_and_mcp_overrides_are_preserved(environment, tmp_path):
    from orbit_hub.assistant.local_mcp import prepare
    from orbit_hub.private_auth import BridgeConfiguration
    from orbit_mcp.config import Config

    settings, _, _ = environment
    key = tmp_path / "existing.key"
    key.write_bytes(bytes(range(32)))
    key.chmod(0o600)
    bridge = BridgeConfiguration(key_id="example-bridge", credential_file=str(key))
    override = tmp_path / "operator.toml"
    override.write_text("preserve me")
    settings = replace(
        settings,
        device_bridge=bridge,
        assistant_tools={
            "claude-code": {"command": [sys.executable]},
            "codex": {
                "command": [sys.executable, "-I"],
                "orbit_mcp_config": str(override),
                "env": ["LANG"],
            },
        },
    )
    prepared = prepare(settings, "https://hub.example")
    tools = prepared.assistant_tool_settings()
    assert tools["codex"].command == (sys.executable, "-I")
    assert tools["codex"].env == ("LANG",)
    assert tools["codex"].orbit_mcp_config == override
    assert override.read_text() == "preserve me"
    assert Config.load(tools["claude-code"].orbit_mcp_config).device_bridge.key == key.read_bytes()
    assert prepared.device_bridge == bridge


def test_generated_translator_credentials_authenticate_against_this_hub(environment):
    from uuid import uuid4

    from orbit_hub.assistant.local_mcp import prepare
    from orbit_hub.bridge_route import DEVICES_PATH, PROTOCOL
    from orbit_hub.private_auth import sign_request
    from orbit_mcp.config import Config

    settings, store, _ = environment
    paired = device(store)
    prepared = prepare(settings, "https://localhost")
    config = Config.load(prepared.assistant_tool_settings()["codex"].orbit_mcp_config)
    headers = sign_request(
        PROTOCOL,
        DEVICES_PATH,
        config.device_bridge.key_id,
        config.device_bridge.key,
        b"",
        issued_at_ms=1_001_000,
        nonce=str(uuid4()),
    )
    app = create_app(prepared, store=store, now=lambda: 1001)
    with TestClient(app, base_url=config.hub_url) as http:
        assert http.get(DEVICES_PATH).status_code == 401
        response = http.get(DEVICES_PATH, headers=headers)
    assert response.status_code == 200, response.json()
    assert paired["device_id"] in response.text


@pytest.mark.parametrize("service", ["claude-code", "codex"])
def test_missing_default_names_service_path_and_republish(
    environment, shipped_codex, monkeypatch, service
):
    import orbit_hub.assistant.tool_settings as module

    path = shipped_codex.parent / (service + "-missing")
    monkeypatch.setattr(module, "bundled_command", lambda _: (str(path),))
    settings, store, _ = environment
    agents = Agents.from_settings(settings, roster=Roster(store))
    with pytest.raises(HubError) as caught:
        agents.add(service, Profile("Aster", "", "c1"), 1001)
    error = caught.value
    assert error.code == "agent_adapter_missing"
    assert service in error.message and str(path) in error.message
    assert "publish the hub again" in error.message
    assert "quien gestiona" not in error.message
    assert error.numbers["path"] == str(path)
    assert error.numbers["cause"] == "does not exist"
    assert json.dumps(error.numbers)


def test_serve_prepares_mcp_with_its_listener(environment, monkeypatch):
    from argparse import Namespace

    from orbit_hub.cli import serve
    from orbit_mcp.config import Config

    settings, _, _ = environment
    settings = replace(settings, allow_insecure_localhost=True)
    seen = {}
    monkeypatch.setattr(
        "orbit_hub.app.create_app", lambda configured: seen.setdefault("settings", configured)
    )
    monkeypatch.setattr("uvicorn.run", lambda *a, **kw: None)
    serve(settings, Namespace(host="127.0.0.1", port=19431, tls_cert=None, tls_key=None))
    prepared = seen["settings"]
    assert (
        Config.load(prepared.assistant_tool_settings()["codex"].orbit_mcp_config).hub_url
        == "http://127.0.0.1:19431"
    )
    assert prepared.device_bridge is not None and prepared.meetings_bridge is not None


@pytest.mark.parametrize("service", ["claude-code", "codex"])
async def test_wrong_platform_binary_names_service_path_and_cause(environment, tmp_path, service):
    from orbit_hub.assistant.backend import TextBlock

    settings, store, _ = environment
    path = tmp_path / "wrong-platform"
    path.write_bytes(b"not an executable for this platform\n")
    path.chmod(0o755)
    settings = replace(settings, assistant_tools={service: {"command": [str(path)]}})
    agents = Agents.from_settings(settings, roster=Roster(store))
    agent = agents.add(service, Profile("Aster", "", "c1"), 1001)

    async def emit(_):
        pass

    connection = agent.backend.connection("example-mac", "Example Mac", emit)
    await connection.open()
    try:
        with pytest.raises(HubError) as caught:
            opened = await connection.create_session()
            if service == "claude-code":
                await connection.live(opened.session_id).execute([TextBlock("Never sent")])
        assert caught.value.code == "agent_adapter_missing"
        assert caught.value.numbers == {
            "service": service,
            "path": str(path),
            "cause": "the binary cannot be executed on this platform",
            "cause_code": "adapter_format",
        }
    finally:
        await connection.close()


@pytest.mark.parametrize("service", ["claude-code", "codex"])
async def test_sign_in_to_unusable_binary_keeps_installation_refusal(
    environment, tmp_path, service
):
    from orbit_hub.assistant.sign_in import SignIns

    settings, store, _ = environment
    path = tmp_path / "wrong-platform"
    path.write_bytes(b"invalid executable\n")
    path.chmod(0o755)
    settings = replace(settings, assistant_tools={service: {"command": [str(path)]}})
    agents = Agents.from_settings(settings, roster=Roster(store))
    agent = agents.add(service, Profile("Aster", "", "c1"), 1001)
    with pytest.raises(HubError) as caught:
        await SignIns(settings, agents).start(agent.id)
    assert caught.value.code == "agent_adapter_missing"
    assert caught.value.numbers["service"] == service
    assert caught.value.numbers["path"] == str(path)
    assert "platform" in caught.value.numbers["cause"]
