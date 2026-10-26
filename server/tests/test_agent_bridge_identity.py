"""Jailed translators cannot borrow another agent's access groups."""

import json
import time
from pathlib import Path
from uuid import uuid4

import pytest
from fastapi.testclient import TestClient
from support_orbit_mcp import write_mcp_config
from test_official_assistants import official, send, until  # noqa: F401

from orbit_hub.app import create_app
from orbit_hub.private_auth import BridgeConfiguration, sign_request
from orbit_mcp.config import Config

# ruff: noqa: F811


def signed(protocol, path, credential, body):
    return {
        "content-type": "application/json",
        **sign_request(
            protocol,
            path,
            credential.key_id,
            credential.key,
            body,
            issued_at_ms=int(time.time() * 1000),
            nonce=str(uuid4()),
        ),
    }


@pytest.mark.parametrize("claim", ["helper", None, "hermes"])
def test_agent_key_binds_identity_on_device_and_meetings_routes(official, tmp_path, claim):
    _, settings, _, paired, headers, client = official
    write_mcp_config(tmp_path, "https://localhost")
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="WAIT")
        session = until(ws, "chat.session", frames)["session_id"]
        until(ws, "chat.delta", frames)
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        until(ws, "chat.completed", frames)
        config = Config.load(settings.data_dir / "agents/helper/bridge/mcp.toml")
        base = Config.load(tmp_path / "mcp.toml")
        assert config.device_bridge.key != base.device_bridge.key
        assert config.meetings.key != config.device_bridge.key
        for path, protocol, credential, payload in (
            ("/internal/v1/agent/access", "orbit_device_bridge_v1", config.device_bridge, {}),
            (
                "/internal/v1/meetings/list",
                "orbit_meetings_bridge_v1",
                config.meetings,
                {"device_id": paired["device_id"], "since_ms": 0, "limit": 10},
            ),
        ):
            body = json.dumps(
                {
                    "agent_id": claim,
                    **({"protocol": protocol} if "agent/access" in path else {}),
                    **payload,
                }
            ).encode()
            response = http.post(
                path, content=body, headers=signed(protocol, path, credential, body)
            )
            assert response.status_code == (403 if claim == "hermes" else 200), response.text
            if claim != "hermes" and path.endswith("/access"):
                assert response.json()["agent_id"] == "helper"
    assert Path(settings.data_dir / "agents/helper/bridge/mcp.toml").stat().st_mode & 0o777 == 0o600


def test_operator_bridge_key_keeps_hermes_selection(official, tmp_path):
    from dataclasses import replace

    _, settings, store, _, _, _ = official
    write_mcp_config(tmp_path, "https://localhost")
    base = Config.load(tmp_path / "mcp.toml")
    configured = replace(
        settings,
        agents=(
            *settings.agents,
            {"id": "hermes", "name": "Deimos", "kind": "hermes", "profile": "default"},
        ),
        default_agent="hermes",
        device_bridge=BridgeConfiguration(
            key_id=base.device_bridge.key_id,
            credential_file=str(tmp_path / "device-bridge.key"),
        ),
    )
    path = "/internal/v1/agent/access"
    with TestClient(create_app(configured, store=store), base_url="https://localhost") as http:
        for claim in (None, "hermes", "helper"):
            body = json.dumps({"protocol": "orbit_device_bridge_v1", "agent_id": claim}).encode()
            response = http.post(
                path,
                content=body,
                headers=signed("orbit_device_bridge_v1", path, base.device_bridge, body),
            )
            assert response.status_code == 200, response.text
            assert response.json()["agent_id"] == (claim or "hermes")
