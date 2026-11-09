"""Explicit acceptance against a real `hermes serve`; never discovered by the ordinary suite.

It needs the private test environment and says so when it is missing:

- ORBIT_HERMES_LIVE=1, so that it never runs by accident;
- ORBIT_TEST_HERMES_PROFILE: an isolated Hermes profile for tests, never the
  live one, whose configuration this test does not change;
- ORBIT_TEST_HERMES_TOKEN_ENV: the variable `hermes serve` reads its session
  token from (the token itself is made here, for this run only);
- ORBIT_HUB_HERMES: the `hermes` launcher, by default ~/.local/bin/hermes.

    ORBIT_HERMES_LIVE=1 timeout 600 server/.venv/bin/python -m pytest -q \\
        -p no:cacheprovider server/tests/acceptance_hermes_requests.py

It starts its own `hermes --profile <test profile> serve --isolated` on
127.0.0.1 and a free port, never one of the fixed ports of this machine's
services. `serve` claims a lock per operating-system user
(gateway/host_rendezvous.py) and may stop an owner it judges orphaned; with
HERMES_GATEWAY_LOCK_DIR in this run's temporary directory it never meets the
production Hermes nor the real test's. The process group it started is
stopped when the test ends, also when it fails, and the lock goes with it.

The hub talks to that Hermes through a relay that records every frame, so the
test sees what the hub wrote and what the real Hermes answered.

End to end, a real Hermes asks the person through the hub, with ORBIT_HERMES_LIVE=1
and no profile: `test_a_real_hermes_asks_the_mac_and_its_agent_hears_the_answers`
runs `serve --isolated` of the installed Hermes (and of Hermes 0.21.4 when
ORBIT_HERMES_LEGACY_PYTHON names its interpreter, see support_hermes_legacy.py)
on a home of its own in this run's temporary directory, against a scripted
OpenAI-compatible model on loopback. The model makes the agent run a command
Hermes asks about, then ask a question; the hub's real runtime shows both to
the Mac, the test answers as the Mac, and checks the effect on a temporary
file and what the agent read back. It runs the interpreter of Hermes's own
committed environment with `hermes_cli.main`, as Hermes's own end-to-end
tests do, not the `hermes` launcher: for a home it has never seen the launcher
first installs a whole dependency environment for it (uv, a Python, every
package, into that home), and with lazy installs off it refuses to start.
Nothing outside loopback is reachable from that Hermes (a proxy that refuses).
"""

import asyncio
import contextlib
import json
import os
import pwd
import re
import secrets
import shlex
import signal
import socket
import subprocess
import threading
import time
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import pytest
from conftest import only
from support_hermes_hub import HubOnPeer
from support_hermes_legacy import LEGACY, LEGACY_PATH, generate_legacy, legacy_hermes
from support_hermes_requests import (
    CONTRACT,
    hermes_checkout,
    hermes_python,
    validate,
    validate_with_hermes,
)
from websockets.asyncio.client import connect
from websockets.asyncio.server import serve

from orbit_hub.assistant.hermes import HermesBackend
from orbit_hub.runtime import DeviceRuntime

# Ports of this machine's own services (production hub and Hermes, Hermes's
# tailnet serve, the real test's hub and Hermes): never this test's.
RESERVED_PORTS = {8642, 8643, 8653, 8700, 8710}
# How long `hermes serve` may take to say it listens: 2.4 to 3.4 s measured
# here, with room for a first start after a Hermes update, which prepares its
# dependencies before it listens. Past it the test fails with the log's tail.
START_SECONDS = 180
# How long its process group has to end after SIGTERM before SIGKILL.
STOP_SECONDS = 20
# A request id Hermes never opened, in its own format.
NEVER_OPENED = CONTRACT["request_frame"]["id"]


def _free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    assert port not in RESERVED_PORTS
    return port


def _stop_group(process: subprocess.Popen) -> None:
    """SIGTERM to the group the test started, SIGKILL to what is left of it.

    The leader is reaped only at the end: until then its group id cannot be
    given to anyone else, so the second signal reaches only this group."""
    with contextlib.suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGTERM)
    deadline = time.monotonic() + STOP_SECONDS
    exited = os.WEXITED | os.WNOHANG | os.WNOWAIT
    while os.waitid(os.P_PID, process.pid, exited) is None and time.monotonic() < deadline:
        time.sleep(0.1)
    with contextlib.suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGKILL)
    process.wait()


