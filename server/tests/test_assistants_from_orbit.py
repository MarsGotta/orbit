"""Assistants from Orbit: each agent's profile, adding and removing Claude Code
and Codex agents, and handing a conversation over to another agent.

Every agent runs on the suite's synthetic adapter (`conftest.FakeAdapter`):
what is tested is the hub's registry, its routes and what reaches the Mac.
Names, ids and conversations are invented.
"""

import json
import sys
from dataclasses import replace
from uuid import uuid4

import pytest
from conftest import device
from fastapi.testclient import TestClient

from orbit_hub.app import create_app
from orbit_hub.assistant.agents import Agents
from orbit_hub.assistant.roster import MAX_AGENTS, Roster
from orbit_hub.config import Settings

TWO_AGENTS = (
    {"id": "deimos", "name": "Deimos", "kind": "hermes", "profile": "deimos-profile"},
    {"id": "lyra", "name": "Lyra", "kind": "hermes", "profile": "lyra-profile"},
)
ALL_ON = {
    "screen": True,
    "control": True,
    "calendar": True,
    "meetings": True,
    "files": True,
    "notes": True,
}


def with_adapters(settings, tmp_path, *services):
    """The hub's `[assistant_tools]`: an executable that exists, and orbit-mcp's TOML path."""
    return replace(
        settings,
        agents=TWO_AGENTS,
        assistant_tools={
            service: {"command": [sys.executable], "orbit_mcp_config": str(tmp_path / "mcp.toml")}
            for service in services
        },
    )


def hub(settings, store, adapter, now=1001):
    return TestClient(
        create_app(settings, store=store, assistant=adapter, speech=adapter, now=lambda: now),
        base_url="https://localhost",
    )


def auth(paired):
    return {"Authorization": "Bearer " + paired["token"]}


def socket(http, paired):
    return http.websocket_connect("wss://localhost/v1/ws", headers=auth(paired))


def profile(name, description="", color="c2", primary=False):
    return {"name": name, "description": description, "color": color, "primary": primary}


def new_agent(service="codex", name="Phobos", **changes):
    return {
        "service": service,
        "name": name,
        "description": "Programa conmigo",
        "color": "c2",
        "access": ALL_ON,
        **changes,
    }


# Profile ---------------------------------------------------------------------


def test_ready_carries_each_agents_profile_and_the_primary(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ready = ws.receive_json()["data"]
    assert ready["default_agent_id"] == "deimos"
    assert ready["addable_agents"] == ["codex", "claude-code"]
    assert [
        (agent["id"], agent["service"], agent["description"], agent["color"], agent["removable"])
        for agent in ready["agents"]
    ] == [("deimos", "hermes", "", "c1", False), ("lyra", "hermes", "", "c2", False)]
    # Only an agent Orbit can sign in has an account to publish.
    assert all("account" not in agent for agent in ready["agents"])


@pytest.mark.parametrize(
    ("sent", "status", "code", "numbers"),
    [
        (profile(""), 422, "agent_profile_invalid", {"field": "name", "rule": "empty"}),
        (profile("   "), 422, "agent_profile_invalid", {"field": "name", "rule": "empty"}),
        (
            profile("A" * 25),
            422,
            "agent_profile_invalid",
            {"field": "name", "rule": "too_long", "characters": 25, "max_characters": 24},
        ),
        (
            profile("Ly\u0000ra"),
            422,
            "agent_profile_invalid",
            {"field": "name", "rule": "control_characters"},
        ),
        (
            profile("Lyra", "x" * 91),
            422,
            "agent_profile_invalid",
            {"field": "description", "characters": 91, "max_characters": 90},
        ),
        (
            profile("Lyra", "una\nlínea"),
            422,
            "agent_profile_invalid",
            {"field": "description", "rule": "control_characters"},
        ),
        (profile("Lyra", color="c5"), 422, "agent_profile_invalid", {"field": "color"}),
        # Said the same as Deimos, ignoring case and accents.
        (profile("DÉIMOS"), 409, "agent_name_taken", {"field": "name", "agent_id": "deimos"}),
        ({"name": "Lyra", "color": "c1"}, 400, "invalid_agent_config", {}),
        ({**profile("Lyra"), "command": ["/bin/sh"]}, 400, "invalid_agent_config", {}),
    ],
)
def test_a_profile_is_refused_with_its_field_and_numbers(
    environment, tmp_path, sent, status, code, numbers
):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path)
    paired = device(store)
    with hub(settings, store, adapter) as http:
        answer = http.put("/v1/agents/lyra/profile", json=sent, headers=auth(paired))
        after = http.get("/v1/agents", headers=auth(paired)).json()
    assert answer.status_code == status, answer.json()
    assert answer.json()["code"] == code
    assert numbers.items() <= answer.json()["refusal"].items()
    assert [agent["name"] for agent in after["agents"]] == ["Deimos", "Lyra"]


