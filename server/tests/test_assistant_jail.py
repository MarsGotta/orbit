"""Kernel isolation on invented folders, using the production launch builder."""

# ruff: noqa: F811

import asyncio
import os
import shutil
import socket
import subprocess
import sys
import tempfile
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace

import pytest
from test_official_assistants import official  # noqa: F401


def fixture_paths(environment, tmp_path, monkeypatch):
    settings, _, _ = environment
    data = tmp_path / "data"
    data.mkdir(exist_ok=True)
    storage_key = tmp_path / "storage.key"
    storage_key.write_text("invented storage key")
    hermes_key = tmp_path / "hermes.key"
    hermes_key.write_text("invented Hermes key")
    settings = replace(
        settings, data_dir=data, key_file=storage_key, hermes_credential_file=hermes_key
    )
    home = tmp_path / "owner"
    (home / ".ssh").mkdir(parents=True)
    (home / ".ssh" / "key").write_text("invented ssh key")
    monkeypatch.setenv("HOME", str(home))
    work = tmp_path / "work"
    work.mkdir()
    vendor = settings.data_dir / "agents" / "helper" / "codex-home"
    vendor.mkdir(parents=True)
    tools = vendor.parent / "bridge"
    tools.mkdir()
    (tools / "device.key").write_bytes(b"fixture")
    other = settings.data_dir.with_name(settings.data_dir.name + "-workspaces") / "other"
    other.mkdir(parents=True)
    (other / "secret").write_text("invented other data")
    (settings.data_dir / "secret").write_text("invented hub data")
    config = tmp_path / "hub.toml"
    config.write_text("invented configuration")
    tls = tmp_path / "tls.key"
    tls.write_text("invented tls key")
    settings = replace(settings, config_file=config, tls_key_file=tls)
    agent = SimpleNamespace(id="helper", kind="codex")
    return settings, agent, work, vendor, tools, other, home


@pytest.fixture
def kernel_root():
    # The jail deliberately replaces /tmp. Keep real mount fixtures elsewhere
    # so their visibility tests exercise the explicit masks and binds.
    target = Path(__file__).resolve().parents[2] / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="orbit-jail-", dir=target) as folder:
        yield Path(folder)


def test_jail_arguments_cover_configured_paths_in_order(environment, tmp_path, monkeypatch):
    from orbit_hub.assistant.jail import arguments

    settings, agent, work, vendor, tools, other, home = fixture_paths(
        environment, tmp_path, monkeypatch
    )
    args = arguments(settings, agent, work)
    assert args[:11] == [
        "bwrap",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--unshare-pid",
    ]
    assert args[11:15] == ["--die-with-parent", "--new-session", "--setenv", "TMPDIR"]
    assert args[15] == "/tmp"
    for path in (settings.data_dir, other, home / ".ssh"):
        i = args.index(str(path.resolve()))
        assert args[i - 1] == "--tmpfs"
        assert ["--remount-ro", str(path.resolve())] == args[
            args.index("--remount-ro", i) : args.index("--remount-ro", i) + 2
        ]
    for path in (
        settings.key_file,
        settings.hermes_credential_file,
        settings.config_file,
        settings.tls_key_file,
    ):
        if path.exists() and settings.data_dir not in path.parents:
            i = args.index(str(path.resolve()))
            assert args[i - 2 : i + 1] == ["--ro-bind", "/dev/null", str(path.resolve())]
    assert args[-8:] == [
        "--bind",
        str(work),
        str(work),
        "--bind",
        str(vendor),
        str(vendor),
        "--chdir",
        str(work),
    ]
    assert ["--ro-bind", str(tools), str(tools)] == args[-11:-8]
    assert "/run" not in args  # /run/systemd/resolve remains visible for DNS.
    for path in (
        f"/run/user/{os.getuid()}",
        "/run/docker.sock",
        "/run/containerd",
        "/var/snap/lxd/common/lxd",
        "/var/lib/lxd",
    ):
        if Path(path).exists():
            assert str(Path(path).resolve()) in args


