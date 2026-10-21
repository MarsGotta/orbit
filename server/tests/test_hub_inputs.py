"""Incoming Mac and Hermes fields remain typed at their trust boundaries."""

import pytest
from pydantic import ValidationError
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414
from support_review_regressions import runtime_fixture

import orbit_hub.assistant.hermes as hermes_module


def test_omitted_screenshot_filters_are_absent():
    from orbit_mcp.bridge import ScreenshotParameters

    parameters = ScreenshotParameters.model_validate({})
    assert parameters.application is None
    assert parameters.title is None
    assert parameters.model_dump() == {}


def test_parameter_models_cover_every_admitted_capability_and_reject_unknown_names():
    from orbit_mcp.bridge import PARAMETER_MODELS, RESULT_MODELS
    from orbit_mcp.hub import HubClientError, validate_parameters

    assert set(PARAMETER_MODELS) == set(RESULT_MODELS)
    with pytest.raises(HubClientError, match="unknown_capability"):
        validate_parameters("future_capability", {})


@pytest.mark.parametrize(
    "kind,payload,field",
    [
        (
            "clarify.request",
            {"request_id": "r1", "questions": [{"qid": 42, "question": "a"}]},
            "qid",
        ),
        ("clarify.request", {"request_id": "r1", "questions": [{"qid": "q1"}]}, "question"),
        ("clarify.request", {"request_id": "r1", "questions": ["unexpected"]}, "item"),
        ("clarify.expire", {}, "request_id"),
    ],
)
async def test_malformed_hermes_interaction_preserves_live_reader_and_snapshots(
    reviewed, monkeypatch, kind, payload, field
):
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    logged = []
    monkeypatch.setattr(hermes_module, "report", lambda *args, **kwargs: logged.append(kwargs))
    queue = runtime.subscribe()
    assert queue.get_nowait()["event"] == "state.snapshot"
    await runtime.connection.receive({"type": kind, "session_id": upstream, "payload": payload})
    assert runtime.connection.live_sessions == {upstream}
    assert runtime.store.session(runtime.device, session)["status"] == "streaming"
    assert runtime.subscribe().get_nowait()["event"] == "state.snapshot"
    await runtime.changed()
    assert queue.get_nowait()["event"] == (
        "clarify.required" if kind.endswith("request") else "state.snapshot"
    )
    assert any(
        item["event"] == "hermes_field_invalid"
        and item["code"].startswith(kind)
        and field in item["code"]
        for item in logged
    )
    await runtime.close()


@pytest.mark.parametrize("tool", ["á", "工具", "x" * 129])
def test_action_tool_name_uses_the_macs_ascii_byte_contract(tool):
    from orbit_mcp.hub import HubClientError
    from orbit_mcp.server import _general_action

    with pytest.raises(HubClientError) as error:
        _general_action({"tool": tool, "arguments": {}})
    assert error.value.code == "invalid_parameters"
    assert error.value.detail["refusal"]["bytes"] == len(tool.encode("utf-8"))
    assert error.value.detail["refusal"]["max_bytes"] == 128


async def test_mcp_names_invalid_mac_result_field_and_version(tmp_path):
    import json

    from fastapi.responses import Response
    from support_orbit_mcp import BRIDGE_RESULT, FakeHub, serving, text_of

    hub = FakeHub(
        device_replies=[
            Response(
                content=json.dumps(
                    {
                        "protocol": BRIDGE_RESULT,
                        "state": "answered",
                        "device_version": "0.7.0-test",
                        "result": {"windows": 7},
                    }
                ),
                media_type="application/json",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        reply = await server.call_tool("orbit_windows", {})
    assert reply.is_error
    text = text_of(reply)
    assert "device_reply_invalid" in text
    assert "result.windows" in text and "lista" in text and "0.7.0-test" in text


@pytest.mark.parametrize(
    "params",
    [
        {"refused": "Negativa antigua", "code": "busy"},
        {"result": {}, "refusal": {"code": "busy"}},
        {},
    ],
)
def test_hub_rejects_legacy_and_ambiguous_reply_envelopes(params):
    from orbit_hub.protocol import DeviceReplyParams

    with pytest.raises(ValidationError):
        DeviceReplyParams.model_validate({"request_id": "r", **params})


async def test_mcp_rejects_legacy_refusal_envelope(tmp_path):
    import json

    from fastapi.responses import Response
    from support_orbit_mcp import BRIDGE_RESULT, FakeHub, serving, text_of

    hub = FakeHub(
        device_replies=[
            Response(
                content=json.dumps(
                    {
                        "protocol": BRIDGE_RESULT,
                        "state": "refused",
                        "code": "busy",
                        "notice": "Negativa antigua",
                    }
                ),
                status_code=403,
                media_type="application/json",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_windows", {})
    assert result.is_error
    assert "invalid_response" in text_of(result)
    assert "Negativa antigua" not in text_of(result)


async def test_mcp_names_an_oversized_mac_cursor_before_constructing_the_next_request(tmp_path):
    import json
    from pathlib import Path

    from fastapi.responses import Response
    from support_orbit_mcp import BRIDGE_RESULT, FakeHub, serving, text_of

    fixture = (
        Path(__file__).resolve().parents[2]
        / "contracts/fixtures/bridge/replies/calendar_read-result.json"
    )
    reading = json.loads(fixture.read_text())["params"]["result"]
    reading["read_id"] = "ñ" * 65
    hub = FakeHub(
        device_replies=[
            Response(
                content=json.dumps(
                    {
                        "protocol": BRIDGE_RESULT,
                        "state": "answered",
                        "result": reading,
                        "device_version": "0.7.0-test",
                    }
                ),
                media_type="application/json",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar",
            {
                "start_ms": reading["requested"]["start_unix_ms"],
                "end_ms": reading["requested"]["end_unix_ms"],
            },
        )
    assert result.is_error
    text = text_of(result)
    assert "device_reply_invalid" in text and "result.read_id" in text
    assert "130" in text and "128" in text and "0.7.0-test" in text
    assert len(hub.device_requests) == 1