def test_the_primary_stays_until_another_one_is_chosen(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path)
    paired = device(store)
    with hub(settings, store, adapter) as http:
        refused = http.put(
            "/v1/agents/deimos/profile", json=profile("Deimos", color="c1"), headers=auth(paired)
        )
        chosen = http.put(
            "/v1/agents/lyra/profile", json=profile("Lyra", primary=True), headers=auth(paired)
        )
    assert refused.status_code == 409
    assert refused.json()["refusal"]["code"] == "agent_primary_required"
    assert chosen.json()["default_agent_id"] == "lyra"


def test_a_profile_changes_how_orbit_shows_and_calls_the_agent(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path)
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ws.receive_json()
        answer = http.put(
            "/v1/agents/deimos/profile",
            json=profile("  Eris ", "Tu asistente de siempre", "c3", primary=True),
            headers=auth(paired),
        )
        assert answer.status_code == 200, answer.json()
        changed = ws.receive_json()
    assert changed["event"] == "agents.changed"
    assert changed["data"] == answer.json()
    eris = changed["data"]["agents"][0]
    assert (eris["id"], eris["name"], eris["description"], eris["color"]) == (
        "deimos",
        "Eris",
        "Tu asistente de siempre",
        "c3",
    )
    # Hermes was never asked anything: renaming is Orbit's alone.
    assert adapter.connections == []
    # Kept by the hub, over what orbit.toml says, after a restart.
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ready = ws.receive_json()["data"]
    assert ready["assistant_name"] == "Eris"
    assert [agent["name"] for agent in ready["agents"]] == ["Eris", "Lyra"]


# Adding and removing ---------------------------------------------------------


