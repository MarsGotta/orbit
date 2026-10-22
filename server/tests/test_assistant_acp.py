"""Claude Code and Codex as the hub's assistant, over ACP, against an invented agent.

The agent is `support_acp_agent.py`, a subprocess speaking the agent side of
the same SDK; no Claude Code, Codex or Node runs here. What runs for real is
the hub's ACP client, its processes, and, in the whole turn, the app, the
device WebSocket, the runtime and the real `orbit-mcp` translator, which the
agent starts the way the adapters do, against the loopback fake hub.
"""

import asyncio
import json
import os
import shlex
import stat
import sys
import threading
from contextlib import contextmanager
from pathlib import Path
from uuid import uuid4

import pytest
from conftest import device, said
from fastapi.testclient import TestClient
from support_client_e2e import DAEMON_E2E_TIMEOUT_SECONDS, tcp_server
from support_orbit_mcp import DEFAULT_DEVICE, FakeHub, write_mcp_config

from orbit_hub.app import create_app
from orbit_hub.assistant import acp
from orbit_hub.assistant.backend import (
    MessageChunk,
    MessageComplete,
    PermissionRequested,
    RequestExpired,
    TextBlock,
)
from orbit_hub.errors import HubError

AGENT = Path(__file__).with_name("support_acp_agent.py")
# Waits observe events; load delays them instead of failing them.
WAIT_SECONDS = DAEMON_E2E_TIMEOUT_SECONDS
CLAUDE_LOCK_DOWN = {"claudeCode": {"options": {"tools": [], "strictMcpConfig": True, "settingSources": []}}}  # fmt: skip
CODEX_LOCK_DOWN = {
    "features": {"shell_tool": False},
    "sandbox_mode": "read-only",
    "approval_policy": "never",
    "web_search": "disabled",
}


def raw_settings(tmp_path, *flags, agent="claude-code", agent_id="claude-prueba", **extra):
    return {
        "id": agent_id,
        "name": "Ayudante",
        "kind": "acp",
        "agent": agent,
        "command": [sys.executable, str(AGENT), "--record", str(tmp_path / "record.jsonl"), *flags],
        "env": ["ORBIT_ACP_TEST_ALLOWED"],
        "orbit_mcp_config": str(tmp_path / "mcp.toml"),
        **extra,
    }


def backend_for(settings, tmp_path, *flags, **extra):
    return acp.build_backend(acp.parse_settings(raw_settings(tmp_path, *flags, **extra)), settings)


def records(tmp_path, kind=None):
    path = tmp_path / "record.jsonl"
    lines = path.read_text(encoding="utf-8").splitlines() if path.exists() else []
    seen = [json.loads(line) for line in lines]
    return [entry for entry in seen if kind is None or entry["kind"] == kind]


class Events:
    """What a connection reported, and a wait for the next one that matters."""

    def __init__(self):
        self.items = []
        self.arrived = asyncio.Event()

    async def __call__(self, event):
        self.items.append(event)
        self.arrived.set()

    async def wait_for(self, predicate):
        async with asyncio.timeout(WAIT_SECONDS):
            while True:
                for event in self.items:
                    if predicate(event):
                        return event
                self.arrived.clear()
                await self.arrived.wait()


def completed(event):
    return isinstance(event, MessageComplete)


def mode(path):
    return stat.S_IMODE(path.stat().st_mode)


@contextmanager
def fake_hub_in_a_thread():
    """The loopback fake hub orbit-mcp reaches, served beside a sync TestClient."""
    hub, state, ready = FakeHub(), {}, threading.Event()
    loop = asyncio.new_event_loop()

    async def serve():
        async with tcp_server(hub.app) as url:
            state["url"], state["stop"] = url, asyncio.Event()
            ready.set()
            await state["stop"].wait()

    thread = threading.Thread(target=loop.run_until_complete, args=(serve(),), daemon=True)
    thread.start()
    try:
        assert ready.wait(WAIT_SECONDS)
        yield hub, state["url"]
    finally:
        loop.call_soon_threadsafe(state["stop"].set)
        thread.join(WAIT_SECONDS)
        loop.close()


