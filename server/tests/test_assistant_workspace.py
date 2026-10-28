"""Official assistants' host workspaces through the paired hub API and socket."""

import json

import pytest
from test_official_assistants import official, send, until  # noqa: F401

# Imported parametrized fixture is used by pytest injection.
# ruff: noqa: F811


def records(tmp_path):
    return [json.loads(line) for line in (tmp_path / "record.jsonl").read_text().splitlines()]


def test_workspace_is_stored_and_used(official, tmp_path):
    vendor, settings, _, _, headers, client = official
    project = tmp_path / "project"
    project.mkdir()
    with client() as http:
        config = http.get("/v1/agents/helper/config", headers=headers).json()
        default = config["working_folder"]
        assert default.startswith("/")
        assert not default.startswith(str(settings.data_dir) + "/")
        response = http.put(
            "/v1/agents/helper/working-folder", headers=headers, json={"path": str(project)}
        )
        assert response.status_code == 200, response.text
        assert response.json()["working_folder"] == str(project)
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        assert http.get("/v1/agents/helper/config", headers=headers).json()[
            "working_folder"
        ] == str(project)
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="WAIT")
        session = until(ws, "chat.session", frames)["session_id"]
        until(ws, "chat.delta", frames)
        send(ws, "chat.interrupt", session_id=session, turn_id=turn)
        until(ws, "chat.completed", frames)
    assert next(row for row in records(tmp_path) if "vendor" in row)["cwd"] == str(project)
    if vendor == "codex":
        started = next(
            row["params"] for row in records(tmp_path) if row.get("method") == "thread/start"
        )
        assert started["cwd"] == str(project)
        assert started["sandbox"] == "danger-full-access"
        assert started["approvalPolicy"] == "untrusted"


@pytest.mark.parametrize(
    "bad", ["relative", "missing", "file", "data", "parent", "other", "symlink"]
)
def test_bad_workspace_has_a_cause_code_and_english_compatibility_message(official, tmp_path, bad):
    _, settings, _, _, headers, client = official
    file = tmp_path / "document.txt"
    file.write_text("fixture")
    other = settings.data_dir.with_name(settings.data_dir.name + "-workspaces") / "other"
    other.mkdir(parents=True)
    link = tmp_path / "link"
    link.symlink_to(settings.data_dir, target_is_directory=True)
    paths = {
        "relative": "project",
        "missing": str(tmp_path / "missing"),
        "file": str(file),
        "data": str(settings.data_dir),
        "parent": str(settings.data_dir.parent),
        "other": str(other),
        "symlink": str(link),
    }
    with client() as http:
        response = http.put(
            "/v1/agents/helper/working-folder", headers=headers, json={"path": paths[bad]}
        )
    assert response.status_code == 400, response.text
    assert response.json()["code"] == "invalid_working_folder"
    assert "working folder" in response.json()["message"].lower()
    assert response.json()["refusal"]["cause_code"].startswith("working_folder_")


@pytest.mark.parametrize(
    "official,tool",
    [
        ("claude-code", "Bash"),
        ("claude-code", "Edit"),
        ("claude-code", "WebFetch"),
        ("codex", "Bash"),
        ("codex", "Edit"),
    ],
    indirect=["official"],
)
@pytest.mark.parametrize("approve", [False, True])
@pytest.mark.parametrize("subagent", [False, True])
def test_host_action_waits_for_permission_and_reports_activity(
    official, tmp_path, tool, approve, subagent
):
    _, _, _, _, headers, client = official
    arguments = (
        {"command": "echo fixture"}
        if tool == "Bash"
        else {"url": "https://example.test"}
        if tool == "WebFetch"
        else {"file_path": "fixture.md", "old_string": "before", "new_string": "after"}
    )
    (tmp_path / "operation.json").write_text(
        json.dumps({"tool": tool, "arguments": arguments, "subagent": subagent})
    )
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="ACTION")
        session = until(ws, "chat.session", frames)["session_id"]
        permission = until(ws, "approval.required", frames)
        assert not any(row.get("executed") for row in records(tmp_path))
        send(
            ws,
            "approval.respond",
            session_id=session,
            turn_id=turn,
            request_id=permission["request_id"],
            approved=approve,
            choice="once" if approve else "deny",
        )
        until(ws, "chat.completed", frames)
    assert any(row.get("executed") for row in records(tmp_path)) is approve
    items = [f["data"] for f in frames if f["event"] == "chat.item"]
    assert any(
        ("echo fixture" if tool == "Bash" else "WebFetch" if tool == "WebFetch" else "fixture.md")
        in json.dumps(item)
        for item in items
    )
    assert any(item["phase"] == ("completed" if approve else "failed") for item in items)


