"""A refusal keeps its cause across the Mac, hub and MCP boundary."""

import asyncio
import json
import re
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from uuid import UUID, uuid4

import httpx
import pytest
from conftest import (
    BRIDGE_KEY_ID,
    BRIDGE_NOW_MS,
    answer_bridge_once,
    bridge_body,
    bridge_signed,
    connect_bridge,
)
from fastapi.testclient import TestClient
from support_orbit_mcp import text_of as _text
from support_orbit_mcp import write_mcp_config

from orbit_hub.bridge_route import ACTION_RESULT_PATH, PATH, PROTOCOL
from orbit_hub.protocol import DeviceReplyParams
from orbit_mcp.calendar import render
from orbit_mcp.config import Config
from orbit_mcp.hub import HubClient, HubClientError
from orbit_mcp.refusals import error_result, unexpected_result
from orbit_mcp.server import build_server


def _through_websocket(app, owner, refusal, capability="screen_sources"):
    body = bridge_body(capability, owner["device_id"])
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={"Authorization": "Bearer " + owner["token"]},
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request = ws.receive_json()
            assert request["event"] == "device.request"
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request["data"]["request_id"], "refusal": refusal},
                }
            )
            return pending.result(timeout=10)


def _real_mcp(http, owner, tmp_path):
    config_path = write_mcp_config(
        tmp_path,
        "https://localhost",
        default_device="fixture",
        devices={"fixture": owner["device_id"]},
    )
    config_path.write_text(
        config_path.read_text().replace('key_id = "mcp-bridge-key"', f'key_id = "{BRIDGE_KEY_ID}"')
    )
    config = Config.load(config_path)

    def loopback(request):
        response = http.request(
            request.method, str(request.url), content=request.content, headers=request.headers
        )
        return httpx.Response(
            response.status_code, content=response.content, headers=response.headers
        )

    hub_client = HubClient(
        config,
        client_factory=lambda **kwargs: httpx.AsyncClient(
            transport=httpx.MockTransport(loopback), **kwargs
        ),
        clock=lambda: BRIDGE_NOW_MS,
    )
    return build_server(config, hub_client)


def test_typed_refusal_crosses_the_actual_websocket(bridged):
    app, _store, _adapter, owner = bridged
    refusal = {
        "code": "too_large",
        "resource": "calendar",
        "bytes": 120000,
        "max_bytes": 98304,
        "retry": {"kind": "after_fix"},
    }
    response = _through_websocket(app, owner, refusal)
    assert response.status_code == 403, response.json()
    assert response.json()["refusal"] == refusal
    from orbit_mcp.hub import _failure

    translated = _text(error_result(_failure(response.json(), response.status_code)))
    assert "120000" in translated and "98304" in translated


def test_capture_capacity_counts_reach_the_mcp_reader():
    refusal = {
        "code": "too_many",
        "resource": "capture",
        "count": 5,
        "max": 4,
        "retry": {"kind": "after_fix"},
    }
    rendered = _text(error_result(HubClientError("too_many", detail={"refusal": refusal})))
    assert "5" in rendered and "4" in rendered
    assert "300" in rendered and "screenshots" in rendered
    assert "request contains more items" not in rendered


def test_upstream_http_429_keeps_its_service_without_guessing_capture_capacity():
    error = HubClientError(
        "upstream_failed",
        detail={
            "refusal": {
                "code": "upstream_failed",
                "service": "hub",
                "status": 429,
                "retry": {"kind": "after_fix"},
            }
        },
    )
    rendered = _text(error_result(error))
    assert "The service this operation depends on failed." in rendered
    assert "service: hub" in rendered and "status: 429" in rendered
    assert "screenshots" not in rendered and "upload" not in rendered