def next_event(ws):
    while True:
        frame = ws.receive_json()
        if frame["event"] != "state.snapshot":
            return frame


def test_a_whole_turn_runs_through_an_acp_agent(environment, tmp_path, monkeypatch):
    settings, store, speech = environment
    monkeypatch.setenv("ORBIT_ACP_TEST_ALLOWED", "sí")
    monkeypatch.setenv("ORBIT_ACP_TEST_WITHHELD", "no")
    paired = device(store, name="Portátil de Prueba")
    headers = {"Authorization": "Bearer " + paired["token"]}
    with fake_hub_in_a_thread() as (hub, url):
        write_mcp_config(tmp_path, url)
        assistant = backend_for(settings, tmp_path)
        app = create_app(
            settings, store=store, assistant=assistant, speech=speech, now=lambda: 1001
        )
        with (
            TestClient(app, base_url="https://localhost") as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            assert next_event(ws)["event"] == "ready"
            turn = str(uuid4())
            ws.send_json(
                {
                    "v": 1,
                    "id": turn,
                    "method": "chat.send",
                    "params": {"text": "Crea la nota de la excursión"},
                }
            )
            opened = next_event(ws)
            assert opened["event"] == "chat.session" and opened["id"] == turn
            session = opened["data"]["session_id"]
            # The acceptance of the turn and the agent's first updates race; each
            # keeps its own order.
            from_agent, accepted = [], None
            while not from_agent or from_agent[-1]["event"] != "approval.required":
                frame = next_event(ws)
                assert frame["id"] == turn
                assert frame["event"] not in {"chat.completed", "error"}, frame
                if frame["event"] == "chat.session":
                    accepted = frame
                else:
                    from_agent.append({"event": frame["event"], "data": frame["data"]})
            assert from_agent == [
                {
                    "event": "chat.status",
                    "data": {"session_id": session, "kind": "tool", "text": "orbit_devices"},
                },
                {
                    "event": "chat.status",
                    "data": {
                        "session_id": session,
                        "kind": "plan",
                        "text": "Mirar los equipos · Crear",
                    },
                },
                {
                    "event": "chat.delta",
                    "data": {"session_id": session, "text": "Voy a crear la nota. "},
                },
                {
                    "event": "approval.required",
                    "data": {
                        "session_id": session,
                        "request_id": from_agent[-1]["data"]["request_id"],
                        "kind": "approval",
                        "state": "pending",
                        "expires_at": None,
                        "description": "Crear la nota «Plan de la excursión»",
                        "choices": ["once", "deny"],
                    },
                },
            ]
            request_id = from_agent[-1]["data"]["request_id"]
            if accepted is None:
                accepted = next_event(ws)
            assert accepted["data"] == {"session_id": session, "status": "streaming"}

            answer = str(uuid4())
            ws.send_json(
                {
                    "v": 1,
                    "id": answer,
                    "method": "approval.respond",
                    "params": {
                        "session_id": session,
                        "turn_id": turn,
                        "request_id": request_id,
                        "approved": True,
                    },
                }
            )
            resolved = next_event(ws)
            assert resolved["event"] == "interaction.resolved"
            assert resolved["data"]["applied"] is True
            assert next_event(ws)["data"] == {"session_id": session, "text": "Hecha."}
            done = next_event(ws)
            assert done["event"] == "chat.completed" and done["id"] == turn
            assert done["data"] == {
                "session_id": session,
                "text": "Voy a crear la nota. Hecha.",
                "status": "completed",
                "persisted": True,
            }

    assert said(store, paired["device_id"], session) == [
        {"role": "user", "text": "Crea la nota de la excursión"},
        {"role": "assistant", "text": "Voy a crear la nota. Hecha."},
    ]
    folder = settings.data_dir / "agents" / "claude-prueba"
    [initialize] = records(tmp_path, "initialize")
    assert initialize["protocol_version"] == 1
    assert initialize["client_capabilities"]["fs"] == {
        "readTextFile": False,
        "writeTextFile": False,
    }
    assert initialize["client_capabilities"]["terminal"] is False
    assert initialize["cwd"] == str(folder)
    assert initialize["environment"]["ORBIT_ACP_TEST_ALLOWED"] == "sí"
    assert initialize["environment"]["ORBIT_ACP_TEST_WITHHELD"] is None
    [created] = records(tmp_path, "session/new")
    assert created["cwd"] == str(folder)
    assert created["meta"] == CLAUDE_LOCK_DOWN
    assert created["mcp_servers"] == [
        {
            "name": "orbit",
            "command": sys.executable,
            "args": ["-m", "orbit_mcp.cli"],
            "env": [
                {"name": "ORBIT_MCP_CONFIG", "value": str(tmp_path / "mcp.toml")},
                {"name": "ORBIT_MCP_AGENT", "value": "claude-prueba"},
            ],
        }
    ]
    for method in ("fs/read_text_file", "terminal/create"):
        [refusal] = records(tmp_path, method)
        assert refusal["code"] == -32601
        assert method in refusal["message"]
        assert "no ofrece" in refusal["message"]
    [tool] = records(tmp_path, "orbit_devices")
    assert tool["is_error"] is False and DEFAULT_DEVICE in tool["text"]
    assert hub.device_list_requests == 1
    [permission] = records(tmp_path, "permission")
    assert permission["outcome"] == {"outcome": "selected", "optionId": "si-una-vez"}
    [prompt] = records(tmp_path, "session/prompt")
    assert prompt["text"].endswith("\nCrea la nota de la excursión")
    assert mode(folder) == 0o700
    assert mode(folder / "claude-config") == 0o700


@pytest.mark.parametrize("agent", ["claude-code", "codex"])
async def test_each_agent_is_locked_in_its_own_tools_and_folder(environment, tmp_path, agent):
    settings, _store, _speech = environment
    backend = backend_for(settings, tmp_path, agent=agent)
    connection = backend.connection("equipo-1", "Portátil de Prueba", Events())
    await connection.open()
    try:
        opened = await connection.create_session()
    finally:
        await connection.close()
    assert opened.session_id == opened.stored_id
    folder = settings.data_dir / "agents" / "claude-prueba"
    [initialize] = records(tmp_path, "initialize")
    [created] = records(tmp_path, "session/new")
    environment_seen = initialize["environment"]
    if agent == "claude-code":
        assert environment_seen["CLAUDE_CONFIG_DIR"] == str(folder / "claude-config")
        assert environment_seen["CODEX_HOME"] is None
        assert environment_seen["CODEX_CONFIG"] is None
        assert created["meta"] == CLAUDE_LOCK_DOWN
    else:
        assert environment_seen["CLAUDE_CONFIG_DIR"] is None
        assert environment_seen["CODEX_HOME"] == str(folder / "codex-home")
        assert json.loads(environment_seen["CODEX_CONFIG"]) == CODEX_LOCK_DOWN
        # codex-acp sends its mode's sandbox and approval policy with every
        # turn, over the config; only this mode keeps the sandbox read-only.
        assert environment_seen["INITIAL_AGENT_MODE"] == "read-only"
        assert created["meta"] == {}
        assert mode(folder / "codex-home") == 0o700
    assert initialize["client_capabilities"]["fs"] == {
        "readTextFile": False,
        "writeTextFile": False,
    }
    assert initialize["client_capabilities"]["terminal"] is False
    assert mode(folder) == 0o700


async def test_a_denied_permission_reaches_the_agent_as_its_reject_option(environment, tmp_path):
    settings, _store, _speech = environment
    async with tcp_server(FakeHub().app) as url:
        write_mcp_config(tmp_path, url)
        events = Events()
        connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", events)
        await connection.open()
        try:
            opened = await connection.create_session()
            await connection.prompt(opened.session_id, [TextBlock("Crea la nota")])
            asked = await events.wait_for(lambda e: isinstance(e, PermissionRequested))
            receipt = await connection.respond_permission(
                opened.session_id, asked.request.request_id, allow=False
            )
            done = await events.wait_for(completed)
        finally:
            await connection.close()
    assert receipt.applied is True
    assert done == MessageComplete(
        opened.session_id, "Voy a crear la nota. No la creo.", "completed"
    )
    [permission] = records(tmp_path, "permission")
    assert permission["outcome"] == {"outcome": "selected", "optionId": "no"}


async def test_a_cancelled_turn_ends_interrupted_and_retires_its_permission(environment, tmp_path):
    settings, _store, _speech = environment
    events = Events()
    connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", events)
    await connection.open()
    try:
        opened = await connection.create_session()
        session = opened.session_id
        await connection.prompt(session, [TextBlock("espera un momento")])
        asked = await events.wait_for(lambda e: isinstance(e, PermissionRequested))
        assert asked.request.description == "Borrar la nota «Compras del sábado»"
        assert await connection.turn_stopped(session) is False
        await connection.cancel(session)
        expired = await events.wait_for(lambda e: isinstance(e, RequestExpired))
        done = await events.wait_for(completed)
        late = await connection.respond_permission(session, asked.request.request_id, allow=True)
        stopped = await connection.turn_stopped(session)
    finally:
        await connection.close()
    assert expired == RequestExpired(session, asked.request.request_id)
    assert done == MessageComplete(session, "Pensando… ", "interrupted")
    assert late.applied is False
    assert stopped is True
    [permission] = records(tmp_path, "permission")
    assert permission["outcome"] == {"outcome": "cancelled"}
    assert [entry["session_id"] for entry in records(tmp_path, "session/cancel")] == [session]


def test_a_command_that_does_not_exist_is_refused_at_startup_with_its_path(environment, tmp_path):
    settings, _store, _speech = environment
    missing = tmp_path / "agents" / "node_modules" / ".bin" / "claude-agent-acp"
    with pytest.raises(ValueError) as refused:
        acp.build_backend(
            acp.parse_settings(raw_settings(tmp_path, command=[str(missing)])), settings
        )
    assert str(missing) in str(refused.value)
    assert "does not exist" in str(refused.value)

    not_executable = tmp_path / "codex-acp"
    not_executable.write_text("#!/bin/sh\n", encoding="utf-8")
    not_executable.chmod(0o600)
    with pytest.raises(ValueError) as refused:
        acp.build_backend(
            acp.parse_settings(raw_settings(tmp_path, command=[str(not_executable)])), settings
        )
    assert str(not_executable) in str(refused.value)
    assert "is not executable" in str(refused.value)


async def test_a_command_that_does_not_start_is_refused_when_the_session_opens(
    environment, tmp_path
):
    settings, _store, _speech = environment
    connection = backend_for(settings, tmp_path, "--exit-before-initialize").connection(
        "equipo-1", "Mac", Events()
    )
    await connection.open()
    with pytest.raises(HubError) as refused:
        await connection.create_session()
    assert refused.value.code == "agent_start_failed"
    assert sys.executable in refused.value.message
    assert "código 7" in refused.value.message
    assert connection.live_sessions == set()

    script = tmp_path / "agente"
    script.write_text(f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {AGENT}\n")
    script.chmod(0o700)
    connection = acp.build_backend(
        acp.parse_settings(raw_settings(tmp_path, command=[str(script)])), settings
    ).connection("equipo-1", "Mac", Events())
    await connection.open()
    script.unlink()
    with pytest.raises(HubError) as refused:
        await connection.create_session()
    await connection.close()
    assert refused.value.code == "agent_start_failed"
    assert str(script) in refused.value.message
    assert "no existe" in refused.value.message


@pytest.mark.parametrize(
    ("agent", "login"),
    [
        ("claude-code", "CLAUDE_CONFIG_DIR={home} {command} --cli auth login --console"),
        ("codex", "CODEX_HOME={home} {command} cli login --device-auth"),
    ],
)
async def test_an_agent_asking_to_sign_in_is_refused_with_the_command_to_run_on_the_server(
    environment, tmp_path, agent, login
):
    settings, _store, _speech = environment
    folder = settings.data_dir / "agents" / "claude-prueba"
    home = shlex.quote(str(folder / ("claude-config" if agent == "claude-code" else "codex-home")))
    raw = raw_settings(tmp_path, "--auth-required", agent=agent)
    expected = login.format(home=home, command=shlex.join(raw["command"]))
    connection = acp.build_backend(acp.parse_settings(raw), settings).connection(
        "equipo-1", "Mac", Events()
    )
    await connection.open()
    with pytest.raises(HubError) as refused:
        await connection.create_session()
    await connection.close()
    assert refused.value.code == "agent_auth_required"
    assert expected in refused.value.message
    assert "Orbit no inicia sesiones" in refused.value.message

    # Signed out between turns: the turn fails with the same instruction.
    raw = raw_settings(tmp_path, agent=agent)
    expected = login.format(home=home, command=shlex.join(raw["command"]))
    events = Events()
    connection = acp.build_backend(acp.parse_settings(raw), settings).connection(
        "equipo-1", "Mac", events
    )
    await connection.open()
    try:
        opened = await connection.create_session()
        await connection.prompt(opened.session_id, [TextBlock("inicia la sesión")])
        done = await events.wait_for(completed)
    finally:
        await connection.close()
    assert done.status == "error"
    assert expected in done.text


async def test_a_resumed_session_loads_without_replaying_its_old_text(environment, tmp_path):
    settings, _store, _speech = environment
    backend = backend_for(settings, tmp_path)
    first = backend.connection("equipo-1", "Mac", Events())
    await first.open()
    opened = await first.create_session()
    await first.close()

    events = Events()
    second = backend.connection("equipo-1", "Mac", events)
    await second.open()
    try:
        assert opened.session_id not in second.live_sessions
        resumed = await second.resume_session(opened.stored_id)
        assert resumed.session_id in second.live_sessions
        await second.prompt(resumed.session_id, [TextBlock("hola")])
        done = await events.wait_for(completed)
    finally:
        await second.close()
    assert (resumed.session_id, resumed.stored_id) == (opened.stored_id, opened.stored_id)
    assert (resumed.message_count, resumed.loading_history) == (None, False)
    assert [e for e in events.items if isinstance(e, MessageChunk)] == [
        MessageChunk(resumed.session_id, "Hola.")
    ]
    assert done == MessageComplete(resumed.session_id, "Hola.", "completed")
    [loaded] = records(tmp_path, "session/load")
    assert loaded["session_id"] == opened.stored_id
    assert loaded["cwd"] == str(settings.data_dir / "agents" / "claude-prueba")
    assert loaded["meta"] == CLAUDE_LOCK_DOWN


async def test_an_agent_without_load_refuses_to_resume_with_its_cause(environment, tmp_path):
    settings, _store, _speech = environment
    connection = backend_for(settings, tmp_path, "--no-load").connection(
        "equipo-1", "Mac", Events()
    )
    await connection.open()
    with pytest.raises(HubError) as refused:
        await connection.resume_session("sesion-guardada")
    await connection.close()
    assert refused.value.code == "session_resume_unsupported"
    assert "session/load" in refused.value.message
    assert records(tmp_path, "session/load") == []


async def test_the_process_ceiling_retires_an_idle_session_and_refuses_when_all_are_busy(
    environment, tmp_path
):
    settings, _store, _speech = environment
    ceiling = acp.MAX_AGENT_PROCESSES
    events = Events()
    connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", events)
    await connection.open()
    try:
        sessions = [(await connection.create_session()).session_id for _ in range(ceiling)]
        newest = (await connection.create_session()).session_id
        # The session used longest ago gave its process to the new one.
        assert sessions[0] not in connection.live_sessions
        assert connection.live_sessions == {*sessions[1:], newest}
        for session in connection.live_sessions:
            await connection.prompt(session, [TextBlock("espera")])
        for session in connection.live_sessions:
            await events.wait_for(
                lambda e, session=session: (
                    isinstance(e, PermissionRequested) and e.session_id == session
                )
            )
        with pytest.raises(HubError) as refused:
            await connection.create_session()
    finally:
        await connection.close()
    assert refused.value.code == "agent_process_limit"
    assert refused.value.numbers == {"busy": ceiling, "limit": ceiling}
    assert f"techo {ceiling}" in refused.value.message


async def test_an_agent_that_dies_in_a_turn_fails_it_with_its_exit_code(environment, tmp_path):
    settings, _store, _speech = environment
    events = Events()
    connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", events)
    await connection.open()
    try:
        opened = await connection.create_session()
        await connection.prompt(opened.session_id, [TextBlock("muere")])
        done = await events.wait_for(completed)
        stopped = await connection.turn_stopped(opened.session_id)
    finally:
        await connection.close()
    assert done.status == "error"
    assert "código 3" in done.text
    assert opened.session_id not in connection.live_sessions
    assert stopped is True


async def test_a_message_over_the_ceiling_fails_the_turn_with_its_numbers(environment, tmp_path):
    settings, _store, _speech = environment
    events = Events()
    connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", events)
    await connection.open()
    try:
        opened = await connection.create_session()
        await connection.prompt(opened.session_id, [TextBlock("enorme")])
        done = await events.wait_for(completed)
    finally:
        await connection.close()
    assert done.status == "error"
    assert f"{acp.MAX_AGENT_MESSAGE_BYTES} bytes" in done.text
    assert opened.session_id not in connection.live_sessions


async def test_a_clarification_is_refused_with_its_cause(environment, tmp_path):
    settings, _store, _speech = environment
    connection = backend_for(settings, tmp_path).connection("equipo-1", "Mac", Events())
    await connection.open()
    with pytest.raises(HubError) as refused:
        await connection.respond_clarification("sesion", "pregunta", "sí", None)
    await connection.close()
    assert refused.value.code == "clarification_unsupported"


@pytest.mark.parametrize(
    ("change", "fragments"),
    [
        ({"agent": "gemini"}, ["`agent`", '"claude-code" or "codex"', "gemini"]),
        ({"agent": None}, ["`agent`", "missing"]),
        ({"command": []}, ["`command`", "at least one"]),
        ({"command": "claude-agent-acp"}, ["`command`", "list"]),
        ({"command": ["node_modules/.bin/claude-agent-acp"]}, ["`command`", "absolute"]),
        ({"command": ["/usr/bin/env", ""]}, ["`command`", "empty"]),
        ({"env": "ANTHROPIC_API_KEY"}, ["`env`", "list"]),
        ({"env": ["1BAD"]}, ["`env`", "1BAD", "not a variable name"]),
        ({"env": ["CLAUDE_CONFIG_DIR"]}, ["`env`", "CLAUDE_CONFIG_DIR", "Orbit sets"]),
        ({"env": ["OPENAI_API_KEY", "OPENAI_API_KEY"]}, ["`env`", "OPENAI_API_KEY", "twice"]),
        ({"orbit_mcp_config": None}, ["`orbit_mcp_config`", "missing"]),
        ({"orbit_mcp_config": "mcp.toml"}, ["`orbit_mcp_config`", "absolute"]),
        ({"id": "Claude Principal"}, ["`id`", "a-z"]),
        ({"profile": "deimos"}, ["unknown", "profile"]),
    ],
)
def test_settings_refusals_name_the_key_and_the_cause(tmp_path, change, fragments):
    raw = raw_settings(tmp_path)
    for key, value in change.items():
        if value is None:
            raw.pop(key)
        else:
            raw[key] = value
    with pytest.raises(ValueError) as refused:
        acp.parse_settings(raw)
    for fragment in fragments:
        assert fragment in str(refused.value)
    assert "claude-prueba" in str(refused.value) or "id" in change


def test_settings_keep_what_the_agent_needs(tmp_path):
    parsed = acp.parse_settings(raw_settings(tmp_path, agent="codex"))
    assert parsed == acp.AcpAgentSettings(
        agent_id="claude-prueba",
        agent="codex",
        command=(sys.executable, str(AGENT), "--record", str(tmp_path / "record.jsonl")),
        orbit_mcp_config=tmp_path / "mcp.toml",
        env=("ORBIT_ACP_TEST_ALLOWED",),
    )
    assert acp.KIND == "acp"
    assert os.path.isabs(parsed.command[0])