def _tail(log: Path, *private: str) -> str:
    text = log.read_text(encoding="utf-8", errors="replace")[-3000:]
    for value in private:
        text = text.replace(value, "<private>")
    return text


@pytest.fixture
def live_hermes(tmp_path):
    if os.environ.get("ORBIT_HERMES_LIVE") != "1":
        pytest.skip("a real Hermes run: set ORBIT_HERMES_LIVE=1 with the private test environment")
    profile = os.environ.get("ORBIT_TEST_HERMES_PROFILE")
    token_variable = os.environ.get("ORBIT_TEST_HERMES_TOKEN_ENV")
    if not profile or not token_variable:
        pytest.skip("ORBIT_TEST_HERMES_PROFILE and ORBIT_TEST_HERMES_TOKEN_ENV are not set")
    assert profile != "deimos", "never the live profile"
    launcher = os.environ.get("ORBIT_HUB_HERMES", str(Path.home() / ".local" / "bin" / "hermes"))
    port, token = _free_port(), secrets.token_urlsafe(32)
    locks = tmp_path / "hermes-locks"
    locks.mkdir(mode=0o700)
    environment = {**os.environ, token_variable: token, "HERMES_GATEWAY_LOCK_DIR": str(locks)}
    log = tmp_path / "hermes-serve.log"
    command = [launcher, "--profile", profile, "serve", "--isolated"]
    command += ["--host", "127.0.0.1", "--port", str(port)]
    with log.open("wb") as output:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            env=environment,
            start_new_session=True,
        )
    started = time.monotonic()
    try:
        while f"HERMES_BACKEND_READY port={port}" not in log.read_text(errors="replace"):
            if process.poll() is not None:
                pytest.fail(
                    f"hermes serve ended ({process.returncode}): {_tail(log, profile, token)}"
                )
            if time.monotonic() - started > START_SECONDS:
                pytest.fail(
                    f"hermes serve did not listen in {START_SECONDS} s: {_tail(log, profile, token)}"
                )
            time.sleep(0.2)
        token_file = tmp_path / "hermes-token"
        token_file.write_text(token, encoding="utf-8")
        token_file.chmod(0o600)
        yield port, token_file, round(time.monotonic() - started, 1)
    finally:
        _stop_group(process)


