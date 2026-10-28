"""Bridge envelope correlation and capability validation at the MCP consumer."""

import asyncio
import json
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch
from uuid import uuid4

import pytest
from conftest import bridge_body, bridge_signed, device
from fastapi.testclient import TestClient

from orbit_hub.bridge_route import ACTION_RESULT_PATH, PATH
from orbit_hub.ws_replies import report
from orbit_mcp.hub import HubClientError, validate_mac_payload
from orbit_mcp.refusals import error_result


@pytest.mark.parametrize("capability", ["screen_sources", "perform_action"])
def test_bad_reply_is_relayed_and_mcp_reports_the_field(bridged, capability):
    app, store, _adapter, owner = bridged
    parameters = (
        {"action": {"tool": "synthetic_tool", "arguments": {}}}
        if capability == "perform_action"
        else {}
    )
    body = bridge_body(capability, owner["device_id"], **parameters)
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={
                "Authorization": "Bearer " + owner["token"],
                "X-Orbit-Device-Version": "0.7.0-test",
            },
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request_id = ws.receive_json()["data"]["request_id"]
            result = {"windows": 7}
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request_id, "result": result},
                }
            )
            ws.send_json({"v": 1, "id": str(uuid4()), "method": "ping", "params": {}})
            assert ws.receive_json()["event"] == "pong"
            response = pending.result(timeout=10)
            if capability == "perform_action":

                async def action_settled():
                    await asyncio.gather(*tuple(app.state.device_bridge._action_tasks))

                http.portal.call(action_settled)
                payload = store.operations.get(owner["device_id"], request_id)["response"]
                assert response.status_code == 200
                assert payload["outcome"] == "unknown"
                assert "Do not repeat" in payload["next_step"]
                lookup = json.dumps(
                    {
                        "protocol": "orbit_device_bridge_v1",
                        "device_id": owner["device_id"],
                        "operation_id": request_id,
                    },
                    separators=(",", ":"),
                ).encode()
                settled = http.post(
                    ACTION_RESULT_PATH,
                    content=lookup,
                    headers=bridge_signed(lookup, path=ACTION_RESULT_PATH),
                )
                assert settled.status_code == 200
                payload = settled.json()
            else:
                assert response.status_code == 200
                payload = response.json()
            assert payload["result"] == result
            assert request_id not in app.state.device_bridge.bridge.pending
            with pytest.raises(HubClientError) as error:
                validate_mac_payload(capability, payload)
            assert error.value.code == "device_reply_invalid"
            field = "result.receipt" if capability == "perform_action" else "result.windows"
            assert error.value.detail["refusal"]["field"] == field
            assert error.value.detail["refusal"]["retry"]["kind"] == (
                "check_outcome" if capability == "perform_action" else "after_fix"
            )
            translated = error_result(error.value).content[0].text
            assert field in translated and "0.7.0-test" in translated
            if capability == "screen_sources":
                assert "list" in translated


@pytest.mark.parametrize("bad_id", [None, 4, "", "r" * 161])
def test_unreadable_reply_id_does_not_claim_a_live_question(bridged, bad_id):
    app, _store, _adapter, owner = bridged
    body = bridge_body("screen_sources", owner["device_id"])
    with (
        patch("orbit_hub.ws_replies.report", wraps=report) as logged,
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request_id = ws.receive_json()["data"]["request_id"]
            invalid_params = {"result": {"windows": []}}
            if bad_id is not None:
                invalid_params["request_id"] = bad_id
            ws.send_json(
                {"v": 1, "id": str(uuid4()), "method": "device.reply", "params": invalid_params}
            )
            error = ws.receive_json()["data"]
            assert error["refusal"]["field"] == "request_id"
            assert "1 to 160" in error["refusal"]["expected"]
            assert request_id in app.state.device_bridge.bridge.pending
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request_id, "result": {"windows": []}},
                }
            )
            assert pending.result(timeout=10).status_code == 200
    assert any(
        call.kwargs.get("device") == owner["device_id"] and call.kwargs.get("field") == "request_id"
        for call in logged.call_args_list
    )


def test_result_valid_for_another_capability_is_refused_by_mcp():
    with pytest.raises(HubClientError) as error:
        validate_mac_payload(
            "screen_sources",
            {
                "result": {"application": "Fixture", "title": "Ventana", "text": "selección"},
                "device_version": "0.7.0-test",
            },
        )
    assert error.value.code == "device_reply_invalid"
    assert error.value.detail["refusal"]["field"] == "result.windows"


@pytest.mark.parametrize("intruder_result", [{"windows": []}, {"windows": 7}])
def test_other_mac_cannot_remove_the_owners_waiter(bridged, intruder_result):
    app, store, _adapter, owner = bridged
    intruder = device(store, "otro Mac")
    body = bridge_body("screen_sources", owner["device_id"])
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as owner_ws,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + intruder["token"]}
        ) as intruder_ws,
    ):
        assert owner_ws.receive_json()["event"] == "ready"
        assert intruder_ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request_id = owner_ws.receive_json()["data"]["request_id"]
            waiter = app.state.device_bridge.waiters[request_id]
            intruder_ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request_id, "result": intruder_result},
                }
            )
            intruder_ws.send_json({"v": 1, "id": str(uuid4()), "method": "ping", "params": {}})
            assert intruder_ws.receive_json()["event"] == "pong"
            intact = (
                request_id in app.state.device_bridge.waiters
                and request_id in app.state.device_bridge.bridge.pending
                and not waiter.done()
            )
            if intact:
                owner_ws.send_json(
                    {
                        "v": 1,
                        "id": str(uuid4()),
                        "method": "device.reply",
                        "params": {"request_id": request_id, "result": {"windows": []}},
                    }
                )
            else:
                http.portal.call(waiter.set_result, {"result": {"windows": []}})
            assert pending.result(timeout=10).status_code == 200
            assert intact, "the other Mac removed the owner's waiter"


