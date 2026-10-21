from support_orbit_mcp import serving

"""DEI-12 A3/A4: the catalogue is the engine's, and no engine is said out loud."""

import pytest
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    DEVICES,
    FakeHub,
    answered,
    hub_rejected,
    text_of,
)

pytestmark = pytest.mark.anyio

TOOLS = {
    "after_index": 0,
    "total": 2,
    "tools": [
        {
            "name": "launch_app",
            "description": "Abre una aplicación por su bundle id.",
            "input_schema": {"type": "object"},
        },
        {
            "name": "a_tool_invented_by_a_newer_engine",
            "description": "Una herramienta que Orbit no nombra en ningún sitio.",
            "input_schema": {"type": "object"},
        },
    ],
}


async def test_a4_the_catalogue_is_read_from_the_engine_not_from_a_list_in_orbit(tmp_path):
    hub = FakeHub(
        device_replies=[answered(TOOLS), answered({"tools": [], "after_index": 0, "total": 0})]
    )
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        listed = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
        empty = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert not listed.is_error
    # A tool Orbit has never named still reaches Deimos.
    assert "a_tool_invented_by_a_newer_engine" in text_of(listed)
    assert not empty.is_error
    assert "no declara ninguna herramienta" in text_of(empty).lower()
    assert [request["capability"] for request in hub.device_requests] == [
        "control_tools",
        "control_tools",
    ]
    assert all(request["device_id"] == DEVICES[DEFAULT_DEVICE] for request in hub.device_requests)


async def test_a4_perform_action_forwards_a_tool_orbit_does_not_know(tmp_path):
    receipt = {
        "receipt": {
            "tool": "a_tool_invented_by_a_newer_engine",
            "state": "performed",
            "operation_id": "synthetic-operation",
            "attempted": True,
            "driver_receipt": {},
            "retry": "no_repetir",
        }
    }
    hub = FakeHub(device_replies=[answered(receipt)])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool(
            "orbit_perform_action",
            {
                "device": DEFAULT_DEVICE,
                "action": {
                    "tool": "a_tool_invented_by_a_newer_engine",
                    "arguments": {"anything": 1},
                },
            },
        )
    finally:
        await context.__aexit__(None, None, None)

    assert not result.is_error
    assert "performed" in text_of(result)
    assert hub.device_requests[0]["capability"] == "perform_action"
    assert hub.device_requests[0]["parameters"] == {
        "action": {"tool": "a_tool_invented_by_a_newer_engine", "arguments": {"anything": 1}}
    }


@pytest.mark.parametrize(
    "action",
    [
        {"tool": "", "arguments": {}},
        {"tool": "tiene espacio", "arguments": {}},
        {"tool": "click", "arguments": ["no", "es", "objeto"]},
        {"tool": "type_text", "arguments": {"text": "x" * 20_000}},
    ],
)
async def test_a4_a_malformed_action_is_refused_by_shape_before_any_connection(tmp_path, action):
    hub = FakeHub()
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool(
            "orbit_perform_action", {"device": DEFAULT_DEVICE, "action": action}
        )
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    assert "invalid_parameters" in text_of(result)
    assert hub.device_list_requests == 0
    assert hub.device_requests == []


async def test_a3_without_an_engine_the_answer_names_the_cause(tmp_path):
    hub = FakeHub(device_replies=[hub_rejected(409, "engine_unavailable")])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool(
            "orbit_perform_action",
            {
                "device": DEFAULT_DEVICE,
                "action": {
                    "tool": "launch_app",
                    "arguments": {"bundle_id": "com.apple.Calculator"},
                },
            },
        )
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    assert "engine_unavailable" in text_of(result)
    assert "motor" in text_of(result).lower()


# REF-11 (b): «there is no helper» and «the helper has not said yet» are two
# causes with two codes. Only the second invites asking again in a moment; the
# first will not answer by itself however often Deimos retries.
