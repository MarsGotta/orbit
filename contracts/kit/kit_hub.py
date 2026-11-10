"""Start a hub as a program and reach it only through its address.

The command that runs the hub comes from `ORBIT_KIT_HUB`, split as a shell
would; without it, today's Python hub. The kit appends `--config <file>
<command> [arguments]`, as the hub's administrator commands take them, and
talks to a serving hub over HTTP as a device it pairs itself (`pair-code`,
then `POST /v1/pair`), with that device's bearer token. Nothing
here imports the hub, so the same kit tests a hub written in any language.

The configuration is invented and names the keys today's hub reads: a
loopback hub without TLS, meeting transcription pointed at an invented service
the kit starts on loopback when it is given one (else at a closed port), one
API agent (`cedar`, of kind openai) whose model is behind that closed port, a
Hermes address and credential that nothing uses, and the key ids of the device
bridge and of the meetings bridge, each with a key the kit draws anew for each
folder it prepares. The kit signs a private request with the key of its
protocol as `contracts/hub-v1.md` writes it («Private meetings for the
assistant»). Asked to, it writes the configuration without the permission
instead (`allow_insecure_localhost = false`) and every address it names HTTPS
on `example.test`: such a hub must refuse to serve without TLS, so nothing
calls them.

A meeting is recorded as a device records it: one chunk of microphone audio
over `PUT /v1/chunks/{id}` with its capture headers, then its close. The
invented transcription service answers `POST /v1/audio/transcriptions` as
OpenAI's API does, with the text it was started with; it runs in a thread of
the kit's process, so it ends with the kit whatever happens.

The hub writes a meeting's note with the notes model its configuration names
(`[processing]`). The invented notes model the kit starts on loopback, in a
thread too, answers `POST /v1/chat/completions` as OpenAI's API does: it fills
the JSON schema the hub sends (`response_format.json_schema.schema`) with
every property of each object, one element in each list and the text it was
started with in anything else. Asked for a vault proposal (the schema named
`orbit_distillation`), it answers a valid one instead: that text as its
summary, the first transcript line the hub sent quoted whole, and one new
`meeting` note, titled after the meeting, adding that line's text.

The API agent answers a turn through the model its configuration names
(`base_url`). The invented agent the kit starts on loopback, in a thread too,
answers `POST /v1/chat/completions` with `stream: true` as OpenAI's API does,
in server-sent events, always with the text it was started with. The hub runs
an API agent beside its orbit-mcp, so the kit writes a valid configuration for
it: the hub's address behind the closed port, the device bridge's key of the
hub's folder and an invented device. A turn without tools never calls it. A
device holds a turn over `GET /v1/ws` with its bearer token: a `chat.send` to
the agent, then the events of that turn until `chat.completed`. The files of a
turn are uploaded before it (`POST /v1/attachments`) and named by its
`chat.send`; the kit writes its PDF by hand, so it needs nothing else.
"""

import hashlib
import hmac
import io
import json
import math
import os
import secrets
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import uuid
import wave
from collections.abc import Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import quote

