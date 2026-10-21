"""Shared fixtures and helpers for the hub's end-to-end tests with the real daemon.

The device side speaks the daemon's own socket the way the app does: one JSON
request per line, one JSON response per line (``crates/orbit-ipc/src/ipc.rs``).
"""

import asyncio
import base64
import copy
import json
import os
import re
import socket
import subprocess
from contextlib import asynccontextmanager, nullcontext
from pathlib import Path
from uuid import uuid4

import uvicorn
from fastapi import FastAPI, Request, WebSocket
from starlette.websockets import WebSocketDisconnect

__all__ = (
    "BINARY",
    "DAEMON_E2E_TIMEOUT_SECONDS",
    "RustDevice",
    "SyntheticHermes",
    "native_state_result",
    "stale_binary_cause",
    "tcp_server",
    "until",
)

BINARY = os.environ.get("ORBIT_TEST_BINARY")
BUILD_COMMAND = "cargo build -p orbit-daemon --bin orbit --locked"
# The daemon's protocol version on its socket (`PROTOCOL_VERSION` in orbit-core).
PROTOCOL_VERSION = 1
# Topic frames and keyed pages are bounded by the daemon; this is the harness reader ceiling.
MAX_RESPONSE_BYTES = 16 * 1024 * 1024
# How many offending sources a refusal lists before summarising the rest.
NAMED_SOURCES = 5

_NATIVE_STATE_RESULT = json.loads(
    (
        Path(__file__).resolve().parents[2] / "contracts/fixtures/native/results/hello.json"
    ).read_text()
)


def native_state_result(permissions=None):
    """Answer a state command with the generated native-v1 shape."""
    result = copy.deepcopy(_NATIVE_STATE_RESULT)
    if permissions is not None:
        result["permissions"] = permissions
        result["calendar"]["authorization"] = permissions["calendars"]
    return result


def stale_binary_cause(binary):
    """Why ``binary`` is older than its code, or None (REF-02 A4).

    Cargo writes next to a built binary the list of every source it came from
    (``orbit.d``). A source changed after the build, or gone, means the e2e
    would drive an older daemon than the code under test. A binary without that
    list (a packaged one) was named explicitly and is taken as chosen.
    """
    binary = Path(binary)
    listing = binary.with_suffix(".d")
    if not listing.is_file():
        return None
    built = binary.stat().st_mtime
    text = listing.read_text(encoding="utf-8").replace("\\\n", " ")
    sources = []
    for line in text.splitlines():
        _, colon, dependencies = line.partition(": ")
        if not colon:
            continue
        sources.extend(
            part.replace("\0", " ") for part in dependencies.replace("\\ ", "\0").split()
        )
    newer, missing = [], []
    for source in sources:
        try:
            if Path(source).stat().st_mtime > built:
                newer.append(source)
        except FileNotFoundError:
            missing.append(source)
    if not newer and not missing:
        return None
    causes = []
    if newer:
        causes.append(
            f"{len(newer)} de sus {len(sources)} fuentes cambiaron después de compilarlo "
            f"({', '.join(newer[:NAMED_SOURCES])})"
        )
    if missing:
        causes.append(f"{len(missing)} ya no existen ({', '.join(missing[:NAMED_SOURCES])})")
    return (
        f"el daemon {binary} es anterior a su código: {'; '.join(causes)}. "
        f"Ejecuta `{BUILD_COMMAND}` y repite los e2e"
    )


# These waits observe conditions, so load should delay the test instead of failing it.
DAEMON_E2E_TIMEOUT_SECONDS = 60


async def until(read, predicate, timeout=DAEMON_E2E_TIMEOUT_SECONDS):
    result = None
    try:
        async with asyncio.timeout(timeout):
            while True:
                result = await read()
                if predicate(result):
                    return result
                await asyncio.sleep(0.05)
    except TimeoutError as exc:
        if isinstance(result, dict) and "segments" in result:
            result = {
                **result,
                "segments": [
                    {
                        key: segment.get(key)
                        for key in ("track", "sequence", "last_sequence", "state", "flush_reason")
                    }
                    for segment in result["segments"]
                ],
            }
        raise AssertionError(
            f"Synthetic fixture timed out; last observation: {str(result)[:4000]}"
        ) from exc


