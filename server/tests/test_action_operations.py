"""Native engine actions leave durable, queryable outcomes."""

import asyncio
import json
from uuid import uuid4

import httpx
import pytest
from conftest import answer_bridge_once as answer_once
from conftest import bridge_body as body
from conftest import bridge_signed as signed
from conftest import connect_bridge as connect

from orbit_hub.bridge_outcomes import action_outcome
from orbit_hub.bridge_route import PATH
from orbit_mcp.hub import HubClientError
from orbit_mcp.refusals import error_result
from orbit_mcp.tools.control import general_action

pytestmark = pytest.mark.anyio

ACTION_RESULT_PATH = "/internal/v1/device/action-result"


def test_translator_action_boundary_carries_size_and_ceiling():
    def action_of_size(size):
        action = {"tool": "x", "arguments": {"data": ""}}
        empty = len(json.dumps(action, separators=(",", ":")).encode())
        action["arguments"]["data"] = "x" * (size - empty)
        assert len(json.dumps(action, separators=(",", ":")).encode()) == size
        return action

    assert general_action(action_of_size(16_384))
    with pytest.raises(HubClientError) as refusal:
        general_action(action_of_size(16_385))
    text = error_result(refusal.value).content[0].text
    assert "16385" in text and "16384" in text


def result_body(device_id: str, operation_id: str) -> bytes:
    return json.dumps(
        {
            "protocol": "orbit_device_bridge_v1",
            "device_id": device_id,
            "operation_id": operation_id,
        },
        separators=(",", ":"),
    ).encode()


async def action_result(client, owner, operation_id):
    payload = result_body(owner["device_id"], operation_id)
    return await client.post(
        ACTION_RESULT_PATH,
        content=payload,
        headers=signed(payload, path=ACTION_RESULT_PATH),
    )


async def test_perform_action_returns_a_durable_pending_operation_immediately(bridged, monkeypatch):
    import orbit_hub.bridge_route as module

    monkeypatch.setattr(module, "REQUEST_TIMEOUT_SECONDS", 0.05)
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body(
            "perform_action",
            owner["device_id"],
            action={"tool": "fixture_action", "arguments": {}},
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=signed(payload))
            assert response.status_code == 202
            pending = response.json()
            assert pending["state"] == "pending"
            # This state was renamed: the Mac has the action and it has
            # not finished. Nobody is being asked to permit anything.
            assert pending["outcome"] == "in_progress"
            operation_id = pending["operation_id"]
            stored = store.operations.get(owner["device_id"], operation_id)
            assert stored == {"status": "pending", "response": None}

            entry = await answer_once(
                service,
                {
                    "result": {
                        "receipt": {
                            "state": "performed",
                            "operation_id": operation_id,
                            "tool": "fixture_action",
                            "attempted": True,
                            "driver_receipt": {"ok": True},
                            "retry": "no_repetir",
                        }
                    }
                },
            )
            assert entry.request_id == operation_id
            for _ in range(100):
                settled = await action_result(client, owner, operation_id)
                if settled.json().get("outcome") == "performed":
                    break
                await asyncio.sleep(0.01)
            assert settled.status_code == 200
            assert settled.json()["receipt"]["state"] == "performed"