import httpx
import pytest
from websockets.sync.client import connect

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
# The operator's meeting transcription model, behind the closed port.
OPERATOR_TRANSCRIPTION_MODEL = "invented-whisper"
# The private protocols, as the contract signs them: each with its section of
# the configuration, its headers' prefix, its key id and the file of the hub's
# folder that holds its key.
BRIDGE_PROTOCOL = "orbit_device_bridge_v1"
MEETINGS_PROTOCOL = "orbit_meetings_bridge_v1"
PRIVATE_PROTOCOLS = {
    BRIDGE_PROTOCOL: ("device_bridge", "x-orbit-bridge-", "contract-kit", "bridge-credential"),
    MEETINGS_PROTOCOL: (
        "meetings_bridge",
        "x-orbit-meetings-",
        "contract-kit-meetings",
        "meetings-credential",
    ),
}
PRIVATE_KEY_BYTES = 32
# The variables that name the hub's implicit agent: beside `[[agents]]` the hub refuses to start.
IMPLICIT_AGENT_VARIABLES = ("ORBIT_HUB_PROFILE", "ORBIT_HUB_ASSISTANT_NAME")
# A recorded meeting: one chunk of the local microphone, 16-bit mono WAV. A
# tone, not silence: the hub sends no audio that is only digital silence.
MEETING_TRACK = "mic_local"
MEETING_SECONDS = 2
MEETING_SAMPLE_RATE = 16000
TONE_HERTZ = 440
TONE_AMPLITUDE = 8000
# The meeting's capture clock, as a device's: an arbitrary monotonic origin.
MEETING_MONOTONIC_NS = 1_000_000_000
# The hub closed the measured meeting's transcription in under 2 s; thirty
# times that, so only a hub that never transcribes fails.
TRANSCRIPT_SECONDS = 60
TRANSCRIPT_POLL_SECONDS = 0.1
# A final pass in these states will still move on its own.
FINAL_PASS_UNDER_WAY = ("queued", "running")
# What the invented transcription service says its language is.
SERVICE_LANGUAGE = "en"
# The kit's own meeting, on the device it pairs: another id than the sample's.
KIT_MEETING_ID = "0b9d7e5f-3c1a-4e2d-8f6b-7a9c1e3d5b2f"
KIT_MEETING_STARTED_AT_MS = 1767261600000
# The invented notes model, as the hub's configuration names it.
NOTES_MODEL = "invented-notes-model"
# The hub wrote the measured meeting's note in the same second as its
# transcript; as long as a transcript is waited for.
NOTE_SECONDS = 60
NOTE_POLL_SECONDS = 0.1
# A vault proposal in these states will still move on its own.
PROPOSAL_UNDER_WAY = ("queued", "preparing", "applying")
# The schema name the hub asks a vault proposal with.
DISTILLATION_SCHEMA = "orbit_distillation"
# The invented device orbit-mcp would act on: no tool runs under the kit.
TOOLS_DEVICE = ("Invented Mac", "invented-device")
# The hub answered the measured turn in under 2 s, orbit-mcp's start included;
# thirty times that, so only a hub that never answers fails.
TURN_SECONDS = 60
# The ids the invented agent gives its streamed answer.
AGENT_ANSWER_ID = "invented-answer"
AGENT_CREATED = 1767225600


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


def prepare(
    folder: Path,
    key: bytes,
    database: Path | None = None,
    transcription: str | None = None,
    model: str | None = None,
    agent: str | None = None,
    insecure_localhost: bool = True,
) -> Path:
    """A hub's folder: data 0700, the key in a file of its own (0600), a new
    key for each private protocol (0600), an invented configuration and, when
    given, a copy of `database` as the hub's, meeting transcription at the
    `transcription` service's URL, the notes model at the `model` URL (with
    an invented credential, 0600) and the API agent's model at the `agent`
    base URL. With `insecure_localhost` false, the configuration holds no
    permission and the addresses not given are HTTPS on `example.test`.
    Answers the configuration's path."""
    folder.mkdir(mode=0o700, parents=True, exist_ok=True)
    data, home = folder / "data", folder / "home"
    data.mkdir(mode=0o700)
    home.mkdir(mode=0o700)
    key_file = folder / "storage-key.bin"
    _secret_file(key_file, key)
    credential = folder / "hermes-credential"
    _secret_file(credential, b"invented-hermes-credential\n")
    permission = "true" if insecure_localhost else "false"
    # Where the addresses not given point: never answered.
    unused = (
        f"http://127.0.0.1:{CLOSED_PORT}" if insecure_localhost else "https://unused.example.test"
    )
    private = []
    for section, _, key_id, private_key_file in PRIVATE_PROTOCOLS.values():
        _secret_file(folder / private_key_file, secrets.token_bytes(PRIVATE_KEY_BYTES))
        private += (
            f"[{section}]",
            f"key_id = {_toml(key_id)}",
            f"credential_file = {_toml(folder / private_key_file)}",
        )
    # The agent's orbit-mcp configuration: valid, so orbit-mcp starts beside a
    # turn; the hub's address is never answered, since no tool runs.
    tools = folder / "orbit-mcp.toml"
    bridge_section, _, bridge_key_id, bridge_key_file = PRIVATE_PROTOCOLS[BRIDGE_PROTOCOL]
    tools.write_text(
        "\n".join(
            (
                f"hub_url = {_toml(unused)}",
                f"default_device = {_toml(TOOLS_DEVICE[0])}",
                f"[{bridge_section}]",
                f"key_id = {_toml(bridge_key_id)}",
                f"key_file = {_toml(folder / bridge_key_file)}",
                "[devices]",
                f"{_toml(TOOLS_DEVICE[0])} = {_toml(TOOLS_DEVICE[1])}",
            )
        )
        + "\n"
    )
    processing = []
    if model is not None:
        model_credential = folder / "notes-model-credential"
        _secret_file(model_credential, b"invented-notes-credential\n")
        processing = [
            "[processing]",
            f"endpoint = {_toml(model)}",
            f"model = {_toml(NOTES_MODEL)}",
            f"credential_file = {_toml(model_credential)}",
            f"allow_insecure_localhost = {permission}",
        ]
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
                f"hermes_url = {_toml(unused)}",
                f"hermes_credential_file = {_toml(credential)}",
                f"allow_insecure_localhost = {permission}",
                "[meeting_transcription]",
                f"endpoint = {_toml(transcription or unused)}",
                f"model = {_toml(OPERATOR_TRANSCRIPTION_MODEL)}",
                *private,
                *processing,
                "[[agents]]",
                f"id = {_toml(AGENT_ID)}",
                f"name = {_toml(AGENT_NAME)}",
                'kind = "openai"',
                f"base_url = {_toml(agent or f'{unused}/v1')}",
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


