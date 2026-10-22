"""Signing a Claude Code or Codex agent in and out from Orbit.

The vendor's tool is `support_vendor_cli.py`, an invented sign-in that
prints what claude-agent-acp's and codex-acp's do and writes an invented
token where they would. What runs for real is the hub: its routes, its
process handling and its reading of the tool's output. Addresses, codes,
accounts and the token are invented.
"""

import json
import logging
import os
import sys
import time
from dataclasses import replace
from pathlib import Path

import pytest
from conftest import device
from fastapi.testclient import TestClient
from support_client_e2e import DAEMON_E2E_TIMEOUT_SECONDS
from support_vendor_cli import CLAUDE_URL, CODEX_URL, PASTED, TOKEN

from orbit_hub.app import create_app
from orbit_hub.assistant.agents import Agents
from orbit_hub.assistant.roster import Profile, Roster

CLI = Path(__file__).with_name("support_vendor_cli.py")
ALL_ON = {"screen": True, "control": True, "calendar": True, "meetings": True, "files": True}
# Waits observe the sign-in's outcome; load delays it instead of failing it.
WAIT_SECONDS = DAEMON_E2E_TIMEOUT_SECONDS


def adapters(tmp_path, *flags, claude_flags=()):
    command = [sys.executable, str(CLI), "--record", str(tmp_path / "record.jsonl"), *flags]
    return {
        "claude-code": {
            "command": [*command, *claude_flags],
            "orbit_mcp_config": str(tmp_path / "mcp.toml"),
        },
        "codex": {"command": command, "orbit_mcp_config": str(tmp_path / "mcp.toml")},
    }


def records(tmp_path):
    path = tmp_path / "record.jsonl"
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


class Hub:
    """The hub over HTTP and one device's socket, keeping everything either said."""

    def __init__(self, settings, store, adapter):
        self.http = TestClient(
            create_app(settings, store=store, assistant=adapter, speech=adapter),
            base_url="https://localhost",
        )
        self.paired = device(store)
        self.said: list[str] = []

    def __enter__(self):
        self.http.__enter__()
        return self

    def __exit__(self, *exc):
        return self.http.__exit__(*exc)

    def call(self, method, path, **body):
        answer = self.http.request(
            method,
            path,
            json=body or None,
            headers={"Authorization": "Bearer " + self.paired["token"]},
        )
        self.said.append(answer.text)
        return answer

    def add(self, service, name):
        answer = self.call(
            "POST",
            "/v1/agents",
            service=service,
            name=name,
            description="",
            color="c2",
            access=ALL_ON,
        )
        assert answer.status_code == 201, answer.json()
        return answer.json()["agent_id"]

    def outcome(self, agent_id):
        """The sign-in once it is no longer waiting."""
        deadline = time.monotonic() + WAIT_SECONDS
        while time.monotonic() < deadline:
            attempt = self.call("GET", f"/v1/agents/{agent_id}/sign-in").json()
            if attempt["state"] != "waiting":
                return attempt
            time.sleep(0.05)
        raise AssertionError("the sign-in never ended")

    def agent(self, agent_id):
        listing = self.call("GET", "/v1/agents").json()
        return next(agent for agent in listing["agents"] if agent["id"] == agent_id)


@pytest.fixture
def signing(environment, tmp_path, caplog):
    settings, store, adapter = environment
    caplog.set_level(logging.DEBUG)

    def hub(*flags, claude_flags=()):
        configured = replace(
            settings, acp_adapters=adapters(tmp_path, *flags, claude_flags=claude_flags)
        )
        return Hub(configured, store, adapter)

    return hub, settings


def assert_no_token(hub, settings, caplog, capfd):
    """The invented token stays in the agent's folder: in no answer, log or output."""
    printed = capfd.readouterr()
    for text in [*hub.said, caplog.text, printed.out, printed.err]:
        assert TOKEN not in text
    folder = settings.data_dir / "agents"
    assert any(TOKEN in path.read_text() for path in folder.rglob("*") if path.is_file())


