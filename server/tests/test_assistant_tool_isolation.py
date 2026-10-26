"""Policy observed through real hub sessions and the official-tool wire fakes."""

# Imported parametrized fixture is used by pytest injection.
# ruff: noqa: F811

import asyncio
import json
import tomllib
from pathlib import Path

import pytest
from test_official_assistants import official, send, until  # noqa: F401


def records(tmp_path):
    return list(map(json.loads, (tmp_path / "record.jsonl").read_text().splitlines()))


def interrupt_waiting(client, headers, session=None):
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="WAIT", **({"session_id": session} if session else {}))
        session = until(ws, "chat.session", frames)["session_id"] if session is None else session
        until(ws, "chat.delta", frames)
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        until(ws, "chat.completed", frames)
    return session


@pytest.mark.parametrize("official", ["codex"], indirect=True)
def test_codex_start_and_resume_decide_every_default_feature_and_ignore_cached_plugin(
    official, tmp_path
):
    _, settings, _, _, headers, client = official
    home = settings.data_dir / "agents/helper/codex-home"
    plugin = home / "plugins/cache/fixture/1/.codex-plugin/plugin.json"
    plugin.parent.mkdir(parents=True)
    plugin.write_text('{"name":"invented-mail-plugin"}')
    local = tmp_path / ".codex/config.toml"
    local.parent.mkdir()
    local.write_text("[features]\nplugins = true\nhooks = true\n")
    # A prior version may have trusted the enclosing repository.
    (home / "config.toml").write_text(
        f'[projects."{tmp_path}"]\ntrust_level = "trusted"\n'
        "[features]\nplugins = true\nhooks = true\nconnectors = true\ncodex_hooks = true\n"
    )
    session = interrupt_waiting(client, headers)
    interrupt_waiting(client, headers, session)
    rows = records(tmp_path)
    starts = [r["params"] for r in rows if r.get("method") in {"thread/start", "thread/resume"}]
    assert len(starts) == 2
    for params in starts:
        config = params["config"]
        # Independent snapshot captured with an empty HOME from the pinned CLI.
        pin = tomllib.loads((Path(__file__).parents[1] / "codex-release.toml").read_text())
        defaults = Path(__file__).with_name("fixtures") / f"codex-{pin['version']}-features.txt"
        for line in defaults.read_text().splitlines():
            name, enabled = line.split()[0], line.split()[-1]
            if enabled == "true":
                assert name in config["features"], f"Undecided upstream default: {name}"
        for name in (
            "plugins",
            "apps",
            "browser_use",
            "hooks",
            "guardian_approval",
            "multi_agent",
            "unbounded_connection_retries",
            "daemon_auto_start",
            "image_generation",
        ):
            assert config["features"][name] is False
        for name in ("shell_tool", "unified_exec", "view_image"):
            assert config["features"][name] is True
        assert config["web_search"] == "disabled"
        assert config["features"]["connectors"] is False
        assert config["features"]["codex_hooks"] is False
        assert set(config["mcp_servers"]) == {"orbit"}
        work = Path(params["cwd"])
        for folder in (work, *work.parents):
            assert config["projects"][str(folder)]["trust_level"] == "untrusted"
        assert params["approvalsReviewer"] == "user"
    assert not any("loaded_plugin" in r or "loaded_project_config" in r for r in rows)
    assert plugin.is_file()  # Ignored, without deleting the account's cache.


@pytest.mark.parametrize("official", ["claude-code"], indirect=True)
def test_claude_only_loads_orbit_mcp_and_orbit_permission_hook(official, tmp_path, monkeypatch):
    _, settings, _, _, headers, client = official
    settings.agents[0]["env"].extend(["ENABLE_CLAUDEAI_MCP_SERVERS", "CLAUDE_CODE_PLUGIN_DIRS"])
    monkeypatch.setenv("ENABLE_CLAUDEAI_MCP_SERVERS", "true")
    monkeypatch.setenv("CLAUDE_CODE_PLUGIN_DIRS", "/invented/plugin")
    interrupt_waiting(client, headers)
    rows = records(tmp_path)
    start = next(r for r in rows if r.get("vendor"))
    assert start["account_connectors"] == "false"
    assert not start["plugin_dirs"]
    args = start["args"]
    assert "--setting-sources=" in args and "--strict-mcp-config" in args
    assert "--plugin-dir" not in args and "--plugin-url" not in args
    assert "--bare" not in args  # Subscription OAuth must remain available.
    config = json.loads(args[args.index("--mcp-config") + 1])
    assert set(config["mcpServers"]) == {"orbit"}
    assert json.loads(args[args.index("--settings") + 1])["disableClaudeAiConnectors"] is True
    hooks = next(r["sdk_hooks"] for r in rows if "sdk_hooks" in r)
    assert set(hooks) == {"PreToolUse"}


def test_silent_provider_answers_after_more_than_30_seconds(official, tmp_path, monkeypatch):
    _, _, _, _, headers, client = official

    async def advance_clock():
        loop = asyncio.get_running_loop()
        real_time = loop.time
        started = real_time()
        # Keep real time progressing; the fake is gated so there is no race
        # between real pipe I/O and the deliberate clock jump.
        monkeypatch.setattr(loop, "time", lambda: real_time() + 61)
        for _ in range(4):
            # Drain overdue timers and their cancellation callbacks before
            # releasing the provider, rather than deciding order with sleeps.
            checkpoint = loop.create_future()
            loop.call_later(0, checkpoint.set_result, None)
            await checkpoint
        return loop.time() - started

    frames = []
    release = tmp_path / "release-silent-turn"
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="SILENT")
        # Initialization and turn submission have completed before the quiet
        # interval starts; their separate request timeouts are not under test.
        until(ws, "chat.delta", frames)
        try:
            assert http.portal.call(advance_clock) > 30
        finally:
            release.touch()
        done = until(ws, "chat.completed", frames)
        assert done["status"] == "completed" and done["persisted"] is True
        assert done["text"] == "Hola. Hecho."


@pytest.mark.parametrize("official", ["codex"], indirect=True)
def test_exhausted_provider_retries_preserve_structured_cause(official):
    _, _, _, _, headers, client = official
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="NETWORK")
        while (frame := ws.receive_json())["event"] != "error":
            frames.append(frame)
        refusal = frame["data"]["refusal"]
        assert refusal["stop_reason"] == "httpConnectionFailed"
        assert refusal["http_status_code"] == 503