def signed_headers(
    configuration: Path, path: str, body: bytes = b"", protocol: str = BRIDGE_PROTOCOL
) -> dict[str, str]:
    """The headers of `protocol` for a request to `path` with `body`, signed
    now with that protocol's key of the hub's folder: seven lines (the
    protocol, `POST` whatever the method, the path, the key id, the time in
    Unix milliseconds, a UUID nonce, the body's SHA-256), HMAC-SHA256 in
    lowercase hexadecimal."""
    _, headers, key_id, key_file = PRIVATE_PROTOCOLS[protocol]
    key = (configuration.parent / key_file).read_bytes()
    issued_at_ms, nonce = str(time.time_ns() // 1_000_000), str(uuid.uuid4())
    lines = (
        protocol,
        "POST",
        path,
        key_id,
        issued_at_ms,
        nonce,
        hashlib.sha256(body).hexdigest(),
    )
    signature = hmac.new(key, "\n".join(lines).encode(), hashlib.sha256).hexdigest()
    values = {
        "key-id": key_id,
        "issued-at-ms": issued_at_ms,
        "nonce": nonce,
        "signature": signature,
    }
    return {f"{headers}{name}": value for name, value in values.items()}


def device_client(url: str, token: str) -> httpx.Client:
    """HTTP as a paired device: its credential in `Authorization: Bearer`."""
    return httpx.Client(base_url=url, headers={"Authorization": f"Bearer {token}"}, timeout=30)


def transcription_answer(text: str) -> dict:
    """What the invented service answers: the text, its language and its
    words spread evenly over the meeting."""
    words = text.split()
    step = MEETING_SECONDS / len(words)
    return {
        "text": text,
        "language": SERVICE_LANGUAGE,
        "words": [
            {"start": round(index * step, 3), "end": round((index + 1) * step, 3), "word": word}
            for index, word in enumerate(words)
        ],
    }


class _TranscriptionHandler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path != "/v1/audio/transcriptions":
            self.send_error(404)
            return
        body = json.dumps(transcription_answer(self.server.text)).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *arguments) -> None:
        pass


def filled(schema: dict, text: str):
    """A value for `schema`: every property of an object, one element in a
    list, `text` in anything else."""
    if schema.get("type") == "object":
        return {name: filled(inner, text) for name, inner in schema["properties"].items()}
    if schema.get("type") == "array":
        return [filled(schema["items"], text)]
    return text


def distillation(request: dict, text: str) -> dict:
    """A valid vault proposal for the meeting the hub sent (as JSON in its last
    message): `text` as its summary, the transcript's first line quoted whole,
    and one new `meeting` note titled after the meeting whose one added line is
    that quote's text."""
    meeting = json.loads(request["messages"][-1]["content"])["meeting"]
    line = meeting["lines"][0]
    quote = {"id": "q1", "at_ms": line["at_ms"], "speaker": line["speaker"], "text": line["text"]}
    cited = [quote["id"]]
    note = {
        "path": f"{meeting['title']}.md",
        "title": meeting["title"],
        "properties": [{"key": "type", "after": "meeting", "quote_ids": cited}],
        "lines": [{"section": "", "before": "", "after": line["text"], "quote_ids": cited}],
    }
    return {"summary": text, "quotes": [quote], "changes": [note]}