def test_uncertain_action_stays_uncertain_through_websocket_and_result_route(
    bridged, monkeypatch, tmp_path
):
    from orbit_hub import bridge_route

    frame = json.loads(
        (
            Path(__file__).resolve().parents[2]
            / "contracts/fixtures/active-action-outcome-unknown-reply.json"
        ).read_text()
    )
    monkeypatch.setattr(bridge_route.uuid, "uuid4", lambda: UUID(frame["id"]))
    app, store, _adapter, owner = bridged
    completed = threading.Event()
    finish = store.operations.finish

    def record_finish(*args):
        finish(*args)
        completed.set()

    monkeypatch.setattr(store.operations, "finish", record_finish)
    body = bridge_body(
        "perform_action",
        owner["device_id"],
        action={
            "tool": "click",
            "arguments": {"x": 1, "y": 1},
        },
    )
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={"Authorization": "Bearer " + owner["token"]},
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(http.post, PATH, content=body, headers=bridge_signed(body))
            request = ws.receive_json()
            assert request["event"] == "device.request"
            operation_id = request["data"]["request_id"]
            assert operation_id == frame["params"]["request_id"]
            accepted = pending.result(timeout=10)
            assert accepted.status_code == 202
            ws.send_json(frame)
            assert completed.wait(5), "the hub did not settle the action's outcome"
            query = json.dumps(
                {
                    "protocol": PROTOCOL,
                    "device_id": owner["device_id"],
                    "operation_id": operation_id,
                }
            ).encode()
            result = http.post(
                ACTION_RESULT_PATH,
                content=query,
                headers=bridge_signed(query, path=ACTION_RESULT_PATH),
            ).json()
            tool = _real_mcp(http, owner, tmp_path)
            rendered = asyncio.run(
                tool.call_tool(
                    "orbit_action_result", {"device": "fixture", "operation_id": operation_id}
                )
            )
    assert result["outcome"] == "unknown", result
    assert result["refusal"]["retry"]["kind"] == "check_outcome"
    assert "Do not repeat" in result["next_step"]
    assert '"outcome": "unknown"' in _text(rendered)
    assert "Do not repeat" in _text(rendered)