def test_an_agent_added_from_orbit_runs_the_hubs_adapter(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ws.receive_json()
        answer = http.post(
            "/v1/agents",
            json=new_agent(access={**ALL_ON, "control": False}),
            headers=auth(paired),
        )
        assert answer.status_code == 201, answer.json()
        changed = ws.receive_json()
        added = answer.json()["agent_id"]
        config = http.get(f"/v1/agents/{added}/config", headers=auth(paired)).json()
    assert changed["event"] == "agents.changed"
    assert added.startswith("codex-")
    phobos = changed["data"]["agents"][-1]
    assert phobos == {
        "id": added,
        "name": "Phobos",
        "kind": "codex",
        "capabilities": ["images", "voice", "resume"],
        "service": "codex",
        "description": "Programa conmigo",
        "color": "c2",
        "removable": True,
        "account": "signed_out",
    }
    assert config["access"]["groups"]["control"] is False
    # It is the hub's: it is there after a restart, on the adapter orbit.toml names.
    with hub(settings, store, adapter) as http:
        listing = http.get("/v1/agents", headers=auth(paired)).json()
    assert [agent["id"] for agent in listing["agents"]] == ["deimos", "lyra", added]
    registry = Agents.from_settings(settings, adapter, adapter, None, Roster(store))
    assert registry.by_id[added].settings.command == (sys.executable,)


@pytest.mark.parametrize(
    ("sent", "status", "code", "numbers"),
    [
        (new_agent("hermes"), 403, "agent_add_unsupported", {"service": "hermes"}),
        (new_agent("openai"), 403, "agent_add_unsupported", {"service": "openai"}),
        (new_agent(name="lyra"), 409, "agent_name_taken", {"agent_id": "lyra"}),
        # Nothing the Mac sends becomes a command or a path.
        ({**new_agent(), "command": ["/bin/sh", "-c", "id"]}, 400, "invalid_agent_config", {}),
        ({**new_agent(), "id": "../../etc"}, 400, "invalid_agent_config", {}),
        (new_agent("../codex"), 400, "invalid_agent_config", {}),
    ],
)
def test_adding_is_refused_with_its_cause(environment, tmp_path, sent, status, code, numbers):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    paired = device(store)
    with hub(settings, store, adapter) as http:
        answer = http.post("/v1/agents", json=sent, headers=auth(paired))
        after = http.get("/v1/agents", headers=auth(paired)).json()
    assert answer.status_code == status, answer.json()
    assert answer.json()["code"] == code
    assert numbers.items() <= answer.json()["refusal"].items()
    assert [agent["id"] for agent in after["agents"]] == ["deimos", "lyra"]
    assert not (settings.data_dir / "agents").exists()


def test_an_adapter_that_cannot_start_is_refused_naming_the_service(environment, tmp_path):
    settings, store, _adapter = environment
    settings = replace(
        with_adapters(settings, tmp_path),
        assistant_tools={
            "codex": {
                "command": [str(tmp_path / "missing-codex")],
                "orbit_mcp_config": str(tmp_path / "mcp.toml"),
            }
        },
    )
    registry = Agents.from_settings(settings, None, None, None, Roster(store))
    with pytest.raises(Exception) as refused:
        registry.add("codex", registry_profile("Phobos"), 1)
    assert refused.value.code == "agent_adapter_missing"
    assert refused.value.numbers == {
        "service": "codex",
        "path": str(tmp_path / "missing-codex"),
        "cause": "does not exist",
        "cause_code": "adapter_path_missing",
    }
    assert list(registry.by_id) == ["deimos", "lyra"]


def registry_profile(name):
    from orbit_hub.assistant.roster import Profile

    return Profile(name, "", "c1")


def test_a_hub_offers_at_most_32_agents(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    paired = device(store)
    with hub(settings, store, adapter) as http:
        for number in range(MAX_AGENTS - len(TWO_AGENTS)):
            answer = http.post(
                "/v1/agents", json=new_agent(name=f"Agente {number}"), headers=auth(paired)
            )
            assert answer.status_code == 201, answer.json()
        refused = http.post("/v1/agents", json=new_agent(name="Uno más"), headers=auth(paired))
    assert refused.status_code == 409
    expected = {"code": "too_many_agents", "count": 32, "max": 32}
    assert expected.items() <= refused.json()["refusal"].items()


def test_the_hub_refuses_to_start_with_more_than_32_agents(tmp_path):
    entries = "".join(
        f'\n[[agents]]\nid = "agente-{number}"\nname = "Agente {number}"\nkind = "hermes"\n'
        f'profile = "perfil-{number}"\n'
        for number in range(MAX_AGENTS + 1)
    )
    path = tmp_path / "orbit.toml"
    path.write_text(
        'data_dir = "/fixture/data"\nkey_file = "/fixture/key"\n'
        'hermes_url = "https://hermes.example.test"\nhermes_credential_file = "/fixture/token"\n'
        + entries
        + '\n[meeting_transcription]\nendpoint = "https://transcriber.example.test"\n'
        'model = "fixture-model"\n'
    )
    with pytest.raises(ValueError, match=r"33 entries; a hub offers at most 32 agents"):
        Settings.load(path)


def test_removing_an_added_agent_keeps_its_conversations(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    paired = device(store)
    with hub(settings, store, adapter) as http:
        added = http.post("/v1/agents", json=new_agent(), headers=auth(paired)).json()["agent_id"]
        http.put(
            f"/v1/agents/{added}/profile",
            json=profile("Phobos", primary=True),
            headers=auth(paired),
        )
        folder = settings.data_dir / "agents" / added / "codex-home"
        folder.mkdir(parents=True)
        (folder / "auth.json").write_text("{}")
        session = store.chat.create_session(
            paired["device_id"], added, "live", "stored", "epoch", 5
        )
        kept = http.delete("/v1/agents/deimos", headers=auth(paired))
        removed = http.delete(f"/v1/agents/{added}", headers=auth(paired))
        rows = http.get("/v1/sessions", headers=auth(paired)).json()["sessions"]
    assert kept.status_code == 403
    assert kept.json()["refusal"]["code"] == "agent_not_removable"
    assert removed.status_code == 200, removed.json()
    assert [agent["id"] for agent in removed.json()["agents"]] == ["deimos", "lyra"]
    # The primary it was falls back to the configured default.
    assert removed.json()["default_agent_id"] == "deimos"
    assert not (settings.data_dir / "agents" / added).exists()
    assert [(row["id"], row["agent_id"]) for row in rows] == [(session, added)]


# Handing a conversation over --------------------------------------------------


def prompts(adapter):
    return [
        params["text"]
        for connection in adapter.connections
        for method, params in connection.calls
        if method == "prompt.submit"
    ]


@pytest.mark.parametrize("another_mac", [False, True])
def test_a_conversation_is_handed_over_with_its_unanswered_question(
    environment, tmp_path, another_mac
):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path)
    paired = device(store)
    source = store.chat.create_session(paired["device_id"], "lyra", "live", "stored", "epoch", 5)
    for role, text in (
        ("user", "¿Cuántas habitaciones libres hay?"),
        ("assistant", "Quedan doce en el hotel Brisa."),
        ("user", "¿Y para el fin de semana?"),
    ):
        store.chat.append_message(source, role, text, now_ms=1000)
    turn = str(uuid4())
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ws.receive_json()
        ws.send_json(
            {
                "v": 1,
                "id": turn,
                "method": "chat.send",
                "params": {"text": "", "agent_id": "deimos", "handover_from": source},
            }
        )
        while (frame := ws.receive_json())["event"] not in {"chat.completed", "error"}:
            pass
        assert frame["event"] == "chat.completed", frame
        new = frame["data"]["session_id"]
        rows = http.get("/v1/sessions", headers=auth(paired)).json()["sessions"]
        page = http.get(f"/v1/sessions/{new}/messages", headers=auth(paired)).json()
    [sent] = prompts(adapter)
    assert "You are taking over the conversation with Lyra" in sent
    assert "User: ¿Cuántas habitaciones libres hay?\nLyra: Quedan doce en el hotel Brisa." in sent
    assert sent.endswith("\n¿Y para el fin de semana?")
    assert sent.count("¿Y para el fin de semana?") == 1
    # The new conversation keeps the question as the user sees it, and where it came from.
    assert page["messages"][0]["text"] == "¿Y para el fin de semana?"
    assert page["agent_id"] == "deimos"
    assert page["handed_over_from"] == {"session_id": source, "agent_id": "lyra"}
    by_id = {row["id"]: row for row in rows}
    assert "handed_over_from" not in by_id[new]
    assert "handed_over_from" not in by_id[source]
    # After the hub restarts, both Macs read the same durable origin on opening.
    reader = device(store, "Segundo Mac") if another_mac else paired
    with hub(settings, store, adapter, now=1001 + 86_400) as http:
        reopened = http.get(f"/v1/sessions/{new}/messages", headers=auth(reader))
        assert reopened.status_code == 200, reopened.json()
        assert reopened.json()["handed_over_from"] == page["handed_over_from"]
        unauthenticated = http.get(f"/v1/sessions/{new}/messages")
        assert unauthenticated.status_code == 401


def test_a_handover_keeps_the_newest_messages_within_its_ceiling(environment, tmp_path):
    from orbit_hub.assistant.handover import MAX_HANDOVER_CHARS, handover_turn

    messages = [
        {"role": "user" if index % 2 == 0 else "assistant", "text": f"{index:04d} " + "x" * 995}
        for index in range(80)
    ]
    turn = handover_turn(messages, 300, "¿Seguimos?", "Lyra")
    assert turn is not None
    carried = [line for line in turn.text.splitlines() if line.startswith(("User:", "Lyra:"))]
    assert sum(len(line) + 1 for line in carried) <= MAX_HANDOVER_CHARS
    # The newest are kept, and the rest are counted, with the ceiling.
    assert carried[-1].startswith("Lyra: 0079")
    assert f"{300 - len(carried)} earlier messages are omitted" in turn.text
    assert f"ceiling of {MAX_HANDOVER_CHARS} characters" in turn.text
    assert turn.visible == "¿Seguimos?"


def test_a_handover_without_a_question_is_refused(environment, tmp_path):
    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path)
    paired = device(store)
    source = store.chat.create_session(paired["device_id"], "lyra", "live", "stored", "epoch", 5)
    store.chat.append_message(source, "user", "Hola", now_ms=1000)
    store.chat.append_message(source, "assistant", "Hola, dime", now_ms=1001)
    turn = str(uuid4())
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ws.receive_json()
        ws.send_json(
            {
                "v": 1,
                "id": turn,
                "method": "chat.send",
                "params": {"text": "", "agent_id": "deimos", "handover_from": source},
            }
        )
        while (frame := ws.receive_json())["event"] != "error":
            pass
    assert frame["data"]["code"] == "handover_no_question"
    assert prompts(adapter) == []
    assert json.dumps(frame).count("Hola") == 0


def test_a_handover_carries_the_question_once_when_the_mac_repeats_it():
    from orbit_hub.assistant.handover import handover_turn

    messages = [
        {"role": "user", "text": "Hola"},
        {"role": "assistant", "text": "Hola, dime"},
        {"role": "user", "text": "¿Qué toca hoy?"},
    ]
    repeated = handover_turn(messages, 3, "¿Qué toca hoy?", "Lyra")
    other = handover_turn(messages, 3, "Mejor, ¿qué toca mañana?", "Lyra")
    assert repeated is not None and other is not None
    assert repeated.text.count("¿Qué toca hoy?") == 1
    assert repeated.text.endswith("\n¿Qué toca hoy?")
    # A new question keeps the old one in the copy, unanswered.
    assert "User: ¿Qué toca hoy?" in other.text
    assert other.text.endswith("\nMejor, ¿qué toca mañana?")


def test_the_hub_refuses_to_start_when_added_agents_pass_the_ceiling(environment, tmp_path):
    from orbit_hub.assistant.roster import Profile

    settings, store, adapter = environment
    settings = with_adapters(settings, tmp_path, "codex")
    registry = Agents.from_settings(settings, adapter, adapter, None, Roster(store))
    for number in range(MAX_AGENTS - len(TWO_AGENTS)):
        registry.add("codex", Profile(f"Agente {number}", "", "c1"), number)
    # The operator lists one more in orbit.toml: 33 together.
    more = replace(
        settings,
        agents=(*TWO_AGENTS, {"id": "vega", "name": "Vega", "kind": "hermes", "profile": "vega"}),
    )
    with pytest.raises(ValueError, match=r"3 agents in its configuration and 30 added from Orbit"):
        Agents.from_settings(more, adapter, adapter, None, Roster(store))
