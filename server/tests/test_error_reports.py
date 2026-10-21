"""One global handler, the `op` of each request, and Sentry with nothing but codes.

The privacy test runs the real Sentry SDK against a fake Sentry on 127.0.0.1:
nothing in this file ever reaches sentry.io.
"""

import gzip
import json
import logging
import re
import socket
import threading
import time
import zlib
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

import pytest
import sentry_sdk
from conftest import device
from fastapi.testclient import TestClient
from support_app import crash_the_revocation_pass
from support_client_e2e import tcp_server
from support_orbit_mcp import (
    FOREGROUND_WINDOW,
    SCREENSHOT_ANSWER,
    FakeHub,
    answered,
    write_mcp_config,
)

from orbit_hub import error_reports
from orbit_hub.app import create_app
from orbit_hub.log import JsonFormatter, report
from orbit_hub.notes.repository import NoteRepository
from orbit_hub.runtime import DeviceRuntime
from orbit_mcp.config import Config
from orbit_mcp.hub import HubClient
from orbit_mcp.server import build_server

MARKER = "MARCADOR-PRIVADO-7c41"
OP = "op-test-0001abcd"
OP_SHAPE = re.compile(r"^[A-Za-z0-9-]{8,64}$")
RECORDING = "6f1d2c1e-5b8a-4c11-9d0e-2a3b4c5d6e7f"


class LogLines:
    """The hub's JSON log lines, formatted when they are written, as in production."""

    def __init__(self):
        self.lines: list[dict] = []
        formatter = JsonFormatter()
        lines = self.lines

        class Collect(logging.Handler):
            def emit(self, record):
                lines.append(json.loads(formatter.format(record)))

        self.handler = Collect()

    def __enter__(self):
        logging.getLogger("orbit_hub").addHandler(self.handler)
        return self

    def __exit__(self, *_exc):
        logging.getLogger("orbit_hub").removeHandler(self.handler)

    def with_event(self, event):
        return [line for line in self.lines if line.get("event") == event]


@pytest.fixture
def lines():
    with LogLines() as collected:
        yield collected


def _http(environment, settings=None):
    base, store, adapter = environment
    return TestClient(
        create_app(settings or base, store=store, adapter=adapter, now=lambda: 1001),
        base_url="https://localhost",
    )


def _explode_note_edit(monkeypatch):
    def exploding_edit(self, device_id, recording, payload, now):
        # A line written in the store thread, which must still carry the op.
        logging.getLogger("orbit_hub.notes").info("x", extra={"event": "store_thread_line"})
        private = MARKER  # a local variable holding content
        raise RuntimeError(f"fallo con contenido {private}")

    monkeypatch.setattr(NoteRepository, "edit", exploding_edit)


# Criterio 1 y 3 ──────────────────────────────────────────────────────────────


def test_unexpected_exception_in_a_route_answers_internal_error_with_the_request_op(
    environment, monkeypatch, lines
):
    _explode_note_edit(monkeypatch)
    _, store, _ = environment
    owner = device(store)
    with _http(environment) as http:
        response = http.put(
            f"/v1/recordings/{RECORDING}/note/edits",
            json={"base_revision": 0},
            headers={"Authorization": "Bearer " + owner["token"], "X-Orbit-Op": OP},
        )
    assert response.status_code == 500
    body = response.json()
    assert body["code"] == "internal_error"
    assert body["op"] == OP
    assert MARKER not in response.text
    [line] = lines.with_event("internal_error")
    assert line["op"] == OP
    assert line["code"] == "internal_error"
    assert line["route"] == "/v1/recordings/{recording_id}/note/edits"
    assert line["status"] == 500
    assert [line["op"] for line in lines.with_event("store_thread_line")] == [OP]
    assert MARKER not in json.dumps(lines.lines)


def test_every_http_error_body_carries_the_op_and_an_invalid_op_is_replaced(environment):
    with _http(environment) as http:
        refused = http.get("/v1/sessions", headers={"X-Orbit-Op": OP})
        invalid = http.get("/v1/sessions", headers={"X-Orbit-Op": "corta"})
        missing = http.get("/v1/sessions")
    assert refused.status_code == 401 and refused.json()["op"] == OP
    for response in (invalid, missing):
        assert response.status_code == 401
        generated = response.json()["op"]
        assert OP_SHAPE.match(generated) and generated != "corta"
    assert invalid.json()["op"] != missing.json()["op"]


