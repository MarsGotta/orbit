"""The test hub and the test Hermes of the real test, on the test server.

The hub runs from the tree under test (`server/`), with its own port, data
directory and `orbit.toml` under `~/.orbit-pruebas/`, listening only on
the test server's tailnet address. It uses the real Whisper and the real note model
(`[processing]`, whose key file is named here and read only by the hub), and
talks to the test profile of Hermes served on its own loopback port. Both are
started by the test and stopped by it, by the process group it created, also
when the test fails. Production is never touched: other ports, other
directories, another profile.

    python isolated_hub.py serve --config C --host H --port P

is how the hub is started: the hub's own `serve` refuses plain HTTP off
loopback, and the Mac reaches this one over plain HTTP on the tailnet (only
«Orbit Pruebas» accepts that, `orbit_bundle::plain_http_hub`). The tailnet is
WireGuard: the link is encrypted, so a request that arrives on it counts as
the secure transport the hub demands (`allowed_transport`). That is the only
difference from `orbit-hub serve`.
"""

from __future__ import annotations

import argparse
import ipaddress
import os
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "server"
PYTHON = SERVER / ".venv" / "bin" / "python"
TAILNET = ipaddress.ip_network("100.64.0.0/10")

# A production hub on the same server listens on 8700 and its Hermes on 8643,
# and Hermes's own tailnet serve holds 8642. These are free on the test server
# and checked before each run.
HUB_PORT = 8710
HERMES_PORT = 8653
WHISPER_MODEL = "openai/whisper-large-v3"
PROCESSING_MODEL = "qwen3.8-flash-next"


def required_environment(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"{name} is missing; see CONTRIBUTING.md, on the real test on the Mac")
    return value


@dataclass(frozen=True)
class Paths:
    base: Path

    @property
    def secrets(self) -> Path:
        # Name kept from the earlier, Spanish harness, so the token `prepare`
        # created once is still found.
        return self.base / "secretos"

    @property
    def hermes_token(self) -> Path:
        return self.secrets / "hermes-token"

    @property
    def hub(self) -> Path:
        return self.base / "hub"

    @property
    def config(self) -> Path:
        return self.base / "orbit.toml"

    @property
    def runs(self) -> Path:
        return self.base / "runs"

    @property
    def processes(self) -> Path:
        """The processes a run started and has not stopped yet: one
        `pid<TAB>start time<TAB>name` per line, so that a run killed before it
        could clean up is cleaned up by the next one. The start time (from
        /proc) tells the process apart from any later one given the same pid.
        Its name is kept from the earlier, Spanish harness, so what one of its
        runs left behind is still stopped."""
        return self.base / "procesos"


