"""Access groups, enforced in both places: orbit-mcp and the hub's bridges.

orbit-mcp neither lists nor runs a tool of a group its agent has off; the
hub refuses a bridge request of such a group, naming the agent and the
group. The agent is the one `ORBIT_MCP_AGENT` names, or the default one.
"""

import json
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import BRIDGE_KEY, BRIDGE_KEY_ID, BRIDGE_NOW_MS, device
from conftest import bridge_body as body
from conftest import bridge_signed as signed
from support_orbit_mcp import TOOL_NAMES, FakeHub, serving, text_of

from orbit_hub.app import create_app
from orbit_hub.assistant import access
from orbit_hub.assistant.agent_config import AgentConfigStore
from orbit_hub.bridge_route import ACCESS_PATH, ACTION_RESULT_PATH, PATH, PROTOCOL
from orbit_hub.meetings_bridge import LIST_PATH
from orbit_hub.meetings_bridge import PROTOCOL as MEETINGS_PROTOCOL
from orbit_hub.private_auth import sign_request
from orbit_mcp.bridge import PARAMETER_MODELS
from orbit_mcp.config import Config, ConfigError

EVERY_GROUP = dict.fromkeys(access.GROUPS, True)
MEETINGS_KEY = bytes(range(32, 64))
MEETINGS_KEY_ID = "meetings-bridge-test-key"


def test_every_tool_and_capability_belongs_to_one_group():
    grouped = [tool for tools in access.TOOLS.values() for tool in tools]
    assert sorted(grouped) == sorted(set(TOOL_NAMES) - access.UNGATED)
    # A capability without a group would pass the hub's check unguarded.
    assert set(access.CAPABILITY_GROUP) == set(PARAMETER_MODELS)


# orbit-mcp ----------------------------------------------------------------------


async def test_orbit_mcp_lists_no_tool_of_a_group_turned_off(tmp_path):
    hub = FakeHub(access={"default": {**EVERY_GROUP, "screen": False, "files": False}})
    async with serving(tmp_path, hub) as (_client, server):
        listed = {tool.name for tool in await server.list_tools()}
    assert listed == set(TOOL_NAMES) - set(access.TOOLS["screen"]) - set(access.TOOLS["files"])


async def test_orbit_mcp_refuses_a_tool_of_a_group_turned_off_if_called_anyway(tmp_path):
    hub = FakeHub(access={"default": {**EVERY_GROUP, "screen": False}})
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_screenshot", {})
        allowed = await server.call_tool("orbit_calendar", {"start_ms": 0, "end_ms": 1})
    assert result.is_error
    text = text_of(result)
    assert text.startswith("agent_access_disabled: ")
    assert "agente: default" in text and "acceso: screen" in text
    # Nothing reached the Mac for the refused tool; another group's still does.
    assert [request["capability"] for request in hub.device_requests] == ["calendar_read"]
    assert "agent_access_disabled" not in text_of(allowed)


async def test_orbit_mcp_offers_every_tool_when_the_hub_cannot_say(tmp_path):
    """Unknown is not off: the hub still refuses what is off when it is asked."""
    async with serving(tmp_path, FakeHub()) as (_client, server):
        listed = {tool.name for tool in await server.list_tools()}
    assert listed == set(TOOL_NAMES)


async def test_orbit_mcp_speaks_for_the_agent_it_was_started_for(tmp_path):
    hub = FakeHub(access={"lyra": {**EVERY_GROUP, "meetings": False}})
    async with serving(tmp_path, hub) as (client, server):
        client.config = replace(client.config, agent_id="lyra")
        listed = {tool.name for tool in await server.list_tools()}
        await server.call_tool("orbit_windows", {})
    assert hub.access_requests == ["lyra"]
    assert listed.isdisjoint(access.TOOLS["meetings"]) and "orbit_windows" in listed
    assert hub.device_requests[0]["agent_id"] == "lyra"


def test_orbit_mcp_takes_its_agent_from_the_environment(tmp_path, monkeypatch):
    from support_orbit_mcp import write_mcp_config

    monkeypatch.setenv("ORBIT_MCP_CONFIG", str(write_mcp_config(tmp_path, "https://hub.example")))
    monkeypatch.delenv("ORBIT_MCP_AGENT", raising=False)
    assert Config.from_env().agent_id is None
    monkeypatch.setenv("ORBIT_MCP_AGENT", "lyra")
    assert Config.from_env().agent_id == "lyra"
    monkeypatch.setenv("ORBIT_MCP_AGENT", "../Lyra")
    with pytest.raises(ConfigError, match="ORBIT_MCP_AGENT"):
        Config.from_env()


# The hub ------------------------------------------------------------------------


@pytest.fixture
def bridged_agents(environment, tmp_path):
    """The hub with two agents, its device and meetings bridges configured."""
    settings, store, adapter = environment
    bridge_key = tmp_path / "bridge-key"
    bridge_key.write_bytes(BRIDGE_KEY)
    bridge_key.chmod(0o600)
    meetings_key = tmp_path / "meetings-key"
    meetings_key.write_bytes(MEETINGS_KEY)
    meetings_key.chmod(0o600)
    settings = replace(
        settings,
        device_bridge={"key_id": BRIDGE_KEY_ID, "credential_file": str(bridge_key)},
        meetings_bridge={"key_id": MEETINGS_KEY_ID, "credential_file": str(meetings_key)},
        agents=(
            {"id": "atlas", "name": "Atlas", "kind": "hermes", "profile": "atlas-perfil"},
            {"id": "vega", "name": "Vega", "kind": "hermes", "profile": "vega-perfil"},
        ),
    )
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: BRIDGE_NOW_MS / 1000
    )
    return app, store