def test_websocket_error_carries_the_frame_op_or_its_id(environment, monkeypatch, lines):
    async def exploding_handle(self, method, params, request_id):
        raise RuntimeError(f"fallo con {MARKER}")

    monkeypatch.setattr(DeviceRuntime, "handle", exploding_handle)
    _, store, _ = environment
    owner = device(store)
    with (
        _http(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        with_op, without_op, invalid_op = (str(uuid4()) for _ in range(3))
        ping = {"v": 1, "method": "ping", "params": {}}
        frames = (
            ping | {"id": with_op, "op": OP},
            ping | {"id": without_op},
            ping | {"id": invalid_op, "op": "no válida"},
        )
        answers = []
        for frame in frames:
            ws.send_json(frame)
            answers.append(ws.receive_json())
    assert [answer["data"]["code"] for answer in answers] == ["internal_error"] * 3
    assert answers[0]["data"]["op"] == OP
    assert answers[1]["data"]["op"] == without_op
    generated = answers[2]["data"]["op"]
    assert OP_SHAPE.match(generated) and generated not in {invalid_op, "no válida"}
    logged = [line["op"] for line in lines.with_event("internal_error")]
    assert logged == [OP, without_op, generated]


def test_op_is_not_part_of_an_operation_fingerprint(environment):
    """A retried frame keeps its receipt whatever op it carries now."""
    _, store, _ = environment
    owner = device(store)
    request = str(uuid4())
    sent = {"v": 1, "id": request, "method": "chat.send", "params": {"text": "hola"}}
    with (
        _http(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        ws.send_json(sent | {"op": OP})
        # The answer to the request itself, its receipt, says the turn's status.
        receipt = ws.receive_json()
        while not (receipt["event"] == "chat.session" and "status" in receipt["data"]):
            receipt = ws.receive_json()
        ws.send_json(sent | {"op": "op-test-retry-0002"})
        replay = ws.receive_json()
        while replay["event"] == "state.snapshot":
            replay = ws.receive_json()
    assert replay == receipt


async def test_the_translator_sends_one_op_per_tool_call(tmp_path):
    windows = answered({"windows": [FOREGROUND_WINDOW]})
    hub = FakeHub(device_replies=[windows, answered(SCREENSHOT_ANSWER), windows])
    async with tcp_server(hub.app) as url:
        config = Config.load(write_mcp_config(tmp_path, url))
        server = build_server(config, HubClient(config))
        await server.call_tool("orbit_look", {})
        first = list(hub.ops)
        await server.call_tool("orbit_windows", {})
        second = hub.ops[len(first) :]
    assert len(first) >= 2 and second
    assert len(set(first)) == 1 and len(set(second)) == 1
    assert OP_SHAPE.match(first[0]) and OP_SHAPE.match(second[0])
    assert first[0] != second[0]


# Criterio 2 ──────────────────────────────────────────────────────────────────


def test_a_background_task_failure_leaves_its_line_and_reaches_report(
    environment, monkeypatch, lines
):
    reported = []
    original = report

    def spy(*args, **kwargs):
        reported.append(kwargs.get("code", args[1] if len(args) > 1 else None))
        return original(*args, **kwargs)

    monkeypatch.setattr("orbit_hub.log.report", spy)
    crashed = threading.Event()

    def explosion():
        crashed.set()
        return RuntimeError(f"fallo de fondo {MARKER}")

    _, store, _ = environment
    paired = device(store)
    crash_the_revocation_pass(store, monkeypatch, explosion)
    with _http(environment) as http:
        # The pass reaches a device only through its runtime: open its socket.
        headers = {"Authorization": "Bearer " + paired["token"]}
        with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
            assert ws.receive_json()["event"] == "ready"
        assert crashed.wait(5)
        deadline = time.monotonic() + 5
        while not lines.with_event("task_crashed") and time.monotonic() < deadline:
            time.sleep(0.01)
    [line, *_] = lines.with_event("task_crashed")
    assert line["code"] == "task_crashed"
    assert line["task"] == "enforce_revocations"
    assert OP_SHAPE.match(line["op"])
    assert "task_crashed" in reported
    assert MARKER not in json.dumps(lines.lines)


# Criterio 4 y 5 ──────────────────────────────────────────────────────────────


class FakeSentry:
    """Receives what the SDK posts: every body, decompressed, byte for byte."""

    def __init__(self):
        self.bodies: list[bytes] = []
        bodies = self.bodies

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                encoding = self.headers.get("Content-Encoding", "")
                if encoding == "gzip":
                    raw = gzip.decompress(raw)
                elif encoding == "deflate":
                    raw = zlib.decompress(raw)
                elif encoding:
                    raise AssertionError(f"unexpected compression: {encoding}")
                bodies.append(raw)
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b"{}")

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.dsn = f"http://clavepublica@127.0.0.1:{self.server.server_address[1]}/1"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_exc):
        self.server.shutdown()
        self.server.server_close()

    def items(self) -> list[tuple[dict, dict]]:
        """(item header, payload) of every envelope item received."""
        found = []
        for body in self.bodies:
            rest = body.split(b"\n", 1)[1]
            while rest.strip():
                header_line, rest = rest.split(b"\n", 1)
                header = json.loads(header_line)
                if "length" in header:
                    payload, rest = rest[: header["length"]], rest[header["length"] + 1 :]
                else:
                    payload, _, rest = rest.partition(b"\n")
                found.append((header, json.loads(payload)))
        return found


def test_privacy_nothing_but_codes_reaches_sentry(environment, monkeypatch):
    _explode_note_edit(monkeypatch)

    async def exploding_handle(self, method, params, request_id):
        private = params.text  # the conversation itself, as a local variable
        # An earlier log line of the same request, the kind a logging
        # integration would keep as a breadcrumb.
        logging.getLogger("uvicorn.error").warning("registro previo %s", private)
        raise RuntimeError(f"fallo con {private}")

    monkeypatch.setattr(DeviceRuntime, "handle", exploding_handle)
    base, store, _ = environment
    # The socket below gives the device the runtime the revocation pass reaches.
    crashes = crash_the_revocation_pass(
        store, monkeypatch, lambda: RuntimeError(f"fallo de fondo {MARKER}")
    )
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    ws_op = "op-test-ws-0003"
    with FakeSentry() as sentry:
        settings = replace(base, error_reports={"dsn": sentry.dsn})
        with _http(environment, settings) as http:
            assert sentry_sdk.get_client().is_active()
            logging.getLogger("uvicorn.error").error("registro previo %s", MARKER)
            logging.getLogger("orbit_hub").warning("registro previo %s", MARKER)
            response = http.put(
                f"/v1/recordings/{RECORDING}/note/edits",
                content=json.dumps({"base_revision": 0, "title": MARKER}),
                headers=headers | {"X-Orbit-Op": OP, "Content-Type": "application/json"},
            )
            assert response.status_code == 500
            with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
                assert ws.receive_json()["event"] == "ready"
                ws.send_json(
                    {
                        "v": 1,
                        "id": str(uuid4()),
                        "op": ws_op,
                        "method": "chat.send",
                        "params": {"text": MARKER},
                    }
                )
                assert ws.receive_json()["data"]["code"] == "internal_error"
            deadline = time.monotonic() + 5
            while crashes[0] and time.monotonic() < deadline:
                time.sleep(0.01)
            assert crashes[0] == 0
        # Leaving the app flushed and closed the SDK.
        assert not sentry_sdk.get_client().is_active()

    received = b"".join(sentry.bodies)
    assert received, "the fake Sentry received nothing"
    assert MARKER.encode() not in received
    assert socket.gethostname().encode() not in received
    items = sentry.items()
    assert {header["type"] for header, _ in items} == {"event"}
    events = [payload for _, payload in items]
    tags = [event["tags"] for event in events]
    assert {"op": OP, "code": "internal_error", "route": "/v1/recordings/{recording_id}/note/edits", "source": "hub"} in tags  # fmt: skip
    assert {"op": ws_op, "code": "internal_error", "source": "hub"} in tags
    assert any(tag["code"] == "task_crashed" for tag in tags)
    allowed = {"event_id", "timestamp", "level", "platform", "release", "message"}
    allowed |= {"tags", "extra", "exception"}
    for event in events:
        assert set(event) <= allowed, set(event) - allowed
        assert event["message"] == event["tags"]["code"]
        assert set(event["tags"]) <= {"op", "code", "route", "source"}
        assert OP_SHAPE.match(event["tags"]["op"])
        assert all(type(value) in {int, float} for value in event.get("extra", {}).values())
        for exception in event.get("exception", {}).get("values", []):
            assert set(exception) <= {"type", "stacktrace"}
            for frame in exception.get("stacktrace", {}).get("frames", []):
                assert set(frame) <= {"filename", "function", "lineno"}


def test_scrub_keeps_only_the_allow_list():
    """The allow-list itself, against an event with content in every field the SDK can fill."""
    event = {
        "event_id": "e" * 32,
        "timestamp": 1.0,
        "level": "error",
        "platform": "python",
        "release": "orbit-hub@0.4.0",
        "server_name": MARKER,
        "environment": MARKER,
        "message": MARKER,
        "logentry": {"message": MARKER, "params": [MARKER]},
        "breadcrumbs": {"values": [{"message": MARKER, "data": {"x": MARKER}}]},
        "request": {"data": MARKER, "headers": {"authorization": MARKER}},
        "user": {"ip_address": MARKER},
        "contexts": {"runtime": {"name": MARKER}},
        "modules": {MARKER: "1"},
        "tags": {"op": OP, "code": "internal_error", "source": "hub", "device": MARKER},
        "extra": {"bytes": 12, "ratio": 0.5, "flag": True, "text": MARKER},
        "exception": {
            "values": [
                {
                    "type": "RuntimeError",
                    "value": MARKER,
                    "module": MARKER,
                    "mechanism": {"type": "generic", "data": {"x": MARKER}},
                    "stacktrace": {
                        "frames": [
                            {
                                "filename": "orbit_hub/app.py",
                                "abs_path": MARKER,
                                "function": "boundary",
                                "lineno": 7,
                                "context_line": MARKER,
                                "pre_context": [MARKER],
                                "vars": {"private": MARKER},
                            }
                        ]
                    },
                }
            ]
        },
    }
    clean = error_reports.scrub(event, {})
    assert MARKER not in json.dumps(clean)
    assert clean == {
        "event_id": "e" * 32,
        "timestamp": 1.0,
        "level": "error",
        "platform": "python",
        "release": "orbit-hub@0.4.0",
        "message": "internal_error",
        "tags": {"op": OP, "code": "internal_error", "source": "hub"},
        "extra": {"bytes": 12, "ratio": 0.5},
        "exception": {
            "values": [
                {
                    "type": "RuntimeError",
                    "stacktrace": {
                        "frames": [
                            {"filename": "orbit_hub/app.py", "function": "boundary", "lineno": 7}
                        ]
                    },
                }
            ]
        },
    }
    # An event that did not come through `report` carries no code: it never leaves.
    assert error_reports.scrub({"message": MARKER, "tags": {}}, {}) is None


def test_without_error_reports_the_sdk_is_never_started(environment, monkeypatch):
    _explode_note_edit(monkeypatch)

    def forbidden(*_args, **_kwargs):
        raise AssertionError("sentry_sdk.init sin [error_reports]")

    monkeypatch.setattr(error_reports.sentry_sdk, "init", forbidden)
    _, store, _ = environment
    owner = device(store)
    with _http(environment) as http:
        response = http.put(
            f"/v1/recordings/{RECORDING}/note/edits",
            json={"base_revision": 0},
            headers={"Authorization": "Bearer " + owner["token"]},
        )
        assert response.status_code == 500
        assert not sentry_sdk.get_client().is_active()


def test_error_reports_section_rejects_a_dsn_that_is_not_one(environment):
    base, _, _ = environment
    with pytest.raises(ValueError, match="error_reports"):
        replace(base, error_reports={"dsn": "no es un dsn"})
    with pytest.raises(ValueError, match="error_reports"):
        replace(base, error_reports={"dsn": "https://k@o1.ingest.sentry.io/1", "otro": 1})