def write_private(path: Path, content: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as file:
        file.write(content)
    os.chmod(path, 0o600)


def prepare_secrets(paths: Paths) -> list[str]:
    """Creates, once, the Hermes token of the test. Never rewrites it: the
    test Hermes may already be using it."""
    created = []
    if not paths.hermes_token.exists():
        write_private(paths.hermes_token, secrets.token_urlsafe(32).encode())
        created.append(str(paths.hermes_token))
    return created


def config_text(
    paths: Paths,
    host: str,
    hermes_port: int,
    providers: dict | None = None,
    *,
    language: str = "es",
) -> str:
    """The test hub's orbit.toml. `providers` replaces Whisper and the note
    model only in the dry run, which must not call the real ones."""
    p = (
        providers
        if providers is not None
        else {
            "whisper": required_environment("ORBIT_TEST_WHISPER_URL"),
            "processing": required_environment("ORBIT_TEST_PROCESSING_URL"),
            "processing_key": required_environment("ORBIT_TEST_PROCESSING_KEY_FILE"),
        }
    )
    profile = (
        required_environment("ORBIT_TEST_HERMES_PROFILE")
        if providers is None
        else "fixture-assistant"
    )
    diarization = ""
    if providers is None:
        python = required_environment("ORBIT_DIARIZATION_PYTHON")
        bundle = required_environment("ORBIT_DIARIZATION_BUNDLE")
        lock = required_environment("ORBIT_DIARIZATION_GPU_LOCK")
        diarization = (
            f'\n[diarization]\npython = "{python}"\nbundle = "{bundle}"\ngpu_lock = "{lock}"\n'
        )
    if language not in ("es", "en"):
        raise ValueError(f"unsupported transcription language {language!r}")
    return f"""# Test hub of Orbit's real test. scripts/e2e-mac writes it on every run.
data_dir = "{paths.hub / "data"}"
key_file = "{paths.hub / "storage.key"}"
hermes_url = "http://127.0.0.1:{hermes_port}"
hermes_credential_file = "{paths.hermes_token}"
hermes_ws_auth = "token"
profile = "{profile}"
allow_insecure_localhost = true
allowed_hosts = ["{host}"]

[meeting_transcription]
endpoint = "{p["whisper"]}"
model = "{WHISPER_MODEL}"
language = "{language}"

[processing]
endpoint = "{p["processing"]}"
model = "{PROCESSING_MODEL}"
credential_file = "{p["processing_key"]}"
reasoning = "off"

{diarization}
"""


def port_free(host: str, port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            probe.bind((host, port))
        except OSError:
            return False
    return True


def _start_time(pid: int) -> str | None:
    """Field 22 of /proc/<pid>/stat: when the process started, in clock ticks."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except OSError:
        return None
    fields = stat.rsplit(")", 1)[-1].split()
    # A zombie has finished; only its parent has not collected it yet.
    return None if fields[0] == "Z" else fields[19]


def stop_leftovers(paths: Paths, bound_s: float) -> list[str]:
    """Stops what an earlier run started and never stopped: only a pid whose
    start time is the one that run wrote down, and its process group."""
    if not paths.processes.exists():
        return []
    stopped = []
    for line in paths.processes.read_text(encoding="utf-8").splitlines():
        parts = line.split("\t")
        if len(parts) != 3 or not parts[0].isdigit():
            continue
        pid, started, name = int(parts[0]), parts[1], parts[2]
        if _start_time(pid) != started:
            continue
        try:
            os.killpg(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
        deadline = time.monotonic() + bound_s
        while _start_time(pid) == started and time.monotonic() < deadline:
            time.sleep(0.2)
        if _start_time(pid) == started:
            os.killpg(pid, signal.SIGKILL)
        stopped.append(f"{name} ({pid})")
    paths.processes.unlink()
    return stopped


class Process:
    """A process this test started, in a process group of its own, so that
    stopping it reaches everything it started and nothing else."""

    def __init__(
        self,
        name: str,
        command: list[str],
        log: Path,
        environment: dict | None = None,
        ledger: Path | None = None,
    ):
        self.name = name
        log.parent.mkdir(parents=True, exist_ok=True)
        self._output = open(log, "ab")  # noqa: SIM115 - closed in stop()
        self.popen = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=self._output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env=environment,
        )
        self.ledger = ledger
        if ledger is not None:
            with open(ledger, "a", encoding="utf-8") as file:
                file.write(f"{self.popen.pid}\t{_start_time(self.popen.pid)}\t{name}\n")

    def alive(self) -> bool:
        return self.popen.poll() is None

    def stop(self, bound_s: float) -> dict:
        """SIGTERM to its group, the bound to leave, SIGKILL to the group after."""
        forced = False
        if self.alive():
            try:
                os.killpg(self.popen.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.popen.wait(bound_s)
            except subprocess.TimeoutExpired:
                forced = True
                try:
                    os.killpg(self.popen.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.popen.wait(10)
        self._output.close()
        if self.ledger is not None and self.ledger.exists():
            left = [
                line
                for line in self.ledger.read_text(encoding="utf-8").splitlines()
                if not line.startswith(f"{self.popen.pid}\t")
            ]
            self.ledger.write_text("".join(f"{line}\n" for line in left), encoding="utf-8")
        return {
            "process": self.name,
            "code": self.popen.returncode,
            "forced": forced,
        }


def wait_health(url: str, process: Process, bound_s: float) -> float:
    """Until /health answers 200; ends at once if the process dies."""
    start = time.monotonic()
    while True:
        if not process.alive():
            raise RuntimeError(
                f"the test hub exited while starting (code {process.popen.returncode})"
            )
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=2) as reply:
                if reply.status == 200:
                    return time.monotonic() - start
        except OSError:
            pass
        if time.monotonic() - start > bound_s:
            raise RuntimeError(f"the test hub did not answer /health within {bound_s:.0f} s")
        time.sleep(0.2)


def wait_port(host: str, port: int, process: Process, bound_s: float) -> float:
    start = time.monotonic()
    while True:
        if not process.alive():
            raise RuntimeError(
                f"{process.name} exited while starting (code {process.popen.returncode})"
            )
        try:
            socket.create_connection((host, port), timeout=1).close()
            return time.monotonic() - start
        except OSError:
            pass
        if time.monotonic() - start > bound_s:
            raise RuntimeError(f"{process.name} did not open {host}:{port} within {bound_s:.0f} s")
        time.sleep(0.2)


def init_store(paths: Paths) -> None:
    """A fresh store each run: the hub's data of one run is never the next one's."""
    if paths.hub.exists():
        shutil.rmtree(paths.hub)
    paths.hub.mkdir(mode=0o700, parents=True)
    subprocess.run(
        [
            str(PYTHON),
            "-m",
            "orbit_hub.cli",
            "--config",
            str(paths.config),
            "init",
        ],
        cwd=SERVER,
        check=True,
        capture_output=True,
        timeout=60,
    )


def pairing_code(paths: Paths) -> str:
    done = subprocess.run(
        [
            str(PYTHON),
            "-m",
            "orbit_hub.cli",
            "--config",
            str(paths.config),
            "pair-code",
        ],
        cwd=SERVER,
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    return done.stdout.strip()


def start_hub(paths: Paths, host: str, port: int, log: Path) -> Process:
    return Process(
        "test hub",
        [
            str(PYTHON),
            str(Path(__file__).resolve()),
            "serve",
            "--config",
            str(paths.config),
            "--host",
            host,
            "--port",
            str(port),
        ],
        log,
        ledger=paths.processes,
    )


def start_hermes(paths: Paths, port: int, log: Path) -> Process:
    token_variable = required_environment("ORBIT_TEST_HERMES_TOKEN_ENV")
    hermes = os.environ.get("ORBIT_HUB_HERMES", str(Path.home() / ".local" / "bin" / "hermes"))
    environment = dict(os.environ)
    environment[token_variable] = paths.hermes_token.read_text(encoding="utf-8").strip()
    # The flags of the Hermes unit the hub's deployment runs
    # (scripts/tests/hub_harness.py SERVE_ARGS), with the test profile and port.
    return Process(
        "test Hermes",
        [
            hermes,
            "--profile",
            required_environment("ORBIT_TEST_HERMES_PROFILE"),
            "serve",
            "--isolated",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
        ],
        log,
        environment,
        ledger=paths.processes,
    )


# ── The server itself ───────────────────────────────────────────────────────


def tailnet_is_secure(app):
    """Marks requests that arrive from a tailnet address as the secure
    transport they are, for the hub's own transport check."""

    async def wrapped(scope, receive, send):
        client = scope.get("client") or ("",)
        if scope.get("type") in ("http", "websocket"):
            try:
                from_tailnet = ipaddress.ip_address(client[0]) in TAILNET
            except ValueError:
                from_tailnet = False
            if from_tailnet:
                scope = dict(
                    scope,
                    scheme={"http": "https", "ws": "wss"}.get(scope["scheme"], scope["scheme"]),
                )
        await app(scope, receive, send)

    return wrapped


def serve(arguments: list[str]) -> None:
    parser = argparse.ArgumentParser(description="Test hub of Orbit's real test on the Mac")
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, required=True)
    options = parser.parse_args(arguments)
    address = ipaddress.ip_address(options.host)
    if not (address in TAILNET or address.is_loopback):
        raise SystemExit(f"the test hub listens only on the tailnet or loopback, not on {address}")
    import uvicorn

    from orbit_hub.app import create_app
    from orbit_hub.config import Settings

    settings = Settings.load(options.config)
    # The same server options as `orbit-hub serve` (orbit_hub/cli.py).
    uvicorn.run(
        tailnet_is_secure(create_app(settings)),
        host=options.host,
        port=options.port,
        ws_max_size=settings.max_frame_bytes,
        ws_max_queue=16,
        limit_concurrency=64,
        access_log=False,
        proxy_headers=False,
    )


if __name__ == "__main__":
    if sys.argv[1:2] == ["serve"]:
        serve(sys.argv[2:])
    else:
        raise SystemExit("usage: isolated_hub.py serve --config C --host H --port P")