def test_malformed_unknown_reply_id_still_gets_a_field_refusal(bridged):
    app, _store, _adapter, owner = bridged
    body = bridge_body("screen_sources", owner["device_id"])
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request_id = ws.receive_json()["data"]["request_id"]
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": str(uuid4()), "result": 7},
                }
            )
            ws.send_json({"v": 1, "id": str(uuid4()), "method": "ping", "params": {}})
            event = ws.receive_json()
            intact = (
                request_id in app.state.device_bridge.bridge.pending
                and request_id in app.state.device_bridge.waiters
            )
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request_id, "result": {"windows": []}},
                }
            )
            assert pending.result(timeout=10).status_code == 200
            assert intact
            assert event["event"] == "error"
            assert event["data"]["code"] == "invalid_request"
            assert event["data"]["refusal"]["field"] == "device.reply"
            assert ws.receive_json()["event"] == "pong"


def test_missing_mac_version_is_named_without_broken_grammar():
    with pytest.raises(HubClientError) as error:
        validate_mac_payload("screen_sources", {"result": {"windows": 7}})
    assert "Mac version: not reported" in error_result(error.value).content[0].text


async def test_unanswered_action_expires_with_injected_deadline_and_frees_slot(bridged):
    import httpx
    from conftest import connect_bridge

    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect_bridge(app, store, adapter, owner)
        service = app.state.device_bridge
        clock = [0.0]
        service.monotonic = lambda: clock[0]
        reached_deadline = asyncio.Event()

        async def deadline(_waiter, _seconds):
            assert _seconds == 30.0
            clock[0] = 31.0
            reached_deadline.set()
            raise TimeoutError

        service.wait_for_reply = deadline
        payload = bridge_body(
            "perform_action",
            owner["device_id"],
            action={"tool": "synthetic_tool", "arguments": {}},
        )
        with patch.object(service.bridge, "expire", wraps=service.bridge.expire) as expired:
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app), base_url="https://localhost"
            ) as client:
                response = await client.post(PATH, content=payload, headers=bridge_signed(payload))
                operation_id = response.json()["operation_id"]
                for _ in range(4):
                    await asyncio.sleep(0)
                assert reached_deadline.is_set()
                outcome = store.operations.get(owner["device_id"], operation_id)["response"]
                assert outcome["outcome"] == "unknown"
                assert outcome["code"] == "device_reply_timeout"
                assert outcome["deadline_ms"] == 30_000
                assert outcome["refusal"]["code"] == "device_reply_timeout"
                assert outcome["refusal"]["deadline_ms"] == 30_000
                assert outcome["refusal"]["expected"] == "perform_action response"
                result_body = json.dumps(
                    {
                        "protocol": "orbit_device_bridge_v1",
                        "device_id": owner["device_id"],
                        "operation_id": operation_id,
                    },
                    separators=(",", ":"),
                ).encode()
                result = await client.post(
                    ACTION_RESULT_PATH,
                    content=result_body,
                    headers=bridge_signed(result_body, path=ACTION_RESULT_PATH),
                )
                assert result.status_code == 200
                assert result.json()["refusal"]["deadline_ms"] == 30_000
                translated = (
                    error_result(
                        HubClientError(
                            "device_reply_timeout",
                            detail={"refusal": result.json()["refusal"]},
                        )
                    )
                    .content[0]
                    .text
                )
                assert "30000" in translated
                assert "Do not repeat" in outcome["next_step"]
                assert operation_id not in service.bridge.pending
                expired.assert_called_once_with(31.0, operation_id)


def test_hub_relays_unknown_parameter_and_result_fields_unchanged(bridged):
    app, _store, _adapter, owner = bridged
    parameters = {"future_filter": {"keep": None}}
    body = bridge_body("screen_sources", owner["device_id"], **parameters)
    result = {"windows": [], "future_result": {"keep": None}}
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={
                "Authorization": "Bearer " + owner["token"],
                "X-Orbit-Device-Version": "0.7.0-test",
            },
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request = ws.receive_json()["data"]
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request["request_id"], "result": result},
                }
            )
            response = pending.result(timeout=10)
            assert request["parameters"] == parameters
            assert response.json()["result"] == result
            assert response.json()["device_version"] == "0.7.0-test"


def test_malformed_reply_envelope_resolves_its_owner_instead_of_timing_out(bridged):
    app, _store, _adapter, owner = bridged
    body = bridge_body("screen_sources", owner["device_id"])
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={
                "Authorization": "Bearer " + owner["token"],
            },
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request_id = ws.receive_json()["data"]["request_id"]
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request_id, "result": []},
                }
            )
            assert ws.receive_json()["data"]["code"] == "invalid_request"
            response = pending.result(timeout=10)
            assert response.status_code == 403
            assert response.json()["refusal"]["code"] == "device_reply_invalid"
            assert request_id not in app.state.device_bridge.bridge.pending
            assert request_id not in app.state.device_bridge.waiters