async def test_engine_refusal_is_actionable_and_never_no_answer(bridged, monkeypatch):
    import orbit_hub.bridge_route as module

    monkeypatch.setattr(module, "REQUEST_TIMEOUT_SECONDS", 0.05)
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body(
            "perform_action",
            owner["device_id"],
            action={"tool": "fixture_action", "arguments": {}},
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            pending = await client.post(PATH, content=payload, headers=signed(payload))
            operation_id = pending.json()["operation_id"]
            await answer_once(
                service,
                {
                    "refusal": {
                        "code": "engine_unavailable",
                        "cause": "not_running",
                        "detail": "El motor todavía no está conectado en este Mac",
                        "retry": {"kind": "after_fix"},
                    },
                },
            )
            for _ in range(100):
                settled = await action_result(client, owner, operation_id)
                if settled.json().get("outcome") == "failed":
                    break
                await asyncio.sleep(0.01)

    result = settled.json()
    assert result["outcome"] == "failed"
    assert result["code"] == "engine_unavailable"
    assert result["refusal"]["detail"] == "El motor todavía no está conectado en este Mac"
    assert result["refusal"]["code"] == "engine_unavailable"
    assert result["refusal"]["cause"] == "not_running"
    # It used to require "Ajustes" here, which pinned the pointer to
    # «Ajustes › Aplicaciones», a section Orbit does not have. The actionable
    # part is the typed cause and its detail, with a next step that is true.
    assert "Check the cause" in result["next_step"]
    assert "Aplicaciones" not in result["next_step"]
    assert "no_answer" not in json.dumps(result)


async def test_orphaned_pending_action_becomes_unknown_instead_of_retryable(bridged):
    app, store, _adapter, owner = bridged
    operation_id = str(uuid4())
    store.operations.reserve(
        owner["device_id"],
        operation_id,
        {
            "protocol": "orbit_device_bridge_v1",
            "device_id": owner["device_id"],
            "capability": "perform_action",
            "parameters": {"action": {"tool": "fixture_action", "arguments": {}}},
        },
    )

    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client,
    ):
        response = await action_result(client, owner, operation_id)

    assert response.status_code == 200
    assert response.json()["outcome"] == "unknown"
    assert response.json()["code"] == "receipt_lost"
    assert response.json()["refusal"]["code"] == "receipt_lost"
    assert response.json()["refusal"]["retry"] == {"kind": "check_outcome"}
    assert "Do not repeat" in response.json()["next_step"]
    assert store.operations.get(owner["device_id"], operation_id)["status"] == "completed"


def test_connection_lost_after_publish_is_unknown_and_forbids_retry():
    outcome = action_outcome(
        {
            "refusal": {
                "code": "mac_disconnected",
                "last_seen_ms": None,
                "retry": {"kind": "check_outcome"},
            },
        }
    )

    assert outcome["outcome"] == "unknown"
    assert outcome["code"] == "mac_disconnected"
    assert "Do not repeat" in outcome["next_step"]


@pytest.mark.parametrize(
    "outcome,code",
    [("expired", "consent_expired"), ("rejected", "mars_rejected"), ("failed", "legacy")],
)
async def test_stored_action_receipts_remain_readable_after_upgrade(bridged, outcome, code):
    from conftest import BRIDGE_KEY, BRIDGE_KEY_ID, BRIDGE_NOW_MS

    from orbit_mcp.config import Config, Credentials
    from orbit_mcp.hub import HubClient
    from orbit_mcp.server import build_server

    app, store, _adapter, owner = bridged
    operation = str(uuid4())
    receipt = {"state": outcome, "operation_id": operation, "code": code}
    store.operations.reserve(
        owner["device_id"],
        operation,
        json.loads(
            body(
                "perform_action",
                owner["device_id"],
                action={"tool": "fixture_action", "arguments": {}},
            )
        ),
    )
    saved = {"outcome": outcome, "code": code, "receipt": receipt}
    if code == "legacy":
        saved["refusal"] = {
            "code": "legacy",
            "text": "calendar_invalid_event: fila 2, campo calendar.calendar_id, tamaño 700001, máximo 700000",
            "retry": {"kind": "never"},
        }
    store.operations.finish(owner["device_id"], operation, saved)
    config = Config(
        "https://localhost",
        Credentials(BRIDGE_KEY_ID, BRIDGE_KEY),
        None,
        "fixture",
        {"fixture": owner["device_id"]},
    )
    hub = HubClient(
        config,
        clock=lambda: BRIDGE_NOW_MS,
        client_factory=lambda **kwargs: httpx.AsyncClient(
            transport=httpx.ASGITransport(app), **kwargs
        ),
    )
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client,
    ):
        response = await action_result(client, owner, operation)
        assert response.status_code == 200, response.text
        assert response.json()["outcome"] == outcome
        assert response.json()["receipt"] == receipt
        translated = await hub.action_result(owner["device_id"], operation)
        if code == "legacy":
            result = await build_server(config, hub).call_tool(
                "orbit_action_result", {"device": "fixture", "operation_id": operation}
            )
            assert not result.is_error
            spoken = json.loads(result.content[0].text)["descripcion"]
            assert "calendar_invalid_event" in spoken and "calendar.calendar_id" in spoken
            assert "700001" in spoken and "700000" in spoken
    assert translated["outcome"] == outcome and translated["receipt"] == receipt
    assert store.operations.get(owner["device_id"], operation)["response"]["receipt"] == receipt
