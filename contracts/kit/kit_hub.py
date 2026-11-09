"""Start a hub as a program and reach it only through its address.

The command that runs the hub comes from `ORBIT_KIT_HUB`, split as a shell
would; without it, today's Python hub. The kit appends `--config <file>
<command> [arguments]`, as the hub's administrator commands take them, and
talks to a serving hub over HTTP as a device it pairs itself (`pair-code`,
then `POST /v1/pair`), with that device's bearer token. Nothing
here imports the hub, so the same kit tests a hub written in any language.

The configuration is invented and names the keys today's hub reads: a
loopback hub without TLS, meeting transcription pointed at a closed port, and
a Hermes address and credential that nothing uses (the hub's implicit agent
is of kind hermes, which makes both required).
"""

import json
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
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


def prepare(folder: Path, key: bytes, database: Path | None = None) -> Path:
    """A hub's folder: data 0700, the key in a file of its own (0600), an
    invented configuration and, when given, a copy of `database` as the hub's.
    Answers the configuration's path."""
    folder.mkdir(mode=0o700, parents=True, exist_ok=True)
    data, home = folder / "data", folder / "home"
    data.mkdir(mode=0o700)
    home.mkdir(mode=0o700)
    key_file = folder / "storage-key.bin"
    key_file.write_bytes(key)
    key_file.chmod(0o600)
    credential = folder / "hermes-credential"
    credential.write_text("invented-hermes-credential\n")
    credential.chmod(0o600)
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
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return configuration


def _environment(configuration: Path) -> dict[str, str]:
    # An empty home of its own: the hub must not find anything of this machine's user.
    return {**os.environ, "HOME": str(configuration.parent / "home")}


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


def pair_device(url: str, code: str, name: str) -> str:
    """`POST /v1/pair` with a code from `pair-code`; answers the device's token."""
    response = httpx.post(f"{url}/v1/pair", json={"code": code, "name": name}, timeout=30)
    if response.status_code != 200:
        raise HubFailed(f"pairing answered {response.status_code}: {response.text}")
    return response.json()["token"]


def device_client(url: str, token: str) -> httpx.Client:
    """HTTP as a paired device: its credential in `Authorization: Bearer`."""
    return httpx.Client(base_url=url, headers={"Authorization": f"Bearer {token}"}, timeout=30)


@pytest.fixture(scope="module")
def sample_hub(tmp_path_factory) -> Iterator[tuple[httpx.Client, dict]]:
    """A hub serving a copy of the sample database, as a device the kit pairs
    on that copy (the sample keeps no token), and the manifest of what was
    written into it."""
    manifest = read_manifest()
    folder = tmp_path_factory.mktemp("sample-hub") / "hub"
    configuration = prepare(folder, read_test_key(), GOLDEN)
    code = run(configuration, "pair-code").strip()
    with (
        serving(configuration) as url,
        device_client(url, pair_device(url, code, KIT_DEVICE_NAME)) as http,
    ):
        yield http, manifest
