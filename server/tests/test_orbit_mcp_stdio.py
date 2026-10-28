"""A real SDK client speaks to the real translator process over stdio.

The server is launched exactly the way Hermes will launch it, ``python -m
orbit_mcp.cli`` with ``ORBIT_MCP_CONFIG`` pointing at a temporary mcp.toml,
and the hub it talks to is the loopback fake with real authentication. No
Hermes, no Mac, no external network.
"""

import os
import sys

from mcp import ClientSession, StdioServerParameters, stdio_client
from support_client_e2e import DAEMON_E2E_TIMEOUT_SECONDS, tcp_server
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    DEVICES,
    OTHER_DEVICE,
    TOOL_NAMES,
    FakeHub,
    write_mcp_config,
)
from support_orbit_mcp import text_of as _text_of

LIST_TIMEOUT = DAEMON_E2E_TIMEOUT_SECONDS


async def test_stdio_client_lists_every_tool_and_calls_devices_end_to_end(tmp_path):
    hub = FakeHub()
    async with tcp_server(hub.app) as url:
        config_path = write_mcp_config(tmp_path, url)
        parameters = StdioServerParameters(
            command=sys.executable,
            args=["-m", "orbit_mcp.cli"],
            env={"ORBIT_MCP_CONFIG": str(config_path)},
        )
        async with (
            stdio_client(parameters) as (read_stream, write_stream),
            ClientSession(read_stream, write_stream) as session,
        ):
            await session.initialize()
            listing = await session.list_tools()
            assert {tool.name for tool in listing.tools} == set(TOOL_NAMES)
            result = await session.call_tool("orbit_devices", {}, read_timeout_seconds=LIST_TIMEOUT)
    assert not result.is_error
    text = _text_of(result)
    assert DEFAULT_DEVICE in text and OTHER_DEVICE in text
    assert "default" in text.lower()
    # The configured names, never the identifiers behind them.
    assert DEVICES[DEFAULT_DEVICE] not in text
    assert DEVICES[OTHER_DEVICE] not in text


async def test_mcp_toml_reload_changes_the_next_call_without_restart(tmp_path):
    old_id = "5f1d0000-0000-4000-8000-000000000046"
    new_id = "5f1d0000-0000-4000-8000-000000000047"
    hub = FakeHub(devices=[])
    async with tcp_server(hub.app) as url:
        config_path = write_mcp_config(
            tmp_path,
            url,
            default_device="equipo",
            devices={"equipo": old_id},
        )
        parameters = StdioServerParameters(
            command=sys.executable,
            args=["-m", "orbit_mcp.cli"],
            env={"ORBIT_MCP_CONFIG": str(config_path)},
        )
        async with (
            stdio_client(parameters) as (read_stream, write_stream),
            ClientSession(read_stream, write_stream) as session,
        ):
            await session.initialize()
            first = await session.call_tool(
                "orbit_windows",
                {"device": "equipo"},
                read_timeout_seconds=LIST_TIMEOUT,
            )
            previous_mtime = config_path.stat().st_mtime_ns
            write_mcp_config(
                tmp_path,
                url,
                default_device="equipo",
                devices={"equipo": new_id},
            )
            current = config_path.stat()
            os.utime(
                config_path,
                ns=(current.st_atime_ns, max(current.st_mtime_ns, previous_mtime + 1)),
            )
            second = await session.call_tool(
                "orbit_windows",
                {"device": "equipo"},
                read_timeout_seconds=LIST_TIMEOUT,
            )

    assert not first.is_error and not second.is_error
    assert [request["device_id"] for request in hub.device_requests] == [old_id, new_id]