@pytest.mark.parametrize(
    "tool_name, arguments, refusal, expected",
    [
        (
            "orbit_calendar",
            {"start_ms": 1789000000000, "end_ms": 1789086400000},
            {"code": "permission_denied", "permission": "calendar", "retry": {"kind": "after_fix"}},
            ("agenda is empty", "System Settings"),
        ),
        (
            "orbit_calendar",
            {"start_ms": 1789000000000, "end_ms": 1789086400000},
            {
                "code": "permission_unknown",
                "permission": "calendar",
                "retry": {"kind": "after_fix"},
            },
            ("agenda is empty", "This is not a denial"),
        ),
        (
            "orbit_windows",
            {},
            {
                "code": "permission_denied",
                "permission": "screen_recording",
                "retry": {"kind": "after_fix"},
            },
            ("Screen Recording", "System Settings"),
        ),
    ],
)
def test_typed_permission_reaches_the_actual_mcp_tool(
    bridged,
    tmp_path,
    tool_name,
    arguments,
    refusal,
    expected,
):
    app, _store, _adapter, owner = bridged
    with (
        TestClient(app, base_url="https://localhost") as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={"Authorization": "Bearer " + owner["token"]},
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        tool = _real_mcp(http, owner, tmp_path)
        with ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(
                lambda: asyncio.run(tool.call_tool(tool_name, {"device": "fixture", **arguments}))
            )
            request = ws.receive_json()
            assert request["event"] == "device.request"
            ws.send_json(
                {
                    "v": 1,
                    "id": str(uuid4()),
                    "method": "device.reply",
                    "params": {"request_id": request["data"]["request_id"], "refusal": refusal},
                }
            )
            rendered = _text(pending.result(timeout=15))
    assert all(phrase in rendered for phrase in expected), rendered


@pytest.mark.parametrize(
    "permission, code, expected",
    [
        ("calendar", "permission_denied", ("agenda is empty", "System Settings")),
        ("calendar", "permission_unknown", ("agenda is empty", "This is not a denial")),
        ("screen_recording", "permission_denied", ("Screen Recording", "System Settings")),
        ("accessibility", "permission_denied", ("Accessibility", "System Settings")),
        ("accessibility", "permission_not_granted", ("Accessibility", "System Settings")),
        (
            "screen_recording",
            "permission_not_granted",
            ("Screen Recording", "System Settings"),
        ),
        ("calendar", "permission_not_granted", ("Calendars", "System Settings")),
        ("screen_and_accessibility", "permission_denied", ("Accessibility", "System Settings")),
        ("screen_recording", "permission_unknown", ("Screen Recording", "This is not a denial")),
        ("accessibility", "permission_unknown", ("Accessibility", "This is not a denial")),
        (
            "screen_and_accessibility",
            "permission_unknown",
            ("Screen Recording", "Accessibility", "This is not a denial"),
        ),
    ],
)
def test_typed_permission_from_mac_keeps_specific_guidance(permission, code, expected):
    refusal = {"code": code, "permission": permission, "retry": {"kind": "after_fix"}}
    rendered = _text(error_result(HubClientError(code, detail={"refusal": refusal})))
    assert all(phrase in rendered for phrase in expected), rendered


@pytest.mark.parametrize(
    "cause, expected",
    [
        ("unsupported", "does not support"),
        ("not_installed", "installed"),
        ("not_running", "running"),
        ("unhealthy", "responding"),
        ("voice_processing_unsupported", "voice"),
        ("initialization_failed", "start"),
    ],
)
def test_engine_cause_has_true_guidance_at_the_mcp_boundary(cause, expected):
    refusal = {"code": "engine_unavailable", "cause": cause, "retry": {"kind": "after_fix"}}
    rendered = _text(
        error_result(HubClientError("engine_unavailable", detail={"refusal": refusal}))
    )
    assert expected in rendered, rendered
    if cause == "unsupported":
        assert "starting it" not in rendered


@pytest.mark.parametrize(
    "refusal",
    [
        {"code": "permission_denied", "permission": ["calendar"], "retry": {"kind": "after_fix"}},
        {"code": "engine_unavailable", "cause": {}, "retry": {"kind": "after_fix"}},
    ],
)
def test_a_malformed_typed_field_keeps_the_code_message(refusal):
    """Malformed optional fields use the code's message without raising."""
    rendered = _text(error_result(HubClientError(refusal["code"], detail={"refusal": refusal})))
    assert rendered, rendered
    assert "does not recognize the code" not in rendered, rendered


def test_device_reply_accepts_a_typed_refusal():
    refusal = {
        "code": "too_large",
        "resource": "calendar",
        "bytes": 120000,
        "max_bytes": 98304,
        "retry": {"kind": "after_fix"},
    }
    reply = DeviceReplyParams.model_validate({"request_id": "ask", "refusal": refusal})
    assert reply.refusal == refusal
    with pytest.raises(ValueError):
        DeviceReplyParams.model_validate(
            {"request_id": "ask", "result": {"windows": []}, "refusal": refusal}
        )


async def test_typed_refusal_crosses_the_private_route(bridged):
    app, store, adapter, owner = bridged
    refusal = {
        "code": "too_large",
        "resource": "calendar",
        "bytes": 120000,
        "max_bytes": 98304,
        "retry": {"kind": "after_fix"},
    }
    async with app.router.lifespan_context(app):
        await connect_bridge(app, store, adapter, owner)
        service = app.state.device_bridge
        body = bridge_body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(answer_bridge_once(service, {"refusal": refusal}))
            response = await client.post(PATH, content=body, headers=bridge_signed(body))
            await mac
    assert response.status_code == 403
    assert response.json()["refusal"] == refusal
    from orbit_mcp.hub import _failure

    translated = _text(error_result(_failure(response.json(), response.status_code)))
    assert "120000" in translated and "98304" in translated


@pytest.mark.parametrize(
    ("code", "fields", "expected"),
    [
        ("work_in_flight", {"in_flight": 1, "max": 1}, ("in flight: 1", "maximum: 1")),
        ("physical_work_finishing", {"in_flight": 1, "max": 1}, ("in flight: 1", "maximum: 1")),
        (
            "work_expired",
            {"operation": "lectura", "started_generation": 4, "current_generation": 5},
            ("initial generation: 4", "current generation: 5"),
        ),
    ],
)
def test_new_work_refusals_reach_deimos_with_numbers(code, fields, expected):
    refusal = {"code": code, "retry": {"kind": "check_outcome"}, **fields}
    spoken = _text(error_result(HubClientError(code, detail={"refusal": refusal})))
    assert all(part in spoken for part in expected), spoken
    assert "The translator does not know" not in spoken


def test_eventkit_unavailable_names_the_backend_in_deimos():
    message = _text(error_result(HubClientError("calendar_backend_unavailable", 403)))
    assert "EventKit" in message
    assert "Orbit is open" not in message
    assert "agenda is empty" in message


def test_invalid_row_names_rows_in_this_page_not_all_conversations():
    refusal = {
        "code": "hub_session_invalid_row",
        "position": 7,
        "rows": 128,
        "field": "id",
        "bytes": 122,
        "max_bytes": 121,
        "retry": {"kind": "after_fix"},
    }
    message = _text(error_result(HubClientError(refusal["code"], detail={"refusal": refusal})))
    assert "rows on this page: 128" in message
    assert "total" not in message


def test_numbers_reach_deimos_without_a_generic_fallback():
    refusal = {
        "code": "too_large",
        "resource": "calendar",
        "bytes": 120000,
        "max_bytes": 98304,
        "retry": {"kind": "after_fix"},
    }
    message = _text(error_result(HubClientError("too_large", 403, {"refusal": refusal})))
    assert "120000" in message and "98304" in message and "calendar" in message
    assert "Orbit refused the request" not in message
    unknown = _text(error_result(HubClientError("future_code", 403)))
    assert "future_code" in unknown and "Orbit refused the request" not in unknown


def test_empty_calendar_render_and_internal_error_keep_their_cause():
    reading = {
        "requested": {"start_unix_ms": 1789000000000, "end_unix_ms": 1789086400000},
        "events_returned": 0,
        "events": [],
        "system_timezone": "Europe/Madrid",
    }
    text = render(reading)
    assert "0 events" in text
    assert not re.search(r"\bsize\b", text)
    internal = _text(unexpected_result(None, 0))
    assert internal.startswith("internal_error:")
    assert "too_large" not in internal


@pytest.mark.parametrize(
    "refusal, words, absent_words",
    [
        (
            {"code": "permission_denied", "permission": "accessibility"},
            ("Accessibility", "System Settings"),
            ("Screen Recording",),
        ),
        (
            {"code": "permission_denied", "permission": "screen_recording"},
            ("Screen Recording", "System Settings"),
            (),
        ),
        (
            {"code": "permission_unknown", "permission": "accessibility"},
            ("Accessibility", "This is not a denial", "helper"),
            ("has not granted",),
        ),
        (
            {"code": "helper_not_running"},
            ("helper",),
            ("seconds", "has not granted", "just started"),
        ),
        (
            {"code": "recovering", "step": "snapshot", "elapsed_ms": 1234},
            ("snapshot", "1234", "command was not applied"),
            (),
        ),
        ({"code": "work_in_flight", "in_flight": 1, "max": 1}, ("1",), ("receipt",)),
        ({"code": "physical_work_finishing", "in_flight": 1, "max": 1}, ("1",), ("receipt",)),
        ({"code": "busy", "operation": "chat"}, ("turn is in progress", "interrupt it"), ()),
        ({"code": "busy", "operation": "audio"}, ("voice note is buffered", "finish"), ()),
        (
            {"code": "busy", "operation": "voice_transcription"},
            ("transcription", "wait or interrupt it"),
            (),
        ),
        (
            {
                "code": "pending_recording",
                "id": "pending-307",
                "title": "Reunión pendiente",
                "phase": "interrupted",
                "sealed_chunks": 2,
                "gaps": 1,
                "expires_at_ms": 61000,
            },
            ("pending-307", "Reunión pendiente", "61000", "2", "1"),
            (),
        ),
        ({"code": "busy", "operation": "audio_session"}, ("Finish PTT",), ()),
        ({"code": "busy", "operation": "audio_drain"}, ("draining",), ()),
        (
            {"code": "engine_unavailable", "cause": "voice_processing_unsupported"},
            ("VoiceProcessing",),
            (),
        ),
        (
            {
                "code": "invalid_parameter",
                "parameter": "request",
                "expected": {"kind": "open_recording"},
            },
            ("Start another recording",),
            (),
        ),
        (
            {
                "code": "invalid_parameter",
                "parameter": "title",
                "expected": {"kind": "title_length", "characters": 257, "max_characters": 256},
            },
            ("title", "257", "256"),
            (),
        ),
        (
            {
                "code": "invalid_parameter",
                "parameter": "request",
                "expected": {"kind": "desktop_frame"},
            },
            ("full_screen=true",),
            (),
        ),
        (
            {"code": "too_large", "resource": "ipc_response", "bytes": 123, "max_bytes": 100},
            ("123", "100", "may have been applied"),
            (),
        ),
        (
            {"code": "helper_refused", "helper_code": "selected_text_not_exposed"},
            ("does not expose", "whether text is selected is unknown"),
            ("no text is selected",),
        ),
    ],
)
def test_refusal_recovery_words_live_at_the_mcp_boundary(refusal, words, absent_words):
    refusal = {**refusal, "retry": {"kind": "after_fix"}}
    rendered = _text(error_result(HubClientError(refusal["code"], detail={"refusal": refusal})))
    for word in words:
        assert word in rendered, rendered
    for word in absent_words:
        assert word not in rendered, rendered


@pytest.mark.parametrize(
    "refusal, words",
    [
        ({"code": "encode_failed", "resource": "calendar"}, ("encode", "calendar")),
        (
            {
                "code": "helper_reply_invalid",
                "field": "text",
                "expected": {"kind": "wire_type", "name": "native_v1/selected_text"},
                "received_type": "number",
            },
            ("text", "number", "native_v1/selected_text"),
        ),
        (
            {
                "code": "read_interrupted",
                "cause": {
                    "kind": "generation_changed",
                    "started_generation": 12,
                    "current_generation": 13,
                },
            },
            ("generation_changed", "12", "13"),
        ),
        (
            {"code": "helper_refused", "helper_code": "selected_text_secure_field"},
            ("protected field",),
        ),
        ({"code": "internal", "cause": {"kind": "native_command"}}, ("native_command",)),
        ({"code": "protocol_version", "received": 7, "required": 1}, ("7", "1", "protocol")),
        ({"code": "hub_reply_invalid", "field": "body", "received_bytes": 4312}, ("4312", "body")),
        (
            {
                "code": "daemon_reply_invalid",
                "field": "calendar.events",
                "issue": {"kind": "count_mismatch", "delivered": 3, "total": 9},
            },
            ("3", "9", "calendar.events"),
        ),
        (
            {"code": "calendar_read_limit", "cause": {"kind": "page", "max_bytes": 64000}},
            ("64000", "calendar"),
        ),
        (
            {
                "code": "storage_failed",
                "operation": "privacy_stop",
                "cause": {
                    "error": "synthetic_write_failure",
                    "step": "snapshot",
                    "elapsed_ms": 1234,
                },
            },
            ("privacy_stop", "snapshot", "1234", "synthetic_write_failure"),
        ),
    ],
)
def test_new_typed_refusals_keep_their_facts_at_the_mcp_boundary(refusal, words):
    refusal = {**refusal, "retry": {"kind": "after_fix"}}
    rendered = _text(error_result(HubClientError(refusal["code"], detail={"refusal": refusal})))
    for word in words:
        assert word in rendered, rendered
