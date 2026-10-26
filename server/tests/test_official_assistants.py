"""The public hub contract against the official SDK and a fake app-server."""

import asyncio
import base64
import builtins
import io
import json
import logging
import os
import sys
import threading
import time
from contextlib import ExitStack, contextmanager
from dataclasses import replace
from pathlib import Path
from uuid import uuid4

import pytest
from conftest import device, said
from fastapi.testclient import TestClient
from support_client_e2e import DAEMON_E2E_TIMEOUT_SECONDS, tcp_server
from support_official_tools import SECRET
from support_orbit_mcp import FakeHub, write_mcp_config
from test_attachments import PNG, uploaded

from orbit_hub.app import create_app

WAIT_SECONDS = DAEMON_E2E_TIMEOUT_SECONDS
CLI = Path(__file__).with_name("support_official_tools.py")


@contextmanager
def mcp_hub():
    hub, state, ready = FakeHub(), {}, threading.Event()
    loop = asyncio.new_event_loop()

    async def serve():
        async with tcp_server(hub.app) as url:
            state.update(url=url, stop=asyncio.Event())
            ready.set()
            await state["stop"].wait()

    thread = threading.Thread(target=loop.run_until_complete, args=(serve(),), daemon=True)
    thread.start()
    try:
        assert ready.wait(WAIT_SECONDS)
        yield state["url"]
    finally:
        loop.call_soon_threadsafe(state["stop"].set)
        thread.join(WAIT_SECONDS)
        loop.close()


def tool(tmp_path):
    path = tmp_path / "official-tool"
    path.write_text(
        f'#!{sys.executable}\nimport runpy\nrunpy.run_path({str(CLI)!r},run_name="__main__")\n'
    )
    path.chmod(0o700)
    return path


def send(ws, method, **params):
    request = str(uuid4())
    ws.send_json({"v": 1, "id": request, "method": method, "params": params})
    return request


def until(ws, event, frames):
    while True:
        frame = ws.receive_json()
        frames.append(frame)
        assert frame["event"] != "error", frame
        if frame["event"] == event:
            return frame["data"]


@pytest.fixture(params=["claude-code", "codex"])
def official(request, environment, tmp_path, monkeypatch):
    settings, store, _ = environment
    binary = tool(tmp_path)
    monkeypatch.setenv("ORBIT_TOOL_RECORD", str(tmp_path / "record.jsonl"))
    monkeypatch.setenv("ORBIT_TOOL_WITHHELD", SECRET)
    settings = replace(
        settings,
        agents=(
            {
                "id": "helper",
                "name": "Ares",
                "kind": request.param,
                "command": [str(binary)],
                "env": ["ORBIT_TOOL_RECORD"],
                "orbit_mcp_config": str(tmp_path / "mcp.toml"),
            },
        ),
    )
    home = (
        settings.data_dir
        / "agents"
        / "helper"
        / ("claude-config" if request.param == "claude-code" else "codex-home")
    )
    home.mkdir(parents=True, mode=0o700)
    credential = home / (".credentials.json" if request.param == "claude-code" else "auth.json")
    credential.write_text(json.dumps({"fixture_credential": SECRET}))
    credential.chmod(0o600)
    # This guard runs in the hub process. Only the fake vendor subprocess may
    # read the invented credential; neither the hub nor the SDK may open it.
    for module in (builtins, io):
        original = module.open

        def guarded(path, mode="r", *args, _open=original, **kwargs):
            if str(path) == str(credential) and "r" in mode:
                raise AssertionError("The hub tried to read an account credential")
            return _open(path, mode, *args, **kwargs)

        monkeypatch.setattr(module, "open", guarded)
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}

    def client():
        return TestClient(create_app(settings, store=store), base_url="https://localhost")

    return request.param, settings, store, paired, headers, client