def completion_answer(request: dict, text: str) -> dict:
    """What the invented notes model answers a `chat/completions` request: a
    vault proposal when the schema is `DISTILLATION_SCHEMA`, else the schema it
    was sent filled with `text`; as one finished choice."""
    form = request["response_format"]["json_schema"]
    if form.get("name") == DISTILLATION_SCHEMA:
        content = json.dumps(distillation(request, text))
    else:
        content = json.dumps(filled(form["schema"], text))
    return {
        "model": request.get("model", NOTES_MODEL),
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
            }
        ],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    }


class _ModelHandler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        try:
            answer = completion_answer(json.loads(raw), self.server.text)
        except (ValueError, KeyError, TypeError, AttributeError):
            self.send_error(400)
            return
        body = json.dumps(answer).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *arguments) -> None:
        pass


def _chunk(delta: dict, finish_reason: str | None = None, usage: dict | None = None) -> dict:
    chunk = {
        "id": AGENT_ANSWER_ID,
        "object": "chat.completion.chunk",
        "created": AGENT_CREATED,
        "model": "invented-model",
        "choices": []
        if usage is not None
        else [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    }
    if usage is not None:
        chunk["usage"] = usage
    return chunk


def streamed_answer(text: str) -> bytes:
    """What the invented agent streams: `text`, the end of its one choice, the
    usage, then `[DONE]`, as server-sent events."""
    chunks = (
        _chunk({"role": "assistant", "content": text}),
        _chunk({}, "stop"),
        _chunk({}, usage={"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}),
    )
    events = [f"data: {json.dumps(chunk)}\n\n" for chunk in chunks] + ["data: [DONE]\n\n"]
    return "".join(events).encode()


class _AgentHandler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        try:
            streamed = json.loads(raw).get("stream") is True
        except (ValueError, AttributeError):
            streamed = False
        if not streamed:
            self.send_error(400)
            return
        body = streamed_answer(self.server.text)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *arguments) -> None:
        pass


@contextmanager
def _service(handler: type[BaseHTTPRequestHandler], text: str) -> Iterator[str]:
    """`handler` on a free loopback port, in a thread of this process,
    answering with `text`; yields its base URL."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    server.text = text
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@contextmanager
def transcription_service(text: str) -> Iterator[str]:
    """An invented transcription service answering `text` to every audio;
    yields its URL."""
    with _service(_TranscriptionHandler, text) as url:
        yield url


@contextmanager
def notes_model(text: str) -> Iterator[str]:
    """An invented notes model writing `text` in every string it is asked
    for; yields its `chat/completions` URL."""
    with _service(_ModelHandler, text) as url:
        yield f"{url}/v1/chat/completions"


@contextmanager
def api_agent(text: str) -> Iterator[str]:
    """An invented API agent streaming `text` as every answer; yields the base
    URL an agent's `base_url` names."""
    with _service(_AgentHandler, text) as url:
        yield f"{url}/v1"


def upload_attachment(http: httpx.Client, name: str, body: bytes) -> dict:
    """A file as a device uploads it before its turn: `POST /v1/attachments`
    with its name (percent-encoded UTF-8) and its SHA-256; answers its
    `attachment`, unbound until a `chat.send` names it."""
    headers = {
        "content-type": "application/octet-stream",
        "x-orbit-attachment-name": quote(name, safe=""),
        "x-orbit-sha256": hashlib.sha256(body).hexdigest(),
    }
    return _answered(http.post("/v1/attachments", content=body, headers=headers))["attachment"]


def invented_pdf(text: str) -> bytes:
    """A one-page PDF written by hand whose only text is `text` (ASCII, without
    parentheses or backslashes), in Helvetica, with its cross-reference table."""
    stream = f"BT /F1 18 Tf 72 720 Td ({text}) Tj ET".encode("ascii")
    objects = (
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]"
            b" /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream),
    )
    document = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(document))
        document += b"%d 0 obj\n%s\nendobj\n" % (number, body)
    table = len(document)
    document += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    document += b"".join(b"%010d 00000 n \n" % offset for offset in offsets)
    document += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        table,
    )
    return bytes(document)


