from support_orbit_mcp import serving

"""DEI-16 A1/A2/A6/A7: an action in flight is not an action awaiting consent.

The bridge answers `state: pending` the moment it hands an action to the Mac.
That means the action is on its way, not that anybody has been asked to approve
it — Alina granted Orbit the use of her computer once, in macOS. Naming that
state «awaiting_approval» made the assistant tell her the page had been left
pending her approval when it was already open.

No Mac, no Hermes and no external network: the hub routes here are the real
ones, verified by the real private authenticator.
"""

import json
import re
import time
from time import monotonic

import httpx
import pytest
from conftest import bridge_body as body
from conftest import bridge_signed as signed
from conftest import connect_bridge as connect
from fastapi import Request
from fastapi.responses import Response
from support_orbit_mcp import (
    BRIDGE_KEY,
    BRIDGE_KEY_ID,
    DEFAULT_DEVICE,
    DEVICES,
    FakeHub,
    answered,
    text_of,
)

from orbit_hub.bridge_route import ACTION_OUTCOMES, ACTION_RESULT_PATH
from orbit_hub.bridge_route import HEADER_PREFIX as BRIDGE_HEADER_PREFIX
from orbit_hub.bridge_route import PATH as BRIDGE_PATH
from orbit_hub.bridge_route import PROTOCOL as BRIDGE_PROTOCOL
from orbit_hub.bridge_route import RESULT_PROTOCOL as BRIDGE_RESULT
from orbit_hub.private_auth import Authenticator

pytestmark = pytest.mark.anyio

ADAPTER_ID = "00000000-0000-4000-8000-000000000501"
EFFECT_ID = "00000000-0000-4000-8000-000000000701"
OPERATION_ID = "00000000-0000-4000-8000-0000000016a1"

# Any word Mars could read as «somebody has to say yes to this». An action in
# flight must not carry one.
APPROVAL_WORDS = re.compile(r"aprob|approval|autoriz|consent|permis", re.IGNORECASE)


class RouteFailure(Exception):
    def __init__(self, status: int, code: str):
        super().__init__(code)
        self.status = status
        self.code = code


def _canonical(value: dict) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def pending(outcome: str, operation_id: str = OPERATION_ID) -> Response:
    """The envelope the bridge returns the moment the Mac has the action."""
    return Response(
        content=_canonical(
            {
                "protocol": BRIDGE_RESULT,
                "state": "pending",
                "outcome": outcome,
                "operation_id": operation_id,
            }
        ),
        status_code=202,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


def with_action_result(hub: FakeHub, outcome: dict) -> FakeHub:
    """The hub's second private route, answering with this outcome."""
    auth = Authenticator(
        BRIDGE_KEY_ID,
        BRIDGE_KEY,
        protocol=BRIDGE_PROTOCOL,
        path=ACTION_RESULT_PATH,
        header_prefix=BRIDGE_HEADER_PREFIX,
        failure=RouteFailure,
    )

    @hub.app.post(ACTION_RESULT_PATH)
    async def action_result(request: Request) -> Response:
        payload = await request.body()
        auth.admit(request.scope["headers"], payload, int(time.time() * 1000), monotonic())
        return Response(
            content=_canonical(
                {
                    "protocol": BRIDGE_RESULT,
                    "state": "answered",
                    "operation_id": OPERATION_ID,
                    **outcome,
                }
            ),
            status_code=200,
            media_type="application/json",
            headers={"Cache-Control": "no-store"},
        )

    return hub


# --- A1 ---------------------------------------------------------------------


async def test_a1_an_action_in_flight_is_never_called_awaiting_approval(tmp_path):
    """The exact defect: `state: pending` produced the approval wording."""
    hub = FakeHub(device_replies=[pending("in_progress")])
    context = serving(tmp_path, hub)
    client, _server = await context.__aenter__()
    try:
        answer = await client.device_request(
            DEVICES[DEFAULT_DEVICE],
            "perform_action",
            {"action": {"tool": "launch_app", "arguments": {"bundle_id": "com.apple.Safari"}}},
        )
    finally:
        await context.__aexit__(None, None, None)

    assert answer["outcome"] == "in_progress"
    assert answer["operation_id"] == OPERATION_ID
    assert not APPROVAL_WORDS.search(json.dumps(answer, ensure_ascii=False)), answer


async def test_a1_the_tool_tells_the_assistant_the_action_is_under_way(tmp_path):
    hub = FakeHub(device_replies=[pending("in_progress")])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool(
            "orbit_perform_action",
            {
                "device": DEFAULT_DEVICE,
                "action": {"tool": "launch_app", "arguments": {"bundle_id": "com.apple.Safari"}},
            },
        )
    finally:
        await context.__aexit__(None, None, None)

    assert not result.is_error
    spoken = text_of(result)
    assert "in_progress" in spoken
    assert not APPROVAL_WORDS.search(spoken), spoken
    answer = json.loads(result.content[0].text)
    assert "orbit_action_result" in answer["next_step"]
    assert not APPROVAL_WORDS.search(answer["next_step"])


async def test_a1_the_hub_route_names_an_accepted_action_in_progress(bridged):
    """The wire itself, not only the client that reads it."""
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        payload = body(
            "perform_action",
            owner["device_id"],
            action={"tool": "synthetic_tool", "arguments": {}},
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(BRIDGE_PATH, content=payload, headers=signed(payload))
        assert response.status_code == 202
        accepted = response.json()
        assert accepted["state"] == "pending"
        assert accepted["outcome"] == "in_progress"
        assert "awaiting_approval" not in json.dumps(accepted)
    assert "awaiting_approval" not in ACTION_OUTCOMES
    assert "in_progress" in ACTION_OUTCOMES


# --- A2 ---------------------------------------------------------------------


async def test_a2_asking_for_the_outcome_while_it_is_still_running_says_so(tmp_path):
    hub = with_action_result(FakeHub(), {"outcome": "in_progress"})
    context = serving(tmp_path, hub)
    client, _server = await context.__aenter__()
    try:
        answer = await client.action_result(DEVICES[DEFAULT_DEVICE], OPERATION_ID)
    finally:
        await context.__aexit__(None, None, None)

    assert answer["outcome"] == "in_progress"
    assert "orbit_action_result" in answer["next_step"]
    assert not APPROVAL_WORDS.search(json.dumps(answer, ensure_ascii=False)), answer


# --- A6/A7 ------------------------------------------------------------------


async def test_a6_a7_a_tool_described_at_length_survives_the_walk(tmp_path):
    """The client assembles what the Mac declares, whole, and counts it."""
    tools = [
        {"name": "launch_app", "description": "Abre una aplicación.", "input_schema": {}},
        {
            "name": "a_tool_the_engine_describes_at_length",
            "description": "á" * 3_000,
            "input_schema": {"type": "object"},
        },
    ]
    hub = FakeHub(device_replies=[answered({"tools": tools, "after_index": 0, "total": 2})])
    context = serving(tmp_path, hub)
    client, _server = await context.__aenter__()
    try:
        catalogue = await client.control_tools(DEVICES[DEFAULT_DEVICE])
    finally:
        await context.__aexit__(None, None, None)

    assert len(catalogue) == 2
    verbose = next(
        tool for tool in catalogue if tool["name"] == "a_tool_the_engine_describes_at_length"
    )
    assert len(verbose["description"].encode("utf-8")) > 4_096