@pytest.mark.parametrize("prompt", ["COMMAND", "EDIT"])
def test_turn_streams_tools_approval_clarification_and_resumes(official, tmp_path, caplog, prompt):
    vendor, settings, store, paired, headers, client = official
    caplog.set_level(logging.DEBUG)
    frames = []
    with mcp_hub() as url:
        write_mcp_config(tmp_path, url)
        with (
            client() as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            until(ws, "ready", frames)
            response = http.put(
                "/v1/agents/helper/personality", headers=headers, json={"text": "Eres Ares, breve."}
            )
            assert response.status_code == 200, response.text
            skill = "---\nname: resumen\ndescription: Resume una reunión.\n---\nConsulta orbit_meetings.\n"
            response = http.put(
                "/v1/agents/helper/skills/resumen", headers=headers, json={"content": skill}
            )
            assert response.status_code == 200, response.text
            image = uploaded(http, paired, "mapa.png", PNG)
            turn = send(ws, "chat.send", text=prompt, attachments=[image["id"]])
            session = until(ws, "chat.session", frames)["session_id"]
            permission = until(ws, "approval.required", frames)
            send(
                ws,
                "approval.respond",
                session_id=session,
                turn_id=turn,
                request_id=permission["request_id"],
                approved=True,
                choice="once",
            )
            question = until(ws, "clarify.required", frames)
            send(
                ws,
                "clarify.respond",
                session_id=session,
                request_id=question["request_id"],
                answer="Jardín",
                question_id=question["questions"][0]["qid"],
            )
            done = until(ws, "chat.completed", frames)
            assert done["text"] == "Hola. Hecho."
            assert done["status"] == "completed" and done["persisted"] is True
            page = http.get(f"/v1/sessions/{session}/messages", headers=headers).json()
            assert page["messages"][-1]["work"]["tools"] == 2
        # A new hub process/connection must load by the vendor's durable id.
        with (
            client() as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            until(ws, "ready", frames)
            waiting = send(ws, "chat.send", text="WAIT", session_id=session)
            until(ws, "chat.delta", frames)
            send(ws, "chat.interrupt", session_id=session, turn_id=waiting)
            interrupted = until(ws, "chat.completed", frames)
            assert interrupted["status"] == "interrupted"
    assert any(f["event"] == "chat.delta" and f["data"]["text"] == "Hola. " for f in frames)
    items = [f["data"] for f in frames if f["event"] == "chat.item"]
    assert any(
        i["label"] == "Mirando los Mac de Orbit" and i["phase"] == "completed" for i in items
    )
    assert said(store, paired["device_id"], session)[1]["text"] == "Hola. Hecho."
    records = [json.loads(line) for line in (tmp_path / "record.jsonl").read_text().splitlines()]
    assert any("mcp_result" in row for row in records)
    starts = [r for r in records if r.get("vendor")]
    assert all(not r["withheld"] for r in starts)
    expected = (
        settings.data_dir
        / "agents"
        / "helper"
        / ("claude-config" if vendor == "claude-code" else "codex-home")
    )
    assert all(r["home"] == str(expected) for r in starts)
    if vendor == "claude-code":
        assert any(a.startswith("--resume=") for a in starts[-1]["args"])
        assert (
            next(r for r in records if "claude_response" in r)["claude_response"]["behavior"]
            == "allow"
        )
        told = starts[0]["args"][starts[0]["args"].index("--append-system-prompt") + 1]
        content = next(r["user_message"] for r in records if "user_message" in r)["message"][
            "content"
        ]
        image = next(b for b in content if b["type"] == "image")
        assert image["source"] == {
            "type": "base64",
            "media_type": "image/png",
            "data": base64.b64encode(PNG).decode(),
        }
        assert "--setting-sources=" in starts[0]["args"]
        assert "--strict-mcp-config" in starts[0]["args"]
        assert starts[0]["args"][starts[0]["args"].index("--tools") + 1] == "default"
    else:
        assert any(r.get("method") == "thread/resume" for r in records)
        assert any(
            r.get("codex_response", {}).get("result", {}).get("decision") == "accept"
            for r in records
        )
        started = next(r["params"] for r in records if r.get("method") == "thread/start")
        told = started["developerInstructions"]
        assert (
            started["sandbox"] == "danger-full-access" and started["approvalPolicy"] == "untrusted"
        )
        assert started["config"]["features.shell_tool"] is True
        assert set(started["config"]["mcp_servers"]) == {"orbit"}
        content = next(r["params"]["input"] for r in records if r.get("method") == "turn/start")
        image = next(b for b in content if b["type"] == "image")
        assert image["url"] == "data:image/png;base64," + base64.b64encode(PNG).decode()
    assert "Eres Ares, breve." in told and "resumen: Resume una reunión." in told
    assert "Consulta orbit_meetings." not in told
    assert SECRET not in json.dumps(frames) + caplog.text
    for database in settings.data_dir.glob("hub.sqlite3*"):
        assert SECRET.encode() not in database.read_bytes()


def test_plan_limit_preserves_question_and_reports_reset(official, tmp_path, caplog):
    _vendor, settings, store, paired, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="LIMIT")
        session = until(ws, "chat.session", frames)["session_id"]
        while (frame := ws.receive_json())["event"] != "error":
            frames.append(frame)
        frames.append(frame)
        assert frame["data"]["code"] == "agent_limit_reached"
        assert frame["data"]["refusal"]["resets_at_ms"] == 1900000000000
        listing = http.get("/v1/agents", headers=headers).json()
        assert listing["agents"][0]["limit"]["until_ms"] == 1900000000000
        assert said(store, paired["device_id"], session) == [{"role": "user", "text": "LIMIT"}]
    assert SECRET not in json.dumps(frames) + caplog.text
    for database in settings.data_dir.glob("hub.sqlite3*"):
        assert SECRET.encode() not in database.read_bytes()


@pytest.mark.parametrize("vendor", ["claude-code", "codex"])
@pytest.mark.parametrize("executable", [False, True])
def test_missing_tool_names_vendor_and_path(environment, tmp_path, vendor, executable):
    settings, store, _adapter = environment
    path = tmp_path / ("missing-" + vendor)
    if executable:
        path.write_text("not executable")
    settings = replace(
        settings,
        assistant_tools={
            vendor: {"command": [str(path)], "orbit_mcp_config": str(tmp_path / "mcp.toml")}
        },
    )
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with TestClient(create_app(settings, store=store), base_url="https://localhost") as http:
        response = http.post(
            "/v1/agents",
            headers=headers,
            json={
                "service": vendor,
                "name": "Ares",
                "description": "",
                "color": "c2",
                "access": {
                    "screen": False,
                    "control": False,
                    "calendar": False,
                    "meetings": False,
                    "files": False,
                },
            },
        )
    assert response.status_code == 409, response.text
    assert response.json()["code"] == "agent_adapter_missing"
    assert str(path) in response.json()["message"]
    assert vendor in response.json()["message"]


@pytest.mark.parametrize(
    "prompt,code",
    [
        ("AUTH", "agent_auth_required"),
        ("CRASH", "agent_stopped"),
        ("MALFORMED", "agent_protocol"),
        ("OVERSIZE", "agent_stopped"),
        ("CAPACITY", "agent_rejected"),
    ],
)
def test_failed_turn_never_exports_vendor_diagnostics(official, caplog, prompt, code):
    _vendor, settings, store, paired, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text=prompt)
        session = until(ws, "chat.session", frames)["session_id"]
        while (frame := ws.receive_json())["event"] != "error":
            frames.append(frame)
        frames.append(frame)
        assert frame["data"]["code"] == code, frame
        if prompt == "CRASH":
            assert frame["data"]["refusal"]["exit_status"] == 7
        if prompt == "OVERSIZE":
            assert frame["data"]["refusal"]["max_bytes"] == 8 * 1024 * 1024
            if prompt == "CAPACITY":
                assert frame["data"]["refusal"]["stop_reason"] == (
                    "model_capacity" if _vendor == "claude-code" else "serverOverloaded"
                )
        assert said(store, paired["device_id"], session) == [{"role": "user", "text": prompt}]
    assert SECRET not in caplog.text + json.dumps(frames)
    for database in settings.data_dir.glob("hub.sqlite3*"):
        assert SECRET.encode() not in database.read_bytes()


