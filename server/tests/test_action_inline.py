"""Fast device actions settle in the original signed request."""

import asyncio

import httpx
import pytest
from conftest import answer_bridge_once, bridge_body, bridge_signed, connect_bridge
from test_action_operations import action_result

from orbit_hub.bridge_route import PATH

pytestmark = pytest.mark.anyio


@pytest.mark.parametrize("outcome", ["performed", "failed", "unknown"])
async def test_fast_action_returns_the_same_result_as_later_lookup(bridged, outcome):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect_bridge(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = bridge_body(
            "perform_action",
            owner["device_id"],
            action={"tool": "fixture_action", "arguments": {}},
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            sending = asyncio.create_task(
                client.post(PATH, content=payload, headers=bridge_signed(payload))
            )
            reply = {
                "result": {
                    "receipt": {
                        "state": "performed",
                        "operation_id": "fixture-operation",
                        "tool": "fixture_action",
                        "attempted": True,
                        "driver_receipt": {"ok": True},
                        "retry": "no_repetir",
                    }
                }
            }
            if outcome == "failed":
                reply = {
                    "refusal": {
                        "code": "engine_unavailable",
                        "cause": "not_running",
                        "retry": {"kind": "after_fix"},
                    }
                }
            elif outcome == "unknown":
                reply = {"result": {}}
            entry = await answer_bridge_once(service, reply)
            response = await sending
            assert response.status_code == 200
            assert response.json()["outcome"] == outcome
            lookup = await action_result(client, owner, entry.request_id)
            assert lookup.json() == response.json()


async def test_inline_bound_is_configurable_and_keeps_the_receipt_worker(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect_bridge(app, store, adapter, owner)
        service = app.state.device_bridge
        service.configuration = service.configuration.model_copy(
            update={"action_inline_wait_seconds": 0.01}
        )
        payload = bridge_body(
            "perform_action", owner["device_id"], action={"tool": "fixture_action", "arguments": {}}
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=bridge_signed(payload))
            assert response.status_code == 202
            operation = response.json()["operation_id"]
            waiter = service.waiters[operation]
            assert not waiter.done()
            entry = await answer_bridge_once(
                service,
                {
                    "refusal": {
                        "code": "engine_unavailable",
                        "cause": "not_running",
                        "retry": {"kind": "after_fix"},
                    }
                },
            )
            assert entry.request_id == operation
            await asyncio.gather(*service._action_tasks)
            settled = await action_result(client, owner, operation)
            assert settled.json()["outcome"] == "failed"
            assert settled.json()["code"] == "engine_unavailable"


@pytest.mark.parametrize("value", [-1, 31, float("nan"), float("inf")])
def test_inline_wait_rejects_invalid_budgets(value):
    from pydantic import ValidationError

    from orbit_hub.bridge_route import BridgeConfiguration

    with pytest.raises(ValidationError):
        BridgeConfiguration(
            key_id="fixture", credential_file="/tmp/fixture-key", action_inline_wait_seconds=value
        )