class Tap:
    """A relay between the hub and Hermes that keeps every text frame both ways."""

    def __init__(self, upstream: int):
        self.upstream = upstream
        self.to_hermes: list[dict] = []
        self.to_hub: list[dict] = []

    async def relay(self, hub_side):
        url = f"ws://127.0.0.1:{self.upstream}{hub_side.request.path}"
        async with connect(url, max_size=None) as hermes_side:

            async def pump(source, sink, kept):
                async for raw in source:
                    if isinstance(raw, str):
                        kept.append(json.loads(raw))
                    await sink.send(raw)

            pumps = [
                asyncio.create_task(pump(hub_side, hermes_side, self.to_hermes)),
                asyncio.create_task(pump(hermes_side, hub_side, self.to_hub)),
            ]
            try:
                await asyncio.wait(pumps, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in pumps:
                    task.cancel()
                await asyncio.gather(*pumps, return_exceptions=True)


async def test_a_real_hermes_takes_the_declaration_and_the_answer_methods(live_hermes, environment):
    port, token_file, started_in = live_hermes
    profile = os.environ["ORBIT_TEST_HERMES_PROFILE"]
    settings, _, _ = environment
    tap = Tap(port)
    async with serve(tap.relay, "127.0.0.1", 0, max_size=None) as relay:
        configured = replace(
            settings,
            profile=profile,
            hermes_url=f"http://127.0.0.1:{relay.sockets[0].getsockname()[1]}",
            allow_insecure_localhost=True,
            hermes_credential_file=token_file,
        )

        async def ignore(_event):
            pass

        connection = HermesBackend(configured, profile).connection("live-device", "Prueba", ignore)
        try:
            await connection.open()
            sent = [frame.get("method") for frame in tap.to_hermes]
            assert sent[:1] == ["client.capabilities"], f"the hub opened with {sent}"
            declaration = tap.to_hermes[0]
            assert declaration["params"] == {"server_requests": True}
            reply = next(frame for frame in tap.to_hub if frame.get("id") == declaration["id"])
            assert "result" in reply, f"the real Hermes refused the declaration: {reply}"
            validate("client.capabilities.result", reply["result"])
            assert {"approval", "clarify"} <= set(reply["result"]["server_requests"])

            assert (await connection.create_session()).session_id
            answered = await connection.rpc(
                "request.answer", {"id": NEVER_OPENED, "result": {"choice": "deny"}}
            )
            assert answered == {"status": "expired"}
            locked = await connection.rpc(
                "clarify.lock", {"request_id": NEVER_OPENED, "question_id": "q", "answer": "x"}
            )
            assert locked == {"status": "expired"}
        finally:
            await connection.close()
    print(f"hermes serve listened after {started_in} s")


# ── End to end: a real Hermes asks the Mac through the hub ──────────────────

# Waits for the next step of a turn (the agent starts, calls the model, runs
# the tool); never a measurement. The Hermes under test waits 120 s for answers.
TURN_SECONDS = 120
MODEL = "orbit-script"
# The tag in the person's text that tells the scripted model which turn it is.
TAG = re.compile(r"\bORBIT-(ONCE|DENY|ASK)\b")
# The question the agent asks, and what the Mac answers.
QUESTION, CHOICES, ANSWER = "¿De qué color es la portada?", ["Rojo", "Azul"], "Azul"
# What the Hermes under test reads: the scripted model, the terminal on this
# machine, every dangerous command asked (`manual`), and no metrics sent.
CONFIGURATION = """\
model:
  provider: custom
  base_url: {base_url}
  default: {model}
  context_length: 128000
agent:
  api_max_retries: 1
approvals:
  mode: manual
  timeout: 120
clarify:
  timeout: 120
terminal:
  backend: local
telemetry:
  shared_metrics:
    enabled: false
    send: false
"""
# A proxy nobody listens on: whatever that Hermes tries beyond loopback fails at once.
NO_NETWORK = "http://127.0.0.1:9"


def _text(message: dict) -> str:
    content = message.get("content")
    if isinstance(content, list):
        return "".join(part.get("text", "") for part in content if isinstance(part, dict))
    return content if isinstance(content, str) else ""


class ScriptedModel:
    """An OpenAI-compatible chat model on loopback whose turns are a script.

    `turns[tag]` is the tool call `(name, arguments)` the agent makes for the
    person's text tagged `ORBIT-<tag>`; once the tool answered, the model says
    `Hecho <tag>`. Requests without tools (a title) get a short text. What each
    tool answered the agent is kept in `results`, by tag."""

    def __init__(self, turns: dict[str, tuple[str, dict]]):
        self.turns = turns
        self.results: dict[str, list[str]] = {}
        self._calls = 0
        self._lock = threading.Lock()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), _model_handler(self))
        self._server.daemon_threads = True
        threading.Thread(target=self._server.serve_forever, daemon=True).start()

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_address[1]}/v1"

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()

    def answer(self, body: dict) -> dict:
        messages = body.get("messages") or []
        users = [message for message in messages if message.get("role") == "user"]
        found = TAG.search(_text(users[-1])) if users and body.get("tools") else None
        if found is None:
            return {"role": "assistant", "content": "Prueba de Orbit"}
        tag = found.group(1)
        with self._lock:
            if messages[-1].get("role") == "tool":
                self.results.setdefault(tag, []).append(_text(messages[-1]))
                return {"role": "assistant", "content": f"Hecho {tag}"}
            self._calls += 1
            call = f"call_orbit_{self._calls}"
        name, arguments = self.turns[tag]
        function = {"name": name, "arguments": json.dumps(arguments, ensure_ascii=False)}
        return {
            "role": "assistant",
            "content": None,
            "tool_calls": [{"id": call, "type": "function", "function": function}],
        }


def _model_handler(model: ScriptedModel):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_arguments):
            pass

        def _send(self, status: int, kind: str, body: bytes) -> None:
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
            self.close_connection = True

        def do_GET(self):
            if self.path.rstrip("/").endswith("/models"):
                listing = {"object": "list", "data": [{"id": MODEL, "object": "model"}]}
                self._send(200, "application/json", json.dumps(listing).encode())
            else:
                self._send(404, "application/json", b'{"error": {"message": "not found"}}')

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or "{}")
            if not self.path.rstrip("/").endswith("/chat/completions"):
                self._send(404, "application/json", b'{"error": {"message": "not found"}}')
                return
            message = model.answer(body)
            finish = "tool_calls" if message.get("tool_calls") else "stop"
            usage = {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}
            head = {"id": "chatcmpl-orbit", "created": 1, "model": MODEL}
            if not body.get("stream"):
                choice = {"index": 0, "message": message, "finish_reason": finish}
                reply = {**head, "object": "chat.completion", "choices": [choice], "usage": usage}
                self._send(200, "application/json", json.dumps(reply).encode())
                return
            deltas = [{"role": "assistant", "content": message.get("content") or ""}]
            for index, call in enumerate(message.get("tool_calls") or []):
                deltas.append({"tool_calls": [{"index": index, **call}]})
            chunks = [
                {**head, "object": "chat.completion.chunk", "choices": [choice]}
                for choice in (
                    *({"index": 0, "delta": delta, "finish_reason": None} for delta in deltas),
                    {"index": 0, "delta": {}, "finish_reason": finish},
                )
            ]
            chunks[-1]["usage"] = usage
            stream = "".join(f"data: {json.dumps(chunk)}\n\n" for chunk in chunks)
            self._send(200, "text/event-stream", (stream + "data: [DONE]\n\n").encode())

    return Handler


