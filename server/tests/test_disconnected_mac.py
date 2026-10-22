"""With the Mac disconnected, every tool says so.

`orbit_calendar` handed its refusal the requested device *name* where a device
selection belongs, so a disconnected Mac made it raise
`AttributeError: 'str' object has no attribute 'device'` from inside its own
`except`, and Deimos got an internal error instead of "el Mac no está
conectado". The first test is that case against the loopback hub with real
auth; the second walks every tool that addresses a Mac, so the same slip in any
of them is caught.
"""

import time

import pytest
from support_client_e2e import tcp_server
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    DEVICES,
    FakeHub,
    hub_rejected,
    text_of,
    write_mcp_config,
)

from orbit_hub.assistant.access import GROUPS
from orbit_mcp.config import Config
from orbit_mcp.hub import HubClient, HubClientError
from orbit_mcp.server import build_server

DISCONNECTED = "device_unavailable: El Mac no está conectado a Orbit. "
LAST_SEEN = f"{DEFAULT_DEVICE} fue visto por última vez hace 2 min."
START_MS = 1_789_000_000_000
END_MS = START_MS + 3_600_000


def _disconnected_devices() -> list[dict]:
    return [
        {
            "id": DEVICES[DEFAULT_DEVICE],
            "name": DEFAULT_DEVICE,
            "paired_at": 1_000.0,
            "connected": False,
            "last_seen": time.time() - 120,
        }
    ]


async def test_orbit_calendar_says_the_mac_is_not_connected(tmp_path):
    hub = FakeHub(
        devices=_disconnected_devices(),
        device_replies=[hub_rejected(404, "device_unavailable")],
    )
    async with tcp_server(hub.app) as url:
        config = Config.load(write_mcp_config(tmp_path, url))
        server = build_server(config, HubClient(config))
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": START_MS, "end_ms": END_MS, "device": DEFAULT_DEVICE}
        )

    assert result.is_error
    assert text_of(result) == DISCONNECTED + LAST_SEEN
    assert [request["capability"] for request in hub.device_requests] == ["calendar_read"]


class DisconnectedHub:
    """A hub that knows the Mac and says it is not connected to every request."""

    def __init__(self):
        self.calls = []

    def update_config(self, _config) -> None:
        return None

    async def devices(self) -> list[dict]:
        return _disconnected_devices()

    async def agent_access(self):
        # The hub answers for itself: it is the Mac that is away, and every
        # group is on, so each tool reaches the Mac's refusal.
        return "default", dict.fromkeys(GROUPS, True)

    async def _unavailable(self, method: str):
        self.calls.append(method)
        raise HubClientError("device_unavailable", 404)

    async def device_request(self, *_args):
        await self._unavailable("device_request")

    async def control_tools(self, *_args):
        await self._unavailable("control_tools")

    async def action_result(self, *_args):
        await self._unavailable("action_result")

    async def meetings(self, *_args):
        await self._unavailable("meetings")

    async def transcript(self, *_args):
        await self._unavailable("transcript")

    async def note(self, *_args):
        await self._unavailable("note")

    # `orbit_calendar` walks the Mac's pages through `calendar_read`;
    # `calendar_query` served the retired stored copy.
    async def calendar_read(self, *_args):
        await self._unavailable("calendar_read")


ARGUMENTS = {
    "orbit_windows": {},
    "orbit_screenshot": {},
    "orbit_look": {},
    "orbit_selected_text": {},
    "orbit_control_tools": {},
    "orbit_perform_action": {"action": {"tool": "click", "arguments": {}}},
    "orbit_action_result": {"operation_id": "00000000-0000-4000-8000-000000000001"},
    "orbit_calendar": {"start_ms": START_MS, "end_ms": END_MS},
    "orbit_meetings": {},
    "orbit_transcript": {"recording_id": "00000000-0000-4000-8000-000000000002"},
    "orbit_note": {"recording_id": "00000000-0000-4000-8000-000000000002"},
    "orbit_files_list": {},
    "orbit_files_search": {"query": "plan"},
    "orbit_files_read": {"path": "/Users/ejemplo/Proyectos/plan.md"},
}


def _server(tmp_path, hub):
    config = Config.load(write_mcp_config(tmp_path, "https://hub.invalid"))
    return build_server(config, hub)


async def test_the_sweep_covers_every_tool_that_addresses_a_mac(tmp_path):
    server = _server(tmp_path, DisconnectedHub())
    addressing = {
        tool.name
        for tool in await server.list_tools()
        if "device" in tool.input_schema.get("properties", {})
    }
    assert addressing == set(ARGUMENTS)


@pytest.mark.parametrize("tool", sorted(ARGUMENTS))
async def test_every_tool_says_the_mac_is_not_connected(tmp_path, tool):
    hub = DisconnectedHub()
    server = _server(tmp_path, hub)

    result = await server.call_tool(tool, {**ARGUMENTS[tool], "device": DEFAULT_DEVICE})

    assert result.is_error, text_of(result)
    assert text_of(result) == DISCONNECTED + LAST_SEEN
    assert hub.calls, "the tool reached the hub before refusing"