def test_claude_signs_in_with_the_code_its_page_shows(signing, tmp_path, caplog, capfd):
    open_hub, settings = signing
    with open_hub() as hub:
        # A name is a label: it never becomes a path or an argument.
        agent = hub.add("claude-code", "../../Ares --console")
        started = hub.call("POST", f"/v1/agents/{agent}/sign-in")
        assert started.status_code == 200, started.json()
        refused = hub.call("POST", f"/v1/agents/{agent}/sign-in/code", code="sin almohadilla\n")
        sent = hub.call("POST", f"/v1/agents/{agent}/sign-in/code", code=PASTED)
        done = hub.outcome(agent)
        ares = hub.agent(agent)
    assert started.json() == {
        "agent_id": agent,
        "state": "waiting",
        "expires_at_ms": started.json()["expires_at_ms"],
        "expects_code": True,
        "url": CLAUDE_URL,
    }
    assert refused.status_code == 422
    assert refused.json()["refusal"]["code"] == "sign_in_code_invalid"
    assert sent.status_code == 200
    assert done == {"agent_id": agent, "state": "done"}
    assert ares["account"] == "signed_in"
    # The adapter the hub configured, with the subscription's sign-in, in the agent's home.
    [login] = records(tmp_path)
    assert login["arguments"] == ["--cli", "auth", "login", "--claudeai"]
    assert login["home"] == str(settings.data_dir / "agents" / agent / "claude-config")
    credentials = settings.data_dir / "agents" / agent / "claude-config" / ".credentials.json"
    assert credentials.stat().st_mode & 0o777 == 0o600
    assert_no_token(hub, settings, caplog, capfd)


def test_an_adapter_that_refuses_subscriptions_signs_in_with_a_console_account(signing, tmp_path):
    open_hub, _settings = signing
    with open_hub(claude_flags=["--hide-claude-auth"]) as hub:
        agent = hub.add("claude-code", "Ares")
        hub.call("POST", f"/v1/agents/{agent}/sign-in")
        hub.call("DELETE", f"/v1/agents/{agent}/sign-in")
    [login] = records(tmp_path)
    assert login["arguments"][-1] == "--console"


def test_codex_signs_in_with_a_device_code(signing, tmp_path, caplog, capfd):
    open_hub, settings = signing
    gate = tmp_path / "aprobado"
    with open_hub("--gate", str(gate)) as hub:
        agent = hub.add("codex", "Phobos")
        started = hub.call("POST", f"/v1/agents/{agent}/sign-in").json()
        again = hub.call("POST", f"/v1/agents/{agent}/sign-in").json()
        gate.touch()
        done = hub.outcome(agent)
        phobos = hub.agent(agent)
    assert (started["state"], started["url"], started["code"]) == (
        "waiting",
        CODEX_URL,
        "ABCD-1234",
    )
    assert started["expects_code"] is False
    # A second press shows the same sign-in, not a new one.
    assert again == started
    assert len(records(tmp_path)) == 1
    assert done == {"agent_id": agent, "state": "done"}
    assert phobos["account"] == "signed_in"
    folder = settings.data_dir / "agents" / agent
    assert (folder / "codex-home" / "auth.json").is_file()
    # The sign-in's own home, where Codex ran, is gone.
    assert sorted(path.name for path in folder.iterdir()) == ["codex-home"]
    assert_no_token(hub, settings, caplog, capfd)


def test_cancelling_a_sign_in_leaves_the_agent_as_it_was(signing, tmp_path):
    open_hub, settings = signing
    with open_hub("--gate", str(tmp_path / "nunca")) as hub:
        agent = hub.add("codex", "Phobos")
        home = settings.data_dir / "agents" / agent / "codex-home"
        home.mkdir(parents=True, mode=0o700)
        (home / "auth.json").write_text("cuenta anterior")
        started = hub.call("POST", f"/v1/agents/{agent}/sign-in").json()
        cancelled = hub.call("DELETE", f"/v1/agents/{agent}/sign-in")
        after = hub.call("GET", f"/v1/agents/{agent}/sign-in").json()
        phobos = hub.agent(agent)
    assert started["state"] == "waiting"
    assert cancelled.json() == {"agent_id": agent, "state": "cancelled"}
    assert after == cancelled.json()
    # Codex deleted the account it had in its own home, not in the agent's.
    assert (home / "auth.json").read_text() == "cuenta anterior"
    assert sorted(path.name for path in home.parent.iterdir()) == ["codex-home"]
    assert phobos["account"] == "signed_out"
    [login] = records(tmp_path)
    with pytest.raises(ProcessLookupError):
        os.kill(login["pid"], 0)


def test_a_failed_sign_in_says_its_cause(signing):
    open_hub, _settings = signing
    with open_hub("--deny") as hub:
        agent = hub.add("codex", "Phobos")
        hub.call("POST", f"/v1/agents/{agent}/sign-in")
        failed = hub.outcome(agent)
    assert failed == {
        "agent_id": agent,
        "state": "failed",
        "cause": "device_code_disabled",
        "exit_status": 1,
    }


def test_a_wrong_code_fails_the_sign_in_with_its_cause(signing):
    open_hub, _settings = signing
    with open_hub() as hub:
        agent = hub.add("claude-code", "Ares")
        hub.call("POST", f"/v1/agents/{agent}/sign-in")
        hub.call("POST", f"/v1/agents/{agent}/sign-in/code", code="otro-codigo#otro-estado")
        failed = hub.outcome(agent)
    assert (failed["state"], failed["cause"], failed["exit_status"]) == (
        "failed",
        "code_rejected",
        1,
    )