def chat_turn(
    url: str, token: str, text: str, agent_id: str = AGENT_ID, attachments: tuple[str, ...] = ()
) -> dict:
    """A turn as a device holds it over `GET /v1/ws`: a `chat.send` of `text`
    to `agent_id`, with the ids of the files uploaded for it when there are
    any, then that turn's events until `chat.completed`; answers its data.
    Fails on an `error` of the turn, or past `TURN_SECONDS`."""
    turn = str(uuid.uuid4())
    params = {"text": text, "agent_id": agent_id}
    if attachments:
        params["attachments"] = list(attachments)
    frame = {"v": 1, "id": turn, "method": "chat.send", "params": params}
    address = "ws" + url.removeprefix("http") + "/v1/ws"
    deadline = time.monotonic() + TURN_SECONDS
    headers = {"Authorization": f"Bearer {token}"}
    with connect(address, additional_headers=headers, open_timeout=TURN_SECONDS) as ws:
        ws.send(json.dumps(frame))
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise HubFailed(f"the turn {turn} did not complete within {TURN_SECONDS} s")
            try:
                event = json.loads(ws.recv(timeout=left))
            except TimeoutError:
                continue
            if event.get("id") != turn:
                continue
            if event.get("event") == "error":
                raise HubFailed(f"the turn {turn} failed: {event.get('data')}")
            if event.get("event") == "chat.completed":
                return event["data"]


