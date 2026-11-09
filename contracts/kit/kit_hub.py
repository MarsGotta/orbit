"""Start a hub as a program and reach it only through its address.

The command that runs the hub comes from `ORBIT_KIT_HUB`, split as a shell
would; without it, today's Python hub. The kit appends `--config <file>
<command> [arguments]`, as the hub's administrator commands take them, and
talks to a serving hub over HTTP as a device it pairs itself (`pair-code`,
then `POST /v1/pair`), with that device's bearer token. Nothing
here imports the hub, so the same kit tests a hub written in any language.

The configuration is invented and names the keys today's hub reads: a
loopback hub without TLS, meeting transcription pointed at a closed port, one
API agent (`cedar`, of kind openai) whose model is behind that closed port, a
Hermes address and credential that nothing uses, and the device bridge's key
id with a key the kit draws anew for each folder it prepares. The kit signs a
private request with that key as `contracts/hub-v1.md` writes it («Private
meetings for the assistant»).
"""

import hashlib
import hmac
import json
import os
import secrets
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import httpx
import pytest

KIT = Path(__file__).resolve().parent
SAMPLE = KIT.parent / "fixtures" / "hub-storage"
GOLDEN = SAMPLE / "golden-v1.sqlite3"
TEST_KEY = SAMPLE / "golden-v1.test-key.hex"
MANIFEST = SAMPLE / "golden-v1.json"
DATABASE_NAME = "hub.sqlite3"
KIT_DEVICE_NAME = "Contract kit"
# A privileged port no unprivileged service can listen on: whatever the hub
# would send there is refused at once, never answered.
CLOSED_PORT = 9
# Today's hub answered its first /health 1.7 to 2.4 s after `serve` on the
# gate host with another heavy session running (server/tests/test_local_mode.py
# measured it). Twenty-five times the worst, so only a hub that never starts
# fails; a hub that exits fails at once.
START_SECONDS = 60
# A serving hub closes its store on SIGTERM; past this its group is killed.
STOP_SECONDS = 30
# How often the kit looks whether a stopped hub's processes are gone.
STOP_POLL_SECONDS = 0.05
# An administrator command opens the store, writes or reads one thing and exits.
COMMAND_SECONDS = 60
# What a failure quotes of the hub's own output: enough for its last error.
OUTPUT_TAIL_CHARS = 4000
# The invented API agent: what the sample's personality and skill belong to.
AGENT_ID = "cedar"
AGENT_NAME = "Cedar"
# The device bridge, as the contract signs it.
BRIDGE_PROTOCOL = "orbit_device_bridge_v1"
BRIDGE_HEADERS = "x-orbit-bridge-"
BRIDGE_KEY_ID = "contract-kit"
BRIDGE_KEY_FILE = "bridge-credential"
BRIDGE_KEY_BYTES = 32
# The variables that name the hub's implicit agent: beside `[[agents]]` the hub refuses to start.
IMPLICIT_AGENT_VARIABLES = ("ORBIT_HUB_PROFILE", "ORBIT_HUB_ASSISTANT_NAME")


class HubFailed(Exception):
    """The hub did not do what the kit asked: it exited, or never answered."""


def hub_command() -> list[str]:
    configured = os.environ.get("ORBIT_KIT_HUB", "").strip()
    if configured:
        return shlex.split(configured)
    return [sys.executable, "-m", "orbit_hub.cli"]


def read_test_key() -> bytes:
    return bytes.fromhex(TEST_KEY.read_text().strip())


def read_manifest() -> dict:
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def _toml(value) -> str:
    return json.dumps(str(value))


def _secret_file(path: Path, value: bytes) -> None:
    path.write_bytes(value)
    path.chmod(0o600)