@pytest.mark.parametrize("official", ["claude-code"], indirect=True)
def test_file_content_is_not_interpreted_as_a_shell_command(official, tmp_path):
    _, _, _, _, headers, client = official
    action = {
        "tool": "Edit",
        "arguments": {
            "file_path": "story.md",
            "old_string": "before",
            "new_string": "It's a sunny day.",
        },
    }
    (tmp_path / "operation.json").write_text(json.dumps(action))
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        turn = send(ws, "chat.send", text="ACTION")
        session = until(ws, "chat.session", frames)["session_id"]
        while (frame := ws.receive_json())["event"] not in {"approval.required", "chat.completed"}:
            frames.append(frame)
        assert frame["event"] == "approval.required", frame
        send(
            ws,
            "approval.respond",
            session_id=session,
            turn_id=turn,
            request_id=frame["data"]["request_id"],
            approved=True,
            choice="once",
        )
        until(ws, "chat.completed", frames)
    assert any(row.get("executed") for row in records(tmp_path))


def test_other_configured_workspace_and_storage_key_are_protected(official, tmp_path):
    _, settings, store, _, headers, client = official
    from orbit_hub.assistant.agent_config import AgentConfigStore

    other_project = tmp_path / "other-project"
    other_project.mkdir()
    # Seed the other assistant's configuration, as a previous hub lifetime would.
    AgentConfigStore(store).set_working_folder("other", str(other_project))
    with client() as http:
        for path in (other_project, settings.key_file):
            response = http.put(
                "/v1/agents/helper/working-folder", headers=headers, json={"path": str(path)}
            )
            assert response.status_code == 400, response.text
            assert response.json()["code"] == "invalid_working_folder"
        assert (
            http.put(
                "/v1/agents/helper/working-folder", json={"path": str(other_project)}
            ).status_code
            == 401
        )
        assert (
            http.put(
                "/v1/agents/missing/working-folder",
                headers=headers,
                json={"path": str(other_project)},
            ).status_code
            == 404
        )


def test_workspace_root_cannot_include_future_assistants(official):
    _, _, _, _, headers, client = official
    from pathlib import Path

    with client() as http:
        current = http.get("/v1/agents/helper/config", headers=headers).json()["working_folder"]
        Path(current).parent.mkdir(parents=True)
        response = http.put(
            "/v1/agents/helper/working-folder",
            headers=headers,
            json={"path": str(Path(current).parent)},
        )
        assert response.status_code == 400, response.text
        assert response.json()["code"] == "invalid_working_folder"


def test_a_removed_workspace_can_be_repaired_from_settings(official, tmp_path):
    _, _, _, _, headers, client = official
    project = tmp_path / "project"
    replacement = tmp_path / "replacement"
    project.mkdir()
    replacement.mkdir()
    with client() as http:
        assert (
            http.put(
                "/v1/agents/helper/working-folder", headers=headers, json={"path": str(project)}
            ).status_code
            == 200
        )
        project.rmdir()
        config = http.get("/v1/agents/helper/config", headers=headers)
        assert config.status_code == 200, config.text
        assert config.json()["working_folder"] == str(project)
        assert (
            http.put(
                "/v1/agents/helper/working-folder", headers=headers, json={"path": str(replacement)}
            ).status_code
            == 200
        )


@pytest.mark.parametrize("official", ["claude-code"], indirect=True)
@pytest.mark.parametrize(
    ("tool", "arguments"),
    [
        ("Read", {"file_path": "fixture.md"}),
        # The session's own bookkeeping changes nothing outside it.
        ("TodoWrite", {"todos": [{"content": "fixture", "status": "pending"}]}),
        ("Task", {"description": "fixture", "prompt": "fixture"}),
    ],
)
def test_reads_and_own_bookkeeping_have_no_approval_card(official, tmp_path, tool, arguments):
    _, _, _, _, headers, client = official
    (tmp_path / "operation.json").write_text(json.dumps({"tool": tool, "arguments": arguments}))
    frames = []
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        until(ws, "ready", frames)
        send(ws, "chat.send", text="ACTION")
        until(ws, "chat.completed", frames)
    assert not any(f["event"] == "approval.required" for f in frames)
    assert any(row.get("executed") for row in records(tmp_path))


@pytest.mark.parametrize("target", ["home", "ancestor", "code", "venv", "contains-hidden"])
def test_workspace_cannot_replace_owner_or_hub_code(official, tmp_path, monkeypatch, target):
    from orbit_hub.assistant.jail import installed_paths

    _, settings, _, _, headers, client = official
    home = tmp_path / "owner"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    project = tmp_path / "project"
    (project / ".ssh").mkdir(parents=True)
    targets = {
        "home": home,
        "ancestor": home.parent,
        "code": installed_paths()[0],
        "venv": installed_paths()[1],
        "contains-hidden": settings.data_dir.parent,
    }
    with client() as http:
        response = http.put(
            "/v1/agents/helper/working-folder", headers=headers, json={"path": str(targets[target])}
        )
    assert response.status_code == 400, response.text
    assert response.json()["code"] == "invalid_working_folder"
    assert "working folder" in response.json()["message"]
