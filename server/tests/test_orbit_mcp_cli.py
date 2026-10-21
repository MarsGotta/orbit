"""The translator process must refuse to start on an unusable key.

Each case builds a key file with exactly one defect — wrong length, loose
permissions, or a symlink — and launches ``python -m orbit_mcp.cli`` with a
temporary configuration. The process has to say what is wrong on stderr,
without printing the whole path or the key material, and never come up.
"""

import asyncio
import os
import sys

import pytest
from support_orbit_mcp import BRIDGE_KEY, write_mcp_config

START_TIMEOUT_SECONDS = 10
# Port 0: nothing can listen on it, so the hub is unreachable by construction.
UNREACHABLE_HUB = "http://127.0.0.1:0"


def _short_key(directory, name):
    path = directory / name
    path.write_bytes(b"K" * 31)
    os.chmod(path, 0o600)
    return path


def _loose_key(directory, name):
    path = directory / name
    path.write_bytes(BRIDGE_KEY)
    os.chmod(path, 0o644)
    return path


def _symlinked_key(directory, name):
    real = directory / f"{name}.real"
    real.write_bytes(BRIDGE_KEY)
    os.chmod(real, 0o600)
    path = directory / name
    path.symlink_to(real)
    return path


@pytest.mark.parametrize(
    "defect, build, markers",
    [
        ("thirty-one-bytes", _short_key, ("byte", "longitud", "tamaño", "length", "31")),
        (
            "loose-permissions",
            _loose_key,
            ("permiso", "permission", "0600", "0644", "modo", "mode"),
        ),
        ("symlink", _symlinked_key, ("enlace", "symlink", "symbolic")),
    ],
)
async def test_the_process_refuses_to_start_on_an_unusable_key(tmp_path, defect, build, markers):
    # The bridge key is the one the translator opens. The calendar key is
    # retired: its section is ignored, so a defect there stops nothing.
    bridge_key = build(tmp_path, "defective-bridge.key")
    config_path = write_mcp_config(
        tmp_path,
        UNREACHABLE_HUB,
        bridge_key_file=bridge_key,
    )
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "orbit_mcp.cli",
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "ORBIT_MCP_CONFIG": str(config_path)},
    )
    try:
        _stdout, stderr = await asyncio.wait_for(process.communicate(), START_TIMEOUT_SECONDS)
    except TimeoutError:
        process.kill()
        await process.wait()
        pytest.fail(f"orbit-mcp kept running with a defective key ({defect})")
    message = stderr.decode("utf-8", "replace")
    assert process.returncode != 0
    assert message.startswith("Orbit MCP cannot start: "), message
    assert any(marker in message.lower() for marker in markers), message
    # Neither the whole path nor the key content is printed.
    assert str(bridge_key) not in message
    if defect == "thirty-one-bytes":
        assert b"K" * 8 not in stderr
