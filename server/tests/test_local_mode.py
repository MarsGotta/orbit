"""The hub in local mode: in a given folder, only on 127.0.0.1, without systemd or Hermes.

The real `orbit-hub` command, as its own process, the way a Mac would start
it: a configuration inside one folder that names no Hermes, one
OpenAI-compatible agent at an address nothing serves, plain HTTP on loopback
and no service manager. The environment carries no systemd or Hermes
variables, and HOME is an empty invented folder that must stay empty.
"""

import json
import os
import socket
import subprocess
import sys
import time
import urllib.request

import pytest

# Measured on the gate host (three runs, another heavy session running):
# `init` took 0.44 to 0.52 s and `serve` answered its first /health in 1.7 to
# 2.4 s. Twenty-five times the worst, so only a hub that never starts fails.
START_SECONDS = 60


def toml(value) -> str:
    return json.dumps(str(value))


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def local_configuration(folder):
    (folder / "mcp.toml").write_text("# orbit-mcp validates its own file\n")
    path = folder / "orbit.toml"
    path.write_text(
        "\n".join(
            (
                f"data_dir = {toml(folder / 'data')}",
                f"key_file = {toml(folder / 'storage.key')}",
                "allow_insecure_localhost = true",
                "[meeting_transcription]",
                'endpoint = "http://127.0.0.1:9"',
                'model = "modelo-sintetico"',
                "[[agents]]",
                'id = "lyra"',
                'name = "Lyra"',
                'kind = "openai"',
                'base_url = "http://127.0.0.1:9/v1"',
                'model = "modelo-sintetico"',
                f"orbit_mcp_config = {toml(folder / 'mcp.toml')}",
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return path


def request(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {} if body is None else {"Content-Type": "application/json"}
    with urllib.request.urlopen(
        urllib.request.Request(url, data=data, headers=headers), timeout=10
    ) as response:
        return response.status, json.loads(response.read())


def wait_for_health(hub, url):
    deadline = time.monotonic() + START_SECONDS
    while time.monotonic() < deadline:
        if hub.poll() is not None:
            raise AssertionError(f"the hub exited: {hub.stderr.read().decode()}")
        try:
            return request(url + "/health")
        except OSError:
            time.sleep(0.1)
    raise AssertionError(f"the hub did not answer /health within {START_SECONDS} s")


@pytest.mark.skipif(not sys.platform.startswith(("linux", "darwin")), reason="a Unix hub")
def test_the_hub_serves_from_its_folder_on_loopback_only_without_hermes(tmp_path):
    # A folder of its own: conftest's fake jail writes into tmp_path itself.
    root = tmp_path / "local"
    folder, home, scratch = root / "hub", root / "home", root / "scratch"
    for path in (root, folder, home, scratch):
        path.mkdir(mode=0o700)
    configuration = local_configuration(folder)
    environment = {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "TMPDIR": str(scratch),
        "LANG": "C.UTF-8",
    }
    command = [sys.executable, "-I", "-m", "orbit_hub.cli", "--config", str(configuration)]
    run = {"env": environment, "capture_output": True, "text": True, "timeout": START_SECONDS}
    initialized = subprocess.run([*command, "init"], check=False, **run)
    assert initialized.returncode == 0, initialized.stderr
    port = free_port()
    hub = subprocess.Popen(
        [*command, "serve", "--port", str(port)],
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    url = f"http://127.0.0.1:{port}"
    try:
        status, health = wait_for_health(hub, url)
        assert status == 200 and health["ok"] is True
        # A Mac pairs with it as with any hub: a code from its own command.
        code = subprocess.run([*command, "pair-code"], check=False, **run)
        assert code.returncode == 0, code.stderr
        status, paired = request(
            url + "/v1/pair", {"code": code.stdout.strip(), "name": "Mac de prueba"}
        )
        assert status == 200 and paired["device_id"] and paired["token"]
        # Only 127.0.0.1: another loopback address on the same port is refused.
        with pytest.raises(OSError):
            socket.create_connection(("127.0.0.2", port), timeout=2).close()
    finally:
        hub.terminate()
        try:
            hub.wait(timeout=30)
        finally:
            if hub.returncode is None:
                hub.kill()
                hub.wait()
            hub.stderr.close()
    # Everything it keeps is in its folder: nothing beside it, nothing in HOME.
    assert sorted(path.name for path in root.iterdir()) == ["home", "hub", "scratch"]
    assert list(home.iterdir()) == []
    assert (folder / "data").is_dir()