@asynccontextmanager
async def tcp_server(app):
    bound = socket.socket()
    bound.bind(("127.0.0.1", 0))
    bound.listen(64)
    port = bound.getsockname()[1]
    config = uvicorn.Config(
        app, log_level="critical", access_log=False, timeout_graceful_shutdown=2
    )
    server = uvicorn.Server(config)
    # Uvicorn must not change the test runner's signal handlers.
    server.capture_signals = nullcontext
    task = asyncio.create_task(server.serve(sockets=[bound]))
    try:
        async with asyncio.timeout(DAEMON_E2E_TIMEOUT_SECONDS):
            while not server.started:
                if task.done():
                    await task
                await asyncio.sleep(0.01)
        yield f"http://127.0.0.1:{port}"
    finally:
        server.should_exit = True
        try:
            await asyncio.wait_for(task, DAEMON_E2E_TIMEOUT_SECONDS)
        finally:
            bound.close()


# DEI-05: every chat turn reaches Hermes behind one line saying where it came
# from, e.g. `[Orbit · Sample Mac · texto]`; the transcript the user sees has none.
TURN_CONTEXT = re.compile(
    r"\[Orbit · (?P<device>[^\n]+) · (?P<input>texto|nota de voz transcrita|"
    r"llamada de voz en curso)\]\n"
)


def split_turn(text):
    """``(context line or None, the user's text)`` of one ``prompt.submit``."""
    context = TURN_CONTEXT.match(text)
    if context is None:
        return None, text
    return context.group(0).rstrip("\n"), text[context.end() :]


class SyntheticHermes:
    """Deliberately minimal protocol peer, not an AI/provider emulator.

    ``prompts`` holds what the user wrote, as before DEI-05; ``contexts`` the
    DEI-05 line each of those turns carried, or None for a turn without one.
    Meeting audio is not Hermes's since P7-06 (see support_synthetic_stt);
    ``audio_calls`` counts interactive dictation only.
    """

    def __init__(self):
        self.app = FastAPI()
        self.prompts = []
        self.contexts = []
        self.audio_calls = 0
        self.interrupted = asyncio.Event()
        self.release = asyncio.Event()
        self.release_privacy = asyncio.Event()

        @self.app.websocket("/api/ws")
        async def ws(ws: WebSocket):
            assert ws.query_params["token"] == "e2e-synthetic-hermes-token"
            await ws.accept()
            await ws.send_json(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": "e2e-fixture-epoch"},
                    },
                }
            )
            writers = []
            send_lock = asyncio.Lock()

            async def send(frame):
                async with send_lock:
                    await ws.send_json(frame)

            async def answer(session, prompt):
                if prompt in {"E2E_SLOW", "E2E_PRIVACY"}:
                    trigger = self.release if prompt == "E2E_SLOW" else self.release_privacy
                    await trigger.wait()
                    final = "OBSOLETE_E2E_RESPONSE"
                else:
                    await asyncio.sleep(0.03)
                    final = "Respuesta final sintética " + prompt
                for kind, payload in (
                    ("message.delta", {"text": "Texto parcial."}),
                    (
                        "message.complete",
                        {"text": final, "status": "error" if prompt == "E2E_ERROR" else "success"},
                    ),
                ):
                    await send(
                        {
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {"type": kind, "session_id": session, "payload": payload},
                        }
                    )

            try:
                while True:
                    request = await ws.receive_json()
                    method, params = request["method"], request.get("params", {})
                    if method == "session.create":
                        assert params["profile"] == "deimos"
                        result = {"session_id": str(uuid4()), "stored_session_id": str(uuid4())}
                    elif method == "prompt.submit":
                        context, prompt = split_turn(params["text"])
                        self.contexts.append(context)
                        self.prompts.append(prompt)
                        result = {"status": "streaming"}
                        writers.append(asyncio.create_task(answer(params["session_id"], prompt)))
                    elif method == "session.interrupt":
                        self.interrupted.set()
                        result = {"status": "interrupted"}
                    else:
                        raise AssertionError(f"Unexpected RPC to synthetic Hermes: {method}")
                    await send({"jsonrpc": "2.0", "id": request["id"], "result": result})
            except WebSocketDisconnect:
                pass
            finally:
                for writer in writers:
                    writer.cancel()
                await asyncio.gather(*writers, return_exceptions=True)

        @self.app.post("/api/audio/transcribe")
        async def transcribe(request: Request):
            assert request.headers["x-hermes-session-token"] == "e2e-synthetic-hermes-token"
            assert request.query_params["profile"] == "deimos"
            payload = await request.json()
            body = base64.b64decode(payload["data_url"].split(",", 1)[1], validate=True)
            assert body.startswith(b"RIFF") and body[8:12] == b"WAVE"
            self.audio_calls += 1
            return {"ok": True, "transcript": "Audio sintético de prueba.", "provider": "fixture"}