def tone_wav() -> bytes:
    """`MEETING_SECONDS` of a tone, 16-bit mono WAV."""
    frames = MEETING_SECONDS * MEETING_SAMPLE_RATE
    samples = (
        round(TONE_AMPLITUDE * math.sin(2 * math.pi * TONE_HERTZ * index / MEETING_SAMPLE_RATE))
        for index in range(frames)
    )
    output = io.BytesIO()
    with wave.open(output, "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(MEETING_SAMPLE_RATE)
        audio.writeframes(struct.pack(f"<{frames}h", *samples))
    return output.getvalue()


def _answered(response: httpx.Response) -> dict:
    if response.status_code != 200:
        raise HubFailed(
            f"{response.request.method} {response.request.url.path} answered "
            f"{response.status_code}: {response.text}"
        )
    return response.json()


def record_meeting(http: httpx.Client, recording: str, started_at_ms: int) -> None:
    """A meeting as a device records it: one chunk of the local microphone with
    its capture headers, then its close."""
    audio = tone_wav()
    frames = MEETING_SECONDS * MEETING_SAMPLE_RATE
    timing = {
        "started-at-ms": started_at_ms,
        "monotonic-ns": MEETING_MONOTONIC_NS,
        "first-sample": 0,
        "frames": frames,
        "sample-rate": MEETING_SAMPLE_RATE,
        "channels": 1,
    }
    headers = {
        "content-type": "audio/wav",
        "x-orbit-recording": recording,
        "x-orbit-track": MEETING_TRACK,
        "x-orbit-sequence": "0",
        "x-orbit-sha256": hashlib.sha256(audio).hexdigest(),
        **{f"x-orbit-{name}": str(value) for name, value in timing.items()},
    }
    _answered(http.put(f"/v1/chunks/{recording}.{MEETING_TRACK}.0", content=audio, headers=headers))
    manifest = {
        "tracks": [
            {"track": MEETING_TRACK, "chunk_count": 1, "last_sequence": 0, "total_frames": frames}
        ],
        "gaps": 0,
        "started_at_ms": started_at_ms,
        "finalized_at_ms": started_at_ms + MEETING_SECONDS * 1000,
    }
    _answered(http.post(f"/v1/recordings/{recording}/finalize", json=manifest))


def wait_for_transcript(http: httpx.Client, recording: str) -> dict:
    """The recording's document once its transcript is ready and no final
    pass is under way; fails past `TRANSCRIPT_SECONDS`."""
    deadline = time.monotonic() + TRANSCRIPT_SECONDS
    while True:
        document = _answered(http.get(f"/v1/recordings/{recording}"))
        transcript = document["transcript"]
        final = transcript.get("final_pass") or {}
        if transcript["state"] == "ready" and final.get("state") not in FINAL_PASS_UNDER_WAY:
            return document
        if time.monotonic() >= deadline:
            raise HubFailed(
                f"recording {recording} was not transcribed within {TRANSCRIPT_SECONDS} s: "
                f"{transcript}"
            )
        time.sleep(TRANSCRIPT_POLL_SECONDS)


def wait_for_note(http: httpx.Client, recording: str) -> dict:
    """The recording's note document (`GET …/note`) once it is `ready`; fails
    past `NOTE_SECONDS`."""
    deadline = time.monotonic() + NOTE_SECONDS
    while True:
        document = _answered(http.get(f"/v1/recordings/{recording}/note"))
        if document["state"] == "ready":
            return document
        if time.monotonic() >= deadline:
            raise HubFailed(
                f"the note of recording {recording} was not written within {NOTE_SECONDS} s: "
                f"{document['state']} {document.get('error')}"
            )
        time.sleep(NOTE_POLL_SECONDS)


def wait_for_proposals(http: httpx.Client, recording: str) -> list[dict]:
    """The vault proposals the hub prepared on its own for the recording,
    once there is one and none is still under way; fails past `NOTE_SECONDS`."""
    deadline = time.monotonic() + NOTE_SECONDS
    while True:
        listing = http.get("/v1/vault/distillations", params={"meeting_id": recording})
        entries = _answered(listing)["entries"]
        if entries and not any(entry["status"] in PROPOSAL_UNDER_WAY for entry in entries):
            return entries
        if time.monotonic() >= deadline:
            states = [entry["status"] for entry in entries]
            raise HubFailed(
                f"the vault proposals of recording {recording} did not settle within "
                f"{NOTE_SECONDS} s: {states}"
            )
        time.sleep(NOTE_POLL_SECONDS)


@pytest.fixture(scope="module")
def kit_model():
    """The invented notes model of the sample's hub: its URL and the text it
    writes, another than the sample's note."""
    text = "The contract kit wrote this invented note"
    with notes_model(text) as url:
        yield url, text


@pytest.fixture(scope="module")
def kit_transcription():
    """The invented service the sample's hub transcribes the kit's own
    meeting with: its URL and the text it answers."""
    text = "The contract kit recorded this invented meeting"
    with transcription_service(text) as url:
        yield url, text


@pytest.fixture(scope="module")
def kit_agent():
    """The invented model of the sample's API agent: its base URL and the text
    it answers, another than the sample's conversation."""
    text = "The contract kit answered this invented turn"
    with api_agent(text) as url:
        yield url, text


@pytest.fixture(scope="module")
def sample_configuration(tmp_path_factory, kit_transcription, kit_model, kit_agent) -> Path:
    """The configuration of the hub that serves a copy of the sample: its
    folder holds the private protocols' keys the kit signs with, its meeting
    transcription is the kit's invented service, its notes model the kit's
    invented model and its API agent's model the kit's invented agent."""
    folder = tmp_path_factory.mktemp("sample-hub") / "hub"
    return prepare(
        folder, read_test_key(), GOLDEN, kit_transcription[0], kit_model[0], kit_agent[0]
    )


@pytest.fixture(scope="module")
def sample_signature(sample_configuration):
    """Signs, when called, a private request to the hub serving the sample
    (the device bridge's unless another protocol is named)."""
    return lambda path, body=b"", protocol=BRIDGE_PROTOCOL: signed_headers(
        sample_configuration, path, body, protocol
    )


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


@pytest.fixture(scope="module")
def kit_meeting(sample_hub) -> str:
    """A meeting the kit records on the sample's hub as its own device, once
    its service has transcribed it; answers its id."""
    http, _ = sample_hub
    record_meeting(http, KIT_MEETING_ID, KIT_MEETING_STARTED_AT_MS)
    wait_for_transcript(http, KIT_MEETING_ID)
    return KIT_MEETING_ID


@pytest.fixture(scope="module")
def kit_note(sample_hub, kit_meeting) -> dict:
    """The note document of the kit's own meeting, once the hub has written it."""
    http, _ = sample_hub
    return wait_for_note(http, kit_meeting)


@pytest.fixture(scope="module")
def kit_turn(sample_hub, kit_agent) -> dict:
    """A turn the kit holds with the sample's API agent as its own device;
    answers its `chat.completed` data."""
    http, _ = sample_hub
    token = http.headers["authorization"].removeprefix("Bearer ")
    return chat_turn(str(http.base_url).rstrip("/"), token, "Hello from the contract kit")