def test_complete_mount_order_uses_only_invented_paths(environment, tmp_path, monkeypatch):
    from orbit_hub.assistant.jail import arguments

    settings, agent, work, vendor, bridge, other, home = fixture_paths(
        environment, tmp_path, monkeypatch
    )
    owner_entries = (
        ".ssh",
        ".gnupg",
        ".claude",
        ".claude.json",
        ".codex",
        ".config/gh",
        ".config/gcloud",
        ".aws",
        ".azure",
        ".kube",
        ".docker",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".git-credentials",
        ".password-store",
        ".local/share/keyrings",
    )
    files = {".claude.json", ".netrc", ".npmrc", ".pypirc", ".git-credentials"}
    for entry in owner_entries:
        path = home / entry
        if entry in files:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("invented credential")
        else:
            path.mkdir(parents=True, exist_ok=True)
    session = tmp_path / "session-control"
    session.mkdir()
    docker = tmp_path / "docker.sock"
    docker.touch()
    monkeypatch.setattr("orbit_hub.assistant.jail.escape_paths", lambda: (session, docker))
    custom = tmp_path / "other-project"
    custom.mkdir()
    configs = SimpleNamespace(
        working_folders=lambda: (("other", str(custom)), (agent.id, str(work)))
    )
    expected = [
        "bwrap",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--unshare-pid",
        "--die-with-parent",
        "--new-session",
        "--setenv",
        "TMPDIR",
        "/tmp",
        "--tmpfs",
        str(settings.data_dir),
        "--dir",
        str(vendor),
        "--dir",
        str(bridge),
        "--remount-ro",
        str(settings.data_dir),
    ]
    for path in (
        settings.key_file,
        settings.hermes_credential_file,
        settings.config_file,
        settings.tls_key_file,
    ):
        expected.extend(("--ro-bind", "/dev/null", str(path)))
    for path in (other, custom, session):
        expected.extend(("--tmpfs", str(path), "--remount-ro", str(path)))
    expected.extend(("--ro-bind", "/dev/null", str(docker)))
    for entry in owner_entries:
        path = str(home / entry)
        expected.extend(
            ("--ro-bind", "/dev/null", path)
            if entry in files
            else ("--tmpfs", path, "--remount-ro", path)
        )
    root = str(other.parent)
    expected.extend(("--tmpfs", root, "--remount-ro", root))
    expected.extend(("--ro-bind", str(bridge), str(bridge)))
    expected.extend(
        ("--bind", str(work), str(work), "--bind", str(vendor), str(vendor), "--chdir", str(work))
    )
    assert arguments(settings, agent, work, configs) == expected


def test_nested_settings_and_operator_translator_secrets_are_hidden(
    environment, tmp_path, monkeypatch
):
    from orbit_hub.assistant.jail import arguments
    from orbit_hub.private_auth import BridgeConfiguration
    from orbit_hub.processing.config import ProcessingConfiguration

    settings, agent, work, _, _, _, _ = fixture_paths(environment, tmp_path, monkeypatch)
    names = ("provider", "speech", "device", "meetings", "processing", "translator")
    secrets = {name: tmp_path / f"{name}.key" for name in names}
    for path in secrets.values():
        path.write_text("invented credential")
    template = tmp_path / "operator-mcp.toml"
    template.write_text(f'[device_bridge]\nkey_file = "{secrets["translator"]}"\n')
    settings = replace(
        settings,
        agents=(
            {
                "id": "planner",
                "name": "Vega",
                "kind": "openai",
                "model": "fixture",
                "base_url": "https://provider.example.test/v1",
                "api_key_file": str(secrets["provider"]),
                "orbit_mcp_config": str(template),
                "speech": {
                    "base_url": "https://voice.example.test/v1",
                    "model": "fixture",
                    "voice": "fixture",
                    "api_key_file": str(secrets["speech"]),
                },
            },
        ),
        assistant_tools={"codex": {"orbit_mcp_config": str(template)}},
        device_bridge=BridgeConfiguration(key_id="device", credential_file=str(secrets["device"])),
        meetings_bridge=BridgeConfiguration(
            key_id="meetings", credential_file=str(secrets["meetings"])
        ),
        processing=ProcessingConfiguration(
            endpoint="https://processing.example.test/v1/chat/completions",
            model="fixture",
            credential_file=secrets["processing"],
        ),
    )
    args = arguments(settings, agent, work)
    for path in (*secrets.values(), template):
        index = args.index(str(path))
        assert args[index - 2 : index + 1] == ["--ro-bind", "/dev/null", str(path)]


