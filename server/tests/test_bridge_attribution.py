"""Mac requests name the credential's agent, never a caller's chosen label."""

import asyncio
import copy
import json

import httpx
import pytest
from conftest import BRIDGE_KEY, answer_bridge_once, connect_bridge, device
from conftest import bridge_signed as signed
from test_agent_access import bridged_hub

from orbit_hub.bridge_route import PATH, PROTOCOL


@pytest.fixture
def attributed_hub(environment, tmp_path):
    app, store = bridged_hub(
        environment,
        tmp_path,
        tuple(
            {"id": agent_id, "name": name, "kind": "hermes", "profile": agent_id}
            for agent_id, name in (("atlas", "Atlas"), ("vega", "Vega"))
        ),
    )
    # Isolated fake vendor backends: only their generated bridge keys matter.
    for index, agent in enumerate(app.state.device_bridge.access.agents.by_id.values()):
        agent.kind = "codex"
        agent.backend = copy.copy(agent.backend)
        agent.backend.folder = tmp_path / agent.id
        folder = agent.backend.folder / "bridge"
        folder.mkdir(parents=True, mode=0o700)
        key = folder / "device.key"
        key.write_bytes(bytes([index + 1]) * 32)
        key.chmod(0o600)
    return app, store, environment[2], device(store)


@pytest.mark.parametrize("capability", ["screen_sources", "perform_action"])
async def test_each_request_carries_its_authenticated_agent(attributed_hub, capability):
    app, store, adapter, owner = attributed_hub
    async with app.router.lifespan_context(app):
        runtime = await connect_bridge(app, store, adapter, owner)
        queue = next(iter(runtime.listeners))
        service = app.state.device_bridge
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as http:
            for index, (agent_id, name) in enumerate((("atlas", "Atlas"), ("vega", "Vega"))):
                payload = json.dumps(
                    {
                        "protocol": PROTOCOL,
                        "device_id": owner["device_id"],
                        "capability": capability,
                        "parameters": {"action": {"tool": "launch_app", "arguments": {}}}
                        if capability == "perform_action"
                        else {},
                    }
                ).encode()
                mac = asyncio.create_task(answer_bridge_once(service, {"result": {}}))
                response = await http.post(
                    PATH,
                    content=payload,
                    headers=signed(
                        payload, key_id=f"agent-{agent_id}-device", key=bytes([index + 1]) * 32
                    ),
                )
                await mac
                assert response.status_code == 200, response.text
                frame = await asyncio.wait_for(queue.get(), 2)
                assert frame["event"] == "device.request"
                assert frame["data"]["agent"] == {"id": agent_id, "name": name}

            # The operator's unbound key keeps the configured default agent.
            payload = json.loads(payload)
            payload["capability"], payload["parameters"] = "screen_sources", {}
            encoded = json.dumps(payload).encode()
            mac = asyncio.create_task(answer_bridge_once(service, {"result": {}}))
            response = await http.post(
                PATH, content=encoded, headers=signed(encoded, key=BRIDGE_KEY)
            )
            await mac
            assert response.status_code == 200
            assert (await asyncio.wait_for(queue.get(), 2))["data"]["agent"] == {
                "id": "atlas",
                "name": "Atlas",
            }


@pytest.mark.parametrize(
    "claim",
    [{"agent_id": "vega"}, {"agent": {"id": "vega", "name": "Vega"}}, {"agent_name": "Vega"}],
)
async def test_a_credential_cannot_supply_another_agents_name(attributed_hub, claim):
    app, store, adapter, owner = attributed_hub
    async with app.router.lifespan_context(app):
        runtime = await connect_bridge(app, store, adapter, owner)
        payload = json.dumps(
            {
                "protocol": PROTOCOL,
                "device_id": owner["device_id"],
                "capability": "screen_sources",
                "parameters": {},
                **claim,
            }
        ).encode()
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as http:
            response = await http.post(
                PATH,
                content=payload,
                headers=signed(payload, key_id="agent-atlas-device", key=bytes([1]) * 32),
            )
        assert response.status_code == (403 if "agent_id" in claim else 400)
        assert all(queue.empty() for queue in runtime.listeners)