class RustDevice:
    def __init__(self, binary, data):
        cause = stale_binary_cause(binary)
        assert cause is None, cause
        self.binary, self.data = binary, data
        self.process = None
        self.reader = None
        self.errors = None
        self.commands = None
        self.events = None
        self.streams = {}
        self.native_commands = []
        self.stderr = b""

    async def request(self, command, ok=True):
        """Send one command (``{"command": ..., ...}``) and return the response.

        ``ok`` asserts the daemon accepted it and names its refusal otherwise.
        """
        request_id = str(uuid4())
        reader, writer = await asyncio.wait_for(
            asyncio.open_unix_connection(str(self.data / "orbit.sock"), limit=MAX_RESPONSE_BYTES),
            DAEMON_E2E_TIMEOUT_SECONDS,
        )
        try:
            frame = {"v": PROTOCOL_VERSION, "id": request_id, "body": command}
            writer.write(json.dumps(frame).encode() + b"\n")
            await writer.drain()
            line = await asyncio.wait_for(reader.readline(), DAEMON_E2E_TIMEOUT_SECONDS)
        finally:
            writer.close()
            await writer.wait_closed()
        response = json.loads(line)
        assert response["v"] == PROTOCOL_VERSION and response["id"] == request_id, response
        if ok:
            assert response["ok"], (
                f"{command['command']} rechazada: {response.get('error')} "
                f"{response.get('refusal')}\ndaemon stderr: {self.stderr.decode()}"
            )
        return response

    async def state(self):
        """Public topic facts and startup counters, as the app reads them."""
        status = (await self.request({"command": "status"}))["status"]
        if status["recovery"]["state"] != "ready":
            return status
        names = ["status", "permissions", "recording", "assistant", "notices"]
        reader, writer = await asyncio.open_unix_connection(
            str(self.data / "orbit.sock"), limit=MAX_RESPONSE_BYTES
        )
        heads = {}
        try:
            request_id = str(uuid4())
            writer.write(
                json.dumps(
                    {
                        "v": PROTOCOL_VERSION,
                        "id": request_id,
                        "body": {"command": "subscribe_topics", "topics": names},
                    }
                ).encode()
                + b"\n"
            )
            await writer.drain()
            reply = json.loads(
                await asyncio.wait_for(reader.readline(), DAEMON_E2E_TIMEOUT_SECONDS)
            )
            assert reply["id"] == request_id and reply["ok"], reply
            while len(heads) < len(names):
                event = json.loads(
                    await asyncio.wait_for(reader.readline(), DAEMON_E2E_TIMEOUT_SECONDS)
                )
                frame = event["frame"]
                if frame["revision"] > 0:
                    data = frame["data"]
                    assert data["knowledge"]["state"] == "present", data
                    heads[data["topic"]] = data["knowledge"]["value"]
        finally:
            writer.close()
            await writer.wait_closed()
        status.update(
            connection=heads["status"]["hub"],
            privacy_stopped=heads["status"]["privacy_stopped"],
            capabilities={"microphone": heads["permissions"]["microphone"] == "granted"},
            recording=heads["recording"]["recording"],
            recording_signal=heads["recording"]["recording_signal"],
            turn=heads["assistant"]["turn"],
            session_id=heads["assistant"]["session_id"],
            active_request="in_flight" if heads["assistant"]["turn_in_flight"] else None,
            notice=heads["notices"]["notice"],
        )
        return status

    async def read(self, read):
        """One keyed read (``{"kind": ...}``), as the app reads: its answer."""
        return (await self.request({"command": "read", "read": read}))["answer"]

    async def emit(self, value):
        self.events.write(json.dumps({"v": 1, **value}).encode() + b"\n")
        await self.events.drain()

    def _relay(self):
        """REF-06: the daemon launches its helper with --helper, as in
        production; a shell relay stands where OrbitNative stands and copies
        the daemon's pipes to two FIFOs this harness owns."""
        # A fresh relay per launch: a restarted daemon launches its own helper,
        # and a FIFO shared with the previous relay could still have that
        # relay's reader on it.
        name = f"helper-{uuid4().hex[:8]}"
        commands = self.data / f"{name}.commands"
        events = self.data / f"{name}.events"
        for fifo in (commands, events):
            os.mkfifo(fifo, 0o600)
        script = self.data / name
        source = self.data / f"{name}.txt"
        source.write_text(f"#!/bin/sh\n/bin/cat '{events}' &\nexec /bin/cat > '{commands}'\n")
        # Written by cp, never held open for writing by this forking process:
        # executing a file someone holds open for writing fails with ETXTBSY.
        subprocess.run(["/bin/cp", str(source), str(script)], check=True)
        script.chmod(0o700)
        return script, commands, events

    async def _connect_relay(self, commands, events):
        loop = asyncio.get_running_loop()
        # Opening a FIFO waits for its other end: off the event loop.
        events_fd = await asyncio.to_thread(os.open, events, os.O_WRONLY)
        commands_fd = await asyncio.to_thread(os.open, commands, os.O_RDONLY)
        self.commands = asyncio.StreamReader(limit=16 * 1024 * 1024)
        await loop.connect_read_pipe(
            lambda: asyncio.StreamReaderProtocol(self.commands),
            os.fdopen(commands_fd, "rb", buffering=0),
        )
        transport, protocol = await loop.connect_write_pipe(
            asyncio.streams.FlowControlMixin, os.fdopen(events_fd, "wb", buffering=0)
        )
        self.events = asyncio.StreamWriter(transport, protocol, None, loop)

    async def start(self):
        # The app gives the daemon a directory that exists; the daemon makes the rest.
        self.data.mkdir(mode=0o700, parents=True, exist_ok=True)
        script, commands, events = self._relay()
        self.process = await asyncio.create_subprocess_exec(
            str(self.binary),
            "--dev",
            "--data-dir",
            str(self.data),
            "daemon",
            "--helper",
            str(script),
            "--parent-pid",
            str(os.getpid()),
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.PIPE,
        )
        self.errors = asyncio.create_task(self._errors())
        connect = asyncio.create_task(self._connect_relay(commands, events))
        exited = asyncio.create_task(self.process.wait())
        await asyncio.wait(
            {connect, exited},
            timeout=DAEMON_E2E_TIMEOUT_SECONDS,
            return_when=asyncio.FIRST_COMPLETED,
        )
        exited.cancel()
        if not connect.done():
            # A daemon that never launched its helper leaves the FIFO opens
            # waiting for ever; opening the other ends releases their threads.
            for _ in range(100):
                if connect.done():
                    break
                for path, flags in (
                    (events, os.O_RDONLY | os.O_NONBLOCK),
                    (commands, os.O_WRONLY | os.O_NONBLOCK),
                ):
                    try:
                        os.close(os.open(path, flags))
                    except OSError:
                        pass
                await asyncio.sleep(0.01)
            await asyncio.gather(connect, return_exceptions=True)
            raise AssertionError(f"el daemon no lanzó su helper de prueba: {self.stderr.decode()}")
        connect.result()
        self.reader = asyncio.create_task(self._native_host())
        ready = json.loads(
            (
                Path(__file__).resolve().parents[2] / "contracts/fixtures/native/events/ready.json"
            ).read_text(encoding="utf-8")
        )
        ready["fixture"] = True
        await self.emit(ready)
        async with asyncio.timeout(DAEMON_E2E_TIMEOUT_SECONDS):
            while not (self.data / "orbit.sock").exists():
                if self.process.returncode is not None:
                    raise AssertionError(self.stderr.decode())
                await asyncio.sleep(0.02)

        async def ready_state():
            try:
                return await self.state()
            except (OSError, ValueError):  # Not listening yet, or closed mid-answer.
                return {}

        await until(
            ready_state,
            lambda state: (
                state.get("capabilities", {}).get("microphone")
                and state.get("recovery", {}).get("state") == "ready"
            ),
        )

    async def _native_host(self):
        while raw := await self.commands.readline():
            command = json.loads(raw)
            self.native_commands.append(command)
            name = command["command"]
            result = {}
            if name in ("hello", "heartbeat", "status"):
                result = native_state_result(
                    {
                        "microphone": "authorized",
                        "screen": True,
                        "accessibility": False,
                        "calendars": "not_determined",
                    }
                )
            elif name == "permissions":
                result = {
                    "microphone": "authorized",
                    "screen": True,
                    "accessibility": False,
                    "calendars": "not_determined",
                }
            elif name == "audio_start":
                self.streams[command["source"]] = command["stream_id"]
            elif name == "audio_stop":
                self.streams = {
                    key: value
                    for key, value in self.streams.items()
                    if value != command["stream_id"]
                }
                result = {"stream_id": command["stream_id"], "drain_complete": True}
            elif name == "privacy_stop":
                self.streams.clear()
                result = native_state_result()
                result.update(privacy_stopped=True, drain_complete=True)
            elif name == "capture_stop":
                # REF-12: the daemon's cut (a failure, closing, a revocation)
                # stops capture without the helper's latch.
                self.streams.clear()
                result = native_state_result()
                result.update(privacy_stopped=False, drain_complete=True)
            await self.emit(
                {"event": "response", "id": command["id"], "ok": True, "result": result}
            )

    async def _errors(self):
        while piece := await self.process.stderr.read(4096):
            self.stderr = (self.stderr + piece)[-16384:]

    async def pcm(self, source, sequence=0):
        await self.emit(
            {
                "event": "audio",
                "source": source,
                "stream_id": self.streams[source],
                "sequence": sequence,
                "sample_rate": 16000,
                "channels": 1,
                "frames": 16000,
                "monotonic_ns": 1000000000 + sequence * 1000000000,
                "clock_domain": "mach_absolute_time",
                # Sound, not digital silence: the hub never sends silence to
                # Whisper (meeting_transcription.trim_digital_silence).
                "pcm_s16le_base64": base64.b64encode(b"\x00\x01" * 16000).decode(),
                "levels": {"rms_dbfs": -120.0, "peak_dbfs": -120.0, "clipped_frames": 0},
                "processing": "unprocessed",
            }
        )

    async def close(self, crash=False):
        if self.process is None:
            return
        if self.process.returncode is None:
            if crash:
                self.process.kill()  # exact child PID: synthetic crash/recovery test
            else:
                try:
                    await self.request({"command": "shutdown"}, ok=False)
                except (OSError, ValueError):
                    pass  # Already gone: the wait below observes the exit.
            try:
                await asyncio.wait_for(self.process.wait(), DAEMON_E2E_TIMEOUT_SECONDS)
            except TimeoutError:
                self.process.kill()
                await self.process.wait()
        if self.reader:
            self.reader.cancel()
        if self.errors:
            self.errors.cancel()
        await asyncio.gather(self.reader, self.errors, return_exceptions=True)
        if self.events is not None:
            # The relay's last cat ends with the harness's end of its FIFO.
            self.events.close()
            self.events = None
        self.process = None