def test_signing_out_runs_the_vendors_sign_out(signing, tmp_path):
    open_hub, settings = signing
    gate = tmp_path / "aprobado"
    gate.touch()
    with open_hub("--gate", str(gate)) as hub:
        agent = hub.add("codex", "Phobos")
        hub.call("POST", f"/v1/agents/{agent}/sign-in")
        hub.outcome(agent)
        signed_out = hub.call("POST", f"/v1/agents/{agent}/sign-out")
    assert signed_out.status_code == 200, signed_out.json()
    [phobos] = [a for a in signed_out.json()["agents"] if a["id"] == agent]
    assert phobos["account"] == "signed_out"
    assert [record["arguments"] for record in records(tmp_path)] == [
        ["cli", "login", "--device-auth"],
        ["cli", "logout"],
    ]
    assert not (settings.data_dir / "agents" / agent / "codex-home" / "auth.json").exists()


def test_an_agent_orbit_cannot_sign_in_says_so(signing):
    open_hub, _settings = signing
    with open_hub() as hub:
        refused = hub.call("POST", "/v1/agents/default/sign-in")
        none = hub.call("GET", "/v1/agents/default/sign-in")
    assert refused.status_code == 409
    assert refused.json()["refusal"] | {} == {
        "code": "sign_in_unsupported",
        "agent_id": "default",
        "kind": "hermes",
        "retry": refused.json()["refusal"]["retry"],
    }
    assert none.status_code == 409


def test_what_the_hub_learns_of_an_account_is_published(environment, tmp_path):
    settings, store, adapter = environment
    settings = replace(settings, acp_adapters=adapters(tmp_path))
    agents = Agents.from_settings(settings, adapter, adapter, None, Roster(store))
    added = agents.add("codex", Profile("Phobos", "", "c2"), 1).id
    published = []
    agents.changed = lambda: published.append(agents.public_agent(agents.by_id[added], 1))
    seen = []
    for fact, values in [
        ("signed_in", {}),
        ("status", {"kind": "account", "email": "marta.soler@ejemplo.test", "plan": "Plus"}),
        ("limit", {"until_ms": 5000}),
        ("answered", {}),
        ("auth_required", {}),
        ("answered", {}),
        ("answered", {}),
    ]:
        agents.observe(added, fact, **values)
        seen.append(
            {key: published[-1].get(key) for key in ("account", "account_email", "plan", "limit")}
        )
    assert seen == [
        {"account": "signed_in", "account_email": None, "plan": None, "limit": None},
        {
            "account": "signed_in",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": None,
        },
        {
            "account": "signed_in",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": {"until_ms": 5000},
        },
        {
            "account": "signed_in",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": None,
        },
        # The session expired: published as the agent's state.
        {
            "account": "expired",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": None,
        },
        {
            "account": "signed_in",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": None,
        },
        # Nothing changed: nothing published again.
        {
            "account": "signed_in",
            "account_email": "marta.soler@ejemplo.test",
            "plan": "Plus",
            "limit": None,
        },
    ]
    assert len(published) == 6
    # Kept by the hub across a restart.
    again = Agents.from_settings(settings, adapter, adapter, None, Roster(store))
    assert again.public_agent(again.by_id[added], 2)["account"] == "signed_in"
    # A never signed-in agent asked to sign in stays signed out, not expired.
    other = agents.add("claude-code", Profile("Ares", "", "c3"), 2).id
    agents.observe(other, "auth_required")
    assert agents.account(other).state == "signed_out"
    agents.observe(other, "usage", used_percent=72, window="seven_day", resets_at_ms=9000)
    assert agents.public_agent(agents.by_id[other], 3)["usage"] == {
        "used_percent": 72,
        "window": "seven_day",
        "resets_at_ms": 9000,
    }


def test_an_account_fact_the_disk_refuses_still_reaches_the_macs(environment, tmp_path):
    """A turn learns that its agent's session expired while the disk is full:
    the write fails, the turn is not ended by it, and the Macs are told."""
    import sqlite3

    settings, store, adapter = environment
    settings = replace(settings, acp_adapters=adapters(tmp_path))
    roster = Roster(store)
    agents = Agents.from_settings(settings, adapter, adapter, None, roster)
    added = agents.add("codex", Profile("Phobos", "", "c2"), 1).id
    published = []
    agents.changed = lambda: published.append(agents.account(added).state)

    def full(*_):
        raise sqlite3.OperationalError("database or disk is full")

    roster.save_account = full
    agents.observe(added, "signed_in")
    agents.observe(added, "auth_required")
    assert published == ["signed_in", "expired"]