@contextlib.contextmanager
def _hermes_serving(python: Path, checkout: Path, root: Path, model: ScriptedModel):
    """`serve --isolated` of the Hermes at `checkout` on a home of its own under
    `root`, until the block ends: its port, token file and seconds to listen."""
    home = root / "home"
    hermes_home = home / ".hermes"
    operator = (Path(pwd.getpwuid(os.getuid()).pw_dir) / ".hermes").resolve()
    assert operator not in (hermes_home.resolve(), *hermes_home.resolve().parents)
    for directory in (hermes_home, root / "locks", root / "tmp"):
        directory.mkdir(parents=True, mode=0o700)
    configuration = CONFIGURATION.format(base_url=model.base_url, model=MODEL)
    (hermes_home / "config.yaml").write_text(configuration, encoding="utf-8")
    port, token = _free_port(), secrets.token_urlsafe(32)
    environment = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL") if key in os.environ}
    environment.update(
        HOME=str(home),
        HERMES_HOME=str(hermes_home),
        TMPDIR=str(root / "tmp"),
        PYTHONPATH=str(checkout),
        HERMES_DASHBOARD_SESSION_TOKEN=token,
        HERMES_GATEWAY_LOCK_DIR=str(root / "locks"),
        # It runs on its own environment's packages and installs nothing
        # anywhere, as Hermes's own test suite runs.
        HERMES_DISABLE_LAZY_INSTALLS="1",
        # Hermes refuses its default home's state.db under a pytest ancestor;
        # this home is the run's own (asserted above), as in Hermes's own tests.
        HERMES_STATE_DB_GUARD_BYPASS="1",
        PYTHONDONTWRITEBYTECODE="1",
        PYTHONUNBUFFERED="1",
        NO_COLOR="1",
        OPENAI_API_KEY="orbit-scripted-model",
        NO_PROXY="127.0.0.1,localhost",
        no_proxy="127.0.0.1,localhost",
        **{name: NO_NETWORK for name in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY")},
        **{name: NO_NETWORK for name in ("http_proxy", "https_proxy", "all_proxy")},
    )
    log = root / "serve.log"
    command = [str(python), "-B", "-m", "hermes_cli.main", "serve", "--isolated"]
    command += ["--host", "127.0.0.1", "--port", str(port)]
    with log.open("wb") as output:
        process = subprocess.Popen(
            command,
            cwd=root,
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            env=environment,
            start_new_session=True,
        )
    started = time.monotonic()
    try:
        while f"HERMES_BACKEND_READY port={port}" not in log.read_text(errors="replace"):
            if process.poll() is not None:
                pytest.fail(f"hermes serve ended ({process.returncode}): {_tail(log, token)}")
            if time.monotonic() - started > START_SECONDS:
                pytest.fail(
                    f"hermes serve did not listen in {START_SECONDS} s: {_tail(log, token)}"
                )
            time.sleep(0.2)
        token_file = root / "hermes-token"
        token_file.write_text(token, encoding="utf-8")
        token_file.chmod(0o600)
        yield port, token_file, round(time.monotonic() - started, 1)
    finally:
        _stop_group(process)


class HubOnHermes(HubOnPeer):
    """The hub's real runtime and Hermes connection, in front of a real Hermes."""

    def __init__(self, environment, tmp_path, port: int, token_file: Path):
        super().__init__(environment, tmp_path, peer=None)
        self.port, self.token_file = port, token_file

    async def start(self) -> "HubOnHermes":
        # "default": the profile of a home without named profiles, which
        # Hermes reads as the home it was started on.
        settings = replace(
            self.settings,
            profile="default",
            hermes_url=f"http://127.0.0.1:{self.port}",
            allow_insecure_localhost=True,
            hermes_credential_file=self.token_file,
        )
        backend = HermesBackend(settings, "default")
        self.runtime = DeviceRuntime(self.owner, self.store, only(backend), lambda: 1001)
        self.queue = self.runtime.subscribe()
        return self

    async def asked_or_ended(self, card: str, session_id: str) -> dict:
        """The first frame of the conversation that is `card` or the turn's end:
        a Hermes that withdraws its question ends the turn without asking."""

        def wanted(frame: dict) -> bool:
            return (
                frame["event"] in (card, "chat.completed")
                and frame["data"].get("session_id") == session_id
            )

        for index, frame in enumerate(self._unclaimed):
            if wanted(frame):
                return self._unclaimed.pop(index)
        try:
            async with asyncio.timeout(TURN_SECONDS):
                while True:
                    frame = await self.queue.get()
                    assert frame is not None, "the hub closed the Mac's stream"
                    self.seen.append(frame)
                    if wanted(frame):
                        return frame
                    self._unclaimed.append(frame)
        except TimeoutError as error:
            got = [frame["event"] for frame in self.seen]
            raise AssertionError(
                f"neither {card} nor the turn's end within {TURN_SECONDS} s; the Mac got {got}"
            ) from error


@pytest.fixture(params=["installed", "0.21.4"])
def scripted_hermes(request, tmp_path):
    if os.environ.get("ORBIT_HERMES_LIVE") != "1":
        pytest.skip("a real Hermes run: set ORBIT_HERMES_LIVE=1")
    if request.param == "installed":
        python, why = hermes_python()
        checkout, contract = hermes_checkout(), CONTRACT
        question = {"questions": [{"question": QUESTION, "choices": CHOICES}]}
    else:
        python, checkout, why = legacy_hermes()
        contract = LEGACY
        # The older call that Hermes still takes: a lone `question`.
        question = {"question": QUESTION, "choices": CHOICES}
    if python is None:
        pytest.skip(why)
    files = {tag: tmp_path / f"{tag.lower()}.txt" for tag in ("ONCE", "DENY")}
    for path in files.values():
        path.write_text("Orbit\n", encoding="utf-8")
        path.chmod(0o600)
    # A command Hermes asks about ("world/other-writable permissions") whose
    # only effect is the mode of a file of this run.
    turns = {
        tag: ("terminal", {"command": f"chmod 666 {shlex.quote(str(path))}"})
        for tag, path in files.items()
    }
    turns["ASK"] = ("clarify", question)
    model = ScriptedModel(turns)
    root = tmp_path / "hermes"
    try:
        with _hermes_serving(python, checkout, root, model) as (port, token_file, started_in):
            yield SimpleNamespace(
                which=request.param,
                python=python,
                checkout=checkout,
                contract=contract,
                files=files,
                model=model,
                port=port,
                token_file=token_file,
                started_in=started_in,
                home=root / "home" / ".hermes",
            )
    finally:
        model.close()


def _user_response(result: str):
    """What the person answered, as the clarify tool told the agent."""
    try:
        payload = json.loads(result)
    except ValueError:
        return None
    if "responses" in payload:
        return payload["responses"][0].get("user_response")
    return payload.get("user_response")


def _size(home: Path) -> tuple[int, list[str]]:
    total = sum(path.stat().st_size for path in home.rglob("*") if path.is_file())
    return total, sorted(path.name for path in home.iterdir())


async def _turn(hub: HubOnHermes, model: ScriptedModel, tag: str, card: str) -> tuple[str, dict]:
    session = await hub.send(f"ORBIT-{tag}: prueba de Orbit")
    frame = await hub.asked_or_ended(card, session)
    assert frame["event"] == card, (
        f"the turn ended without asking the Mac ({card}); the agent read {model.results.get(tag)}"
    )
    return session, frame["data"]


async def _completed(hub: HubOnHermes, session: str, tag: str) -> None:
    done = await hub.frame(
        "chat.completed", lambda data: data["session_id"] == session, TURN_SECONDS
    )
    assert f"Hecho {tag}" in done["data"]["text"], done["data"]


async def test_a_real_hermes_asks_the_mac_and_its_agent_hears_the_answers(
    scripted_hermes, environment, tmp_path
):
    hermes = scripted_hermes
    tap = Tap(hermes.port)
    timings = {}
    async with serve(tap.relay, "127.0.0.1", 0, max_size=None) as relay:
        port = relay.sockets[0].getsockname()[1]
        hub = await HubOnHermes(environment, tmp_path, port, hermes.token_file).start()
        try:
            for tag, choice in (("ONCE", "once"), ("DENY", "deny")):
                began = time.monotonic()
                session, required = await _turn(hub, hermes.model, tag, "approval.required")
                assert {"once", "deny"} <= set(required["choices"]), required
                name, receipt = await hub.approve(session, required["request_id"], choice)
                assert name == "interaction.resolved" and receipt["applied"] is True, receipt
                await _completed(hub, session, tag)
                timings[tag] = round(time.monotonic() - began, 1)

            began = time.monotonic()
            session, required = await _turn(hub, hermes.model, "ASK", "clarify.required")
            assert required["question"] == QUESTION and "questions" not in required, required
            assert ANSWER in required["choices"], required
            name, receipt = await hub.clarify(session, required["request_id"], ANSWER)
            assert name == "interaction.resolved" and receipt["applied"] is True, receipt
            await _completed(hub, session, "ASK")
            timings["ASK"] = round(time.monotonic() - began, 1)
        finally:
            await hub.close()

    # The person's answers decided: `once` ran the command, `deny` did not,
    # and the agent read each answer back.
    assert hermes.files["ONCE"].stat().st_mode & 0o777 == 0o666
    assert hermes.files["DENY"].stat().st_mode & 0o777 == 0o600
    results = hermes.model.results
    assert len(results["ONCE"]) == 1 and "approved by the user" in results["ONCE"][0], results
    assert len(results["DENY"]) == 1 and "denied by user" in results["DENY"][0], results
    assert len(results["ASK"]) == 1 and _user_response(results["ASK"][0]) == ANSWER, results

    # On the wire: the declaration first; Hermes's requests as its contract
    # says; each answer by the method that Hermes reads, and its receipt `ok`.
    sent = [frame.get("method") for frame in tap.to_hermes]
    assert sent[0] == "client.capabilities", sent
    assert "approval.respond" not in sent and "clarify.respond" not in sent, sent
    refused = [frame for frame in tap.to_hermes if "method" not in frame and "error" in frame]
    assert refused == [], refused
    asked = [frame for frame in tap.to_hub if frame.get("method") in ("approval", "clarify")]
    assert [frame["method"] for frame in asked] == ["approval", "approval", "clarify"], asked
    for frame in asked:
        validate(f"server_request.{frame['method']}.params", frame["params"], hermes.contract)
    once, deny, clarification = asked
    expected = [
        {"id": once["id"], "result": {"choice": "once"}},
        {"id": deny["id"], "result": {"choice": "deny"}},
    ]
    written = [("client.capabilities.params", tap.to_hermes[0]["params"])]
    written += [("server_request.approval.result", answer["result"]) for answer in expected]
    questions = clarification["params"].get("questions")
    if questions:
        [lock] = [
            frame["params"] for frame in tap.to_hermes if frame.get("method") == "clarify.lock"
        ]
        assert lock == {
            "request_id": clarification["id"],
            "question_id": questions[0]["qid"],
            "answer": ANSWER,
        }
        written.append(("clarify.lock.params", lock))
    else:
        assert "clarify.lock" not in sent, sent
        expected.append({"id": clarification["id"], "result": {"answer": ANSWER}})
        written.append(("server_request.clarify.result", {"answer": ANSWER}))
    answers = [
        frame["params"] for frame in tap.to_hermes if frame.get("method") == "request.answer"
    ]
    assert answers == expected
    written += [("request.answer.params", answer) for answer in answers]
    receipts = {frame.get("id"): frame for frame in tap.to_hub if "method" not in frame}
    for frame in tap.to_hermes:
        if frame.get("method") in ("request.answer", "clarify.lock"):
            assert receipts[frame["id"]]["result"]["status"] == "ok", receipts[frame["id"]]
    problems = await asyncio.to_thread(
        validate_with_hermes, hermes.python, written, hermes.checkout
    )
    assert problems == []

    size, entries = _size(hermes.home)
    print(
        f"{hermes.which}: serve listened after {hermes.started_in} s; turns {timings} s; "
        f"its new home holds {size / 1e6:.1f} MB in {entries}"
    )


def test_the_older_contract_is_what_that_hermes_declares():
    if os.environ.get("ORBIT_HERMES_LIVE") != "1":
        pytest.skip("a real Hermes run: set ORBIT_HERMES_LIVE=1")
    python, checkout, why = legacy_hermes()
    if python is None:
        pytest.skip(why)
    assert generate_legacy(python, checkout) == LEGACY_PATH.read_text(encoding="utf-8")