def envelope(agent_id=None, **fields):
    payload = json.loads(body(**fields))
    if agent_id is not None:
        payload["agent_id"] = agent_id
    return json.dumps(payload, separators=(",", ":")).encode()


def refusal_of(response, status, code):
    assert response.status_code == status, response.text
    payload = response.json()
    assert payload["code"] == code
    numbers = {k: v for k, v in payload["refusal"].items() if k not in {"code", "retry"}}
    assert all(isinstance(value, str | int | bool) for value in numbers.values()), numbers
    return numbers, payload["message"]


async def post(app, *requests):
    """Each `(path, payload, headers)` posted in one run of the hub, in order."""
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url="https://localhost") as http,
    ):
        answers = [
            await http.post(path, content=payload, headers=headers)
            for path, payload, headers in requests
        ]
    return answers if len(answers) > 1 else answers[0]


async def test_the_hub_refuses_a_request_of_a_group_its_agent_has_off(bridged_agents):
    app, store = bridged_agents
    AgentConfigStore(store).set_access("vega", {**EVERY_GROUP, "screen": False})
    refused = envelope("vega", capability="screenshot", device_id="equipo-1")
    # The same request for an agent with the group on passes the check and
    # meets the next one: no Mac is connected.
    passed = envelope("atlas", capability="screenshot", device_id="equipo-1")
    first, second = await post(
        app, (PATH, refused, signed(refused)), (PATH, passed, signed(passed))
    )
    numbers, message = refusal_of(first, 403, "agent_access_disabled")
    assert numbers == {"agent_id": "vega", "group": "screen"}
    assert "vega" in message and "screen" in message
    assert second.json()["code"] == "device_unavailable"


async def test_a_request_without_an_agent_is_the_default_agents(bridged_agents):
    app, store = bridged_agents
    AgentConfigStore(store).set_access("atlas", {**EVERY_GROUP, "calendar": False})
    payload = envelope(capability="calendar_read", device_id="equipo-1")
    numbers, _ = refusal_of(
        await post(app, (PATH, payload, signed(payload))), 403, "agent_access_disabled"
    )
    assert numbers == {"agent_id": "atlas", "group": "calendar"}


async def test_an_agent_the_hub_does_not_have_is_named(bridged_agents):
    app, _store = bridged_agents
    payload = envelope("orion", capability="files_list", device_id="equipo-1")
    numbers, _ = refusal_of(
        await post(app, (PATH, payload, signed(payload))), 404, "agent_not_found"
    )
    assert numbers == {"agent_id": "orion", "agent_ids": "atlas,vega"}


async def test_an_action_result_is_the_control_groups(bridged_agents):
    app, store = bridged_agents
    AgentConfigStore(store).set_access("atlas", {**EVERY_GROUP, "control": False})
    payload = json.dumps(
        {"protocol": PROTOCOL, "device_id": "equipo-1", "operation_id": str(uuid4())},
        separators=(",", ":"),
    ).encode()
    numbers, _ = refusal_of(
        await post(app, (ACTION_RESULT_PATH, payload, signed(payload, path=ACTION_RESULT_PATH))),
        403,
        "agent_access_disabled",
    )
    assert numbers == {"agent_id": "atlas", "group": "control"}


async def test_a_meeting_read_is_refused_when_meetings_are_off(bridged_agents):
    app, store = bridged_agents
    owner = device(store)
    AgentConfigStore(store).set_access("vega", {**EVERY_GROUP, "meetings": False})
    payload = json.dumps(
        {"device_id": owner["device_id"], "since_ms": 0, "limit": 5, "agent_id": "vega"},
        separators=(",", ":"),
    ).encode()
    headers = {
        "content-type": "application/json",
        **sign_request(
            MEETINGS_PROTOCOL,
            LIST_PATH,
            MEETINGS_KEY_ID,
            MEETINGS_KEY,
            payload,
            issued_at_ms=BRIDGE_NOW_MS,
            nonce=str(uuid4()),
        ),
    }
    numbers, _ = refusal_of(
        await post(app, (LIST_PATH, payload, headers)), 403, "agent_access_disabled"
    )
    assert numbers == {"agent_id": "vega", "group": "meetings"}


async def test_the_access_route_tells_orbit_mcp_its_agents_groups(bridged_agents):
    app, store = bridged_agents
    AgentConfigStore(store).set_access("vega", {**EVERY_GROUP, "files": False})
    payload = json.dumps({"protocol": PROTOCOL, "agent_id": "vega"}, separators=(",", ":")).encode()
    default = json.dumps({"protocol": PROTOCOL}, separators=(",", ":")).encode()
    response, by_default = await post(
        app,
        (ACCESS_PATH, payload, signed(payload, path=ACCESS_PATH)),
        (ACCESS_PATH, default, signed(default, path=ACCESS_PATH)),
    )
    assert response.status_code == 200, response.text
    assert response.json() == {
        "protocol": "orbit_device_bridge_result_v1",
        "state": "answered",
        "agent_id": "vega",
        "groups": {**EVERY_GROUP, "files": False},
    }
    assert by_default.json()["agent_id"] == "atlas"
    assert by_default.json()["groups"] == EVERY_GROUP