def test_real_jail_hides_secrets_and_confines_writes(environment, kernel_root, monkeypatch, capsys):
    from orbit_hub.assistant.jail import launch

    # Only this exact capability probe may skip the real-kernel acceptance.
    binary = shutil.which("bwrap", path=os.defpath)
    try:
        probe = subprocess.run(
            [binary or "bwrap", "--ro-bind", "/", "/", "true"],
            capture_output=True,
            check=False,
            timeout=10,
        )
    except FileNotFoundError as error:
        pytest.fail(f"bubblewrap must be installed for kernel acceptance: {error}")
    if probe.returncode:
        reason = "bubblewrap cannot create a jail here: " + probe.stderr.decode().strip()
        with capsys.disabled():
            print(reason)
        pytest.skip(reason)
    # Bypass only the protocol suite's autouse process double. Both the
    # production admission probe and the command below must use real bwrap.
    monkeypatch.setattr("orbit_hub.assistant.jail.shutil.which", lambda _: binary)
    settings, agent, work, vendor, _, other, home = fixture_paths(
        environment, kernel_root, monkeypatch
    )
    secret_root = kernel_root / "stacks"
    secret_root.mkdir()
    (secret_root / ".env").write_text("invented service secret")
    (secret_root / "config.toml").write_text("invented tool configuration")
    (secret_root / "readme.txt").write_text("public neighbor")
    work = secret_root / "project"
    work.mkdir()
    (work / ".env.local").write_text("invented workspace secret")
    settings = replace(settings, assistant_tools={"jail": {"secret_roots": [str(secret_root)]}})
    dock = kernel_root / "docker.sock"
    with socket.socket(socket.AF_UNIX) as sock:
        sock.bind(str(dock))
        monkeypatch.setattr("orbit_hub.assistant.jail.escape_paths", lambda: (dock,))
        parent = os.getpid()
        script = """import os, pathlib, socket, sys
hidden, other, ssh, dock, outside, work, vendor, parent, secrets = sys.argv[1:]
for path in (pathlib.Path(secrets, '.env'), pathlib.Path(secrets, 'config.toml'),
             pathlib.Path(work, '.env.local')):
    # The /dev/null bind is nodev in an unprivileged namespace, so opening
    # it is refused; either way the secret's bytes never reach the jail.
    try:
        content = path.read_bytes()
    except PermissionError:
        content = b''
    assert content == b'', 'secret readable: ' + str(path)
assert pathlib.Path(secrets, 'readme.txt').read_text() == 'public neighbor'
for path, name in ((hidden, 'secret'), (other, 'secret'), (ssh, 'key')):
    try:
        pathlib.Path(path, name).read_text()
    except OSError:
        pass
    else:
        raise AssertionError('hidden file readable: ' + path)
try:
    pathlib.Path(hidden, 'escaped').write_text('escaped')
except OSError:
    pass
else:
    raise AssertionError('hidden mount writable')
try:
    socket.socket(socket.AF_UNIX).connect(dock)
except OSError:
    pass
else:
    raise AssertionError('docker socket exposed')
try:
    pathlib.Path(outside).write_text('escaped')
except OSError:
    pass
else:
    raise AssertionError('outside write allowed')
try:
    os.link(hidden + '/secret', work + '/link')
except OSError:
    pass
else:
    raise AssertionError('hidden hard link allowed')
for folder in (work, vendor, '/tmp'):
    pathlib.Path(folder, 'written').write_text('ok')
assert not pathlib.Path('/proc', parent).exists(), 'parent visible'
assert os.environ['TMPDIR'] == '/tmp'
"""
        prefix = asyncio.run(launch(settings, agent, work))
        result = subprocess.run(
            [
                *prefix,
                sys.executable,
                "-c",
                script,
                str(settings.data_dir),
                str(other),
                str(home / ".ssh"),
                str(dock),
                str(kernel_root / "outside"),
                str(work),
                str(vendor),
                str(parent),
                str(secret_root),
            ],
            capture_output=True,
            check=False,
            timeout=20,
        )
    assert result.returncode == 0, result.stderr.decode()
    assert (work / "written").read_text() == "ok"
    assert (vendor / "written").read_text() == "ok"


@pytest.mark.parametrize("official", ["claude-code", "codex"], indirect=True)
def test_missing_bwrap_fails_closed_through_chat(official, monkeypatch):
    from test_official_assistants import send

    _, _, _, _, headers, client = official
    monkeypatch.setattr("orbit_hub.assistant.jail.shutil.which", lambda _: None)
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        while ws.receive_json()["event"] != "ready":
            pass
        send(ws, "chat.send", text="Must never reach a vendor")
        while (frame := ws.receive_json())["event"] != "error":
            pass
    error = frame["data"]
    assert error["code"] == "agent_adapter_missing"
    assert "bubblewrap" in error["message"] and "instala" in error["message"].lower()