def test_interrupt_expires_pending_approval_and_stops_its_process(official, tmp_path):
    _vendor, _settings, _store, _paired, headers, client = official
    frames = []
    with mcp_hub() as url:
        write_mcp_config(tmp_path, url)
        with (
            client() as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            until(ws, "ready", frames)
            turn = send(ws, "chat.send", text="COMMAND")
            session = until(ws, "chat.session", frames)["session_id"]
            until(ws, "approval.required", frames)
            send(ws, "chat.interrupt", session_id=session, turn_id=turn)
            done = until(ws, "chat.completed", frames)
            assert done["status"] == "interrupted"
    assert any(f["event"] == "interaction.expired" for f in frames)
    for row in map(json.loads, (tmp_path / "record.jsonl").read_text().splitlines()):
        if "pid" in row:
            with pytest.raises(ProcessLookupError):
                os.kill(row["pid"], 0)


def test_disconnect_during_a_live_turn_reaps_its_cli(official, tmp_path):
    _vendor, _settings, _store, _paired, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="WAIT")
        until(ws, "chat.delta", frames)
    for row in map(json.loads, (tmp_path / "record.jsonl").read_text().splitlines()):
        if "pid" in row:
            with pytest.raises(ProcessLookupError):
                os.kill(row["pid"], 0)