def prepare(folder: Path, key: bytes, database: Path | None = None) -> Path:
    """A hub's folder: data 0700, the key in a file of its own (0600), a new
    device bridge key (0600), an invented configuration and, when given, a
    copy of `database` as the hub's. Answers the configuration's path."""
    folder.mkdir(mode=0o700, parents=True, exist_ok=True)
    data, home = folder / "data", folder / "home"
    data.mkdir(mode=0o700)
    home.mkdir(mode=0o700)
    key_file = folder / "storage-key.bin"
    _secret_file(key_file, key)
    credential = folder / "hermes-credential"
    _secret_file(credential, b"invented-hermes-credential\n")
    bridge_key = folder / BRIDGE_KEY_FILE
    _secret_file(bridge_key, secrets.token_bytes(BRIDGE_KEY_BYTES))
    # The agent's orbit-mcp configuration: the hub only checks that it exists.
    tools = folder / "orbit-mcp.toml"
    tools.write_text("# Invented: no tool of the agent runs under the kit.\n")
    if database is not None:
        # Always a copy: SQLite leaves -wal and -shm beside a WAL database it
        # opens, and a hub writes to the one it serves.
        shutil.copyfile(database, data / DATABASE_NAME)
    configuration = folder / "hub.toml"
    configuration.write_text(
        "\n".join(
            (
                f"data_dir = {_toml(data)}",
                f"key_file = {_toml(key_file)}",
                f'hermes_url = "http://127.0.0.1:{CLOSED_PORT}"',
                f"hermes_credential_file = {_toml(credential)}",
                "allow_insecure_localhost = true",
                "[meeting_transcription]",
                f'endpoint = "http://127.0.0.1:{CLOSED_PORT}"',
                'model = "invented-whisper"',
                "[device_bridge]",
                f"key_id = {_toml(BRIDGE_KEY_ID)}",
                f"credential_file = {_toml(bridge_key)}",
                "[[agents]]",
                f"id = {_toml(AGENT_ID)}",
                f"name = {_toml(AGENT_NAME)}",
                'kind = "openai"',
                f'base_url = "http://127.0.0.1:{CLOSED_PORT}/v1"',
                'model = "invented-model"',
                f"orbit_mcp_config = {_toml(tools)}",
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return configuration


def _environment(configuration: Path) -> dict[str, str]:
    # An empty home of its own: the hub must not find anything of this machine's user.
    environment = {**os.environ, "HOME": str(configuration.parent / "home")}
    for variable in IMPLICIT_AGENT_VARIABLES:
        environment.pop(variable, None)
    return environment


def _start(configuration: Path, arguments: list[str], **streams) -> subprocess.Popen:
    # A process group of its own: a hub started through a wrapper (`sh -c`,
    # `cargo run`, `npx`) is a grandchild, and stopping only the wrapper
    # would leave it serving. The kit stops the whole group.
    return subprocess.Popen(
        [*hub_command(), "--config", str(configuration), *arguments],
        env=_environment(configuration),
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        **streams,
    )


def _group_alive(group: int) -> bool:
    try:
        os.killpg(group, 0)
    except ProcessLookupError:
        return False
    return True


def _stop(process: subprocess.Popen) -> None:
    """SIGTERM to the process's group, then SIGKILL to what is left of it
    after `STOP_SECONDS`. Returns once the group is gone."""
    group = process.pid
    try:
        os.killpg(group, signal.SIGTERM)
    except ProcessLookupError:
        process.wait()
        return
    deadline = time.monotonic() + STOP_SECONDS
    while time.monotonic() < deadline:
        process.poll()  # reaps the leader, which would otherwise keep the group
        if not _group_alive(group):
            return
        time.sleep(STOP_POLL_SECONDS)
    try:
        os.killpg(group, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def run(configuration: Path, *arguments: str) -> str:
    """One administrator command; answers what it printed."""
    command = _start(
        configuration, list(arguments), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
    )
    try:
        out, err = command.communicate(timeout=COMMAND_SECONDS)
    except subprocess.TimeoutExpired:
        _stop(command)
        out, err = command.communicate()
        said = (out + err)[-OUTPUT_TAIL_CHARS:]
        raise HubFailed(
            f"`{' '.join(arguments)}` did not finish within {COMMAND_SECONDS} s:\n{said}"
        ) from None
    finally:
        _stop(command)
    if command.returncode != 0:
        said = (out + err)[-OUTPUT_TAIL_CHARS:]
        raise HubFailed(f"`{' '.join(arguments)}` exited with {command.returncode}:\n{said}")
    return out


def free_port() -> int:
    # Bound by this process for a moment, so a test process whose sockets are
    # fenced to its own ports (server/tests) may connect to it. Should another
    # process take it before the hub binds, the hub exits and the kit fails.
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def _tail(path: Path) -> str:
    return path.read_text(errors="replace")[-OUTPUT_TAIL_CHARS:] if path.exists() else ""


def _wait_for_health(hub: subprocess.Popen, url: str, output: Path) -> None:
    deadline = time.monotonic() + START_SECONDS
    while time.monotonic() < deadline:
        code = hub.poll()
        if code is not None:
            raise HubFailed(
                f"the hub exited with {code} before answering /health:\n{_tail(output)}"
            )
        try:
            if httpx.get(f"{url}/health", timeout=2).status_code == 200:
                return
        except httpx.TransportError:
            pass
        time.sleep(0.1)
    raise HubFailed(f"the hub did not answer /health within {START_SECONDS} s:\n{_tail(output)}")


@contextmanager
def serving(configuration: Path) -> Iterator[str]:
    """`serve` on a free loopback port until the block ends; yields its URL.
    Fails, with what the hub said, if it exits or never answers /health."""
    port = free_port()
    output = configuration.parent / "serve.log"
    with output.open("w") as log:
        hub = _start(
            configuration, ["serve", "--port", str(port)], stdout=log, stderr=subprocess.STDOUT
        )
    url = f"http://127.0.0.1:{port}"
    try:
        _wait_for_health(hub, url, output)
        yield url
    finally:
        _stop(hub)


def pair_device(url: str, code: str, name: str) -> dict:
    """`POST /v1/pair` with a code from `pair-code`; answers the device's
    `device_id` and `token`."""
    response = httpx.post(f"{url}/v1/pair", json={"code": code, "name": name}, timeout=30)
    if response.status_code != 200:
        raise HubFailed(f"pairing answered {response.status_code}: {response.text}")
    return response.json()


def signed_headers(configuration: Path, path: str, body: bytes = b"") -> dict[str, str]:
    """The device bridge's headers for a request to `path` with `body`, signed
    now with the key of the hub's folder: seven lines (the protocol, `POST`
    whatever the method, the path, the key id, the time in Unix milliseconds,
    a UUID nonce, the body's SHA-256), HMAC-SHA256 in lowercase hexadecimal."""
    key = (configuration.parent / BRIDGE_KEY_FILE).read_bytes()
    issued_at_ms, nonce = str(time.time_ns() // 1_000_000), str(uuid.uuid4())
    lines = (
        BRIDGE_PROTOCOL,
        "POST",
        path,
        BRIDGE_KEY_ID,
        issued_at_ms,
        nonce,
        hashlib.sha256(body).hexdigest(),
    )
    signature = hmac.new(key, "\n".join(lines).encode(), hashlib.sha256).hexdigest()
    values = {
        "key-id": BRIDGE_KEY_ID,
        "issued-at-ms": issued_at_ms,
        "nonce": nonce,
        "signature": signature,
    }
    return {f"{BRIDGE_HEADERS}{name}": value for name, value in values.items()}


def device_client(url: str, token: str) -> httpx.Client:
    """HTTP as a paired device: its credential in `Authorization: Bearer`."""
    return httpx.Client(base_url=url, headers={"Authorization": f"Bearer {token}"}, timeout=30)


@pytest.fixture(scope="module")
def sample_configuration(tmp_path_factory) -> Path:
    """The configuration of the hub that serves a copy of the sample: its
    folder holds the device bridge key the kit signs with."""
    folder = tmp_path_factory.mktemp("sample-hub") / "hub"
    return prepare(folder, read_test_key(), GOLDEN)


@pytest.fixture(scope="module")
def sample_signature(sample_configuration):
    """Signs, when called, a device bridge request to the hub serving the sample."""
    return lambda path, body=b"": signed_headers(sample_configuration, path, body)


@pytest.fixture(scope="module")
def sample_hub(sample_configuration) -> Iterator[tuple[httpx.Client, dict]]:
    """A hub serving a copy of the sample database, as a device the kit pairs
    on that copy (the sample keeps no token), and the manifest of what was
    written into it."""
    manifest = read_manifest()
    code = run(sample_configuration, "pair-code").strip()
    with serving(sample_configuration) as url:
        paired = pair_device(url, code, KIT_DEVICE_NAME)
        with device_client(url, paired["token"]) as http:
            yield http, manifest
