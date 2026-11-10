"""A hub run as a program refuses to serve without its transport.

`contracts/hub-v1.md` makes TLS mandatory except explicit localhost
development, and `allow_insecure_localhost` allows plain HTTP on loopback only.
So `serve` without TLS beyond loopback, on loopback without that permission,
or with `--tls-cert` but no `--tls-key` must not start: the hub exits with 1,
writes one line of error output that holds the cause, and never listens on the
port it was given. The kit compares the cause, not how the hub begins its line.

Each case is a new folder with a new storage key and no database. The kit
watches the port while the hub lives and fails at once if it answers; the hub
is stopped when the case ends, whatever happened.
"""

import secrets
import socket
import subprocess
import time
from pathlib import Path

import pytest
from kit_hub import COMMAND_SECONDS, _start, _stop, free_port, prepare

# The size of the storage key a hub reads from its key file.
STORAGE_KEY_BYTES = 32
NO_TLS = "serve requires TLS unless allow_insecure_localhost is set and the host is loopback"
ONE_TLS_FILE = "TLS requires both --tls-cert and --tls-key"
# How often the kit looks whether the hub listens or has exited.
WATCH_SECONDS = 0.05


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=WATCH_SECONDS):
            return True
    except OSError:
        return False


def refusal(configuration: Path, arguments: list[str]) -> tuple[int, str]:
    """`serve` on a free port with `arguments`, watched until it exits;
    answers its exit code and its error output. Fails if it listens or does
    not exit within `COMMAND_SECONDS`."""
    port = free_port()
    folder = configuration.parent
    err_path = folder / "serve.err"
    with err_path.open("w") as err:
        hub = _start(
            configuration,
            ["serve", "--port", str(port), *arguments],
            stdout=subprocess.DEVNULL,
            stderr=err,
        )
    try:
        deadline = time.monotonic() + COMMAND_SECONDS
        while hub.poll() is None:
            if listening(port):
                pytest.fail(f"the hub served on port {port} with {arguments}")
            if time.monotonic() > deadline:
                pytest.fail(
                    f"`serve {' '.join(arguments)}` did not exit within {COMMAND_SECONDS} s"
                )
            time.sleep(WATCH_SECONDS)
    finally:
        _stop(hub)
    return hub.returncode, err_path.read_text(errors="replace")


@pytest.mark.parametrize(
    "arguments, insecure_localhost, cause",
    [
        (["--host", "0.0.0.0"], True, NO_TLS),
        (["--host", "127.0.0.1"], False, NO_TLS),
        # A file that is not there: the hub must refuse before it reads it.
        (["--tls-cert", "/fixture/cert.pem"], False, ONE_TLS_FILE),
    ],
    ids=["beyond loopback", "loopback without the permission", "one TLS file"],
)
def test_serve_refuses_to_start_without_its_transport(
    tmp_path, arguments, insecure_localhost, cause
):
    configuration = prepare(
        tmp_path / "hub",
        secrets.token_bytes(STORAGE_KEY_BYTES),
        insecure_localhost=insecure_localhost,
    )
    code, said = refusal(configuration, arguments)

    assert code == 1, said
    lines = said.splitlines()
    assert len(lines) == 1 and cause in lines[0], said