def test_process_ceiling_and_idle_eviction_are_public_refusals(official):
    _vendor, _settings, _store, _paired, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        sessions = []
        for _ in range(4):
            turn = send(ws, "chat.send", text="WAIT")
            session = until(ws, "chat.session", frames)["session_id"]
            until(ws, "chat.delta", frames)
            sessions.append((session, turn))
        send(ws, "chat.send", text="WAIT")
        while (frame := ws.receive_json())["event"] != "error":
            frames.append(frame)
        assert frame["data"]["code"] == "agent_process_limit", frame
        assert frame["data"]["refusal"]["busy"] == 4 and frame["data"]["refusal"]["limit"] == 4
        session, turn = sessions[0]
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        assert until(ws, "chat.completed", frames)["status"] == "interrupted"
        send(ws, "chat.send", text="WAIT")
        until(ws, "chat.delta", frames)


def test_operator_command_prefix_is_preserved(official):
    _vendor, settings, store, _paired, headers, _client = official
    entry = {**settings.agents[0], "command": [sys.executable, str(CLI)]}
    settings = replace(settings, agents=(entry,))
    frames = []
    with (
        TestClient(create_app(settings, store=store), base_url="https://localhost") as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="WAIT")
        session = until(ws, "chat.session", frames)["session_id"]
        until(ws, "chat.delta", frames)
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        assert until(ws, "chat.completed", frames)["status"] == "interrupted"


@pytest.mark.parametrize("official", ["codex"], indirect=True)
def test_rpc_rejection_preserves_code_without_raw_reason(official, caplog):
    _vendor, _settings, _store, _paired, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="RPCERROR")
        while (frame := ws.receive_json())["event"] != "error":
            frames.append(frame)
        frames.append(frame)
        assert frame["data"]["code"] == "agent_rejected"
        assert frame["data"]["refusal"]["rpc_code"] == -32602
    assert SECRET not in caplog.text + json.dumps(frames)


@pytest.mark.parametrize("official", ["claude-code"], indirect=True)
def test_stop_during_sdk_initialization_never_submits_a_prompt(official, tmp_path):
    _vendor, _settings, _store, _paired, headers, client = official
    marker = tmp_path / "hold-start"
    marker.touch()
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="WAIT")
        session = until(ws, "chat.session", frames)["session_id"]
        deadline = time.monotonic() + WAIT_SECONDS
        while not (tmp_path / "record.jsonl").exists():
            assert time.monotonic() < deadline
            time.sleep(0.01)
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        assert until(ws, "chat.completed", frames)["status"] == "interrupted"
    records = list(map(json.loads, (tmp_path / "record.jsonl").read_text().splitlines()))
    assert not any("user_message" in r for r in records)


@pytest.mark.parametrize("official", ["codex"], indirect=True)
def test_initializing_runners_are_busy_until_their_sessions_open(official, tmp_path):
    _vendor, _settings, store, _paired, _headers, client = official
    marker = tmp_path / "hold-start"
    marker.touch()
    frames = []

    def starts():
        path = tmp_path / "record.jsonl"
        return (
            [r for r in map(json.loads, path.read_text().splitlines()) if r.get("vendor")]
            if path.exists()
            else []
        )

    decision = []
    arrived = threading.Event()

    def receive_decision(ws):
        while True:
            frame = ws.receive_json()
            if frame["event"] in {"error", "chat.session"}:
                decision.append(frame)
                arrived.set()
                return

    with client() as http, ExitStack() as stack:
        sockets = []
        for _ in range(5):
            paired = device(store)
            ws = stack.enter_context(
                http.websocket_connect(
                    "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
                )
            )
            until(ws, "ready", frames)
            sockets.append(ws)
        try:
            for ws in sockets[:4]:
                send(ws, "chat.send", text="WAIT")
            deadline = time.monotonic() + WAIT_SECONDS
            while len(starts()) < 4:
                assert time.monotonic() < deadline
                time.sleep(0.01)
            send(sockets[-1], "chat.send", text="WAIT")
            receiver = threading.Thread(target=receive_decision, args=(sockets[-1],), daemon=True)
            receiver.start()
            # The old bug starts a fifth process by evicting an initializer.
            # Release the fixture on that observed fact or on the refusal;
            # the assertion is independent of how fast either happens.
            while not arrived.is_set() and len(starts()) == 4:
                assert time.monotonic() < deadline
                time.sleep(0.01)
        finally:
            marker.unlink(missing_ok=True)
        assert arrived.wait(WAIT_SECONDS)
        receiver.join(WAIT_SECONDS)
        assert len(starts()) == 4
        assert decision[0]["event"] == "error", decision
        assert decision[0]["data"]["code"] == "agent_process_limit"
