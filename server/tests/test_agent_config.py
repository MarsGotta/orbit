"""Settings › Agents over HTTP: personality, skills and access groups of each agent.

An `openai` and an official vendor agent keep theirs in the hub's database; a `hermes`
agent's personality and skills are its profile's, reached through a fake
Hermes panel (`support_hermes_panel`), never a real Hermes. Every refusal is
checked with its code and its scalar numbers, as the Mac decodes them.
"""

import os
import sys
from dataclasses import replace

import pytest
from conftest import device
from fastapi.testclient import TestClient
from support_hermes_panel import TOKEN, FakeHermesPanel
from support_openai_compatible import closed_port, serve

from orbit_hub.app import create_app
from orbit_hub.assistant.agent_config import MAX_PERSONALITY_BYTES, AgentConfigStore
from orbit_hub.assistant.skills import MAX_SKILL_BYTES, MAX_SKILLS, parse

PROFILE = "atlas-perfil"
EVERY_GROUP = {
    "screen": True,
    "control": True,
    "calendar": True,
    "meetings": True,
    "files": True,
    "notes": True,
}


def skill_md(name, description="Resume la semana en cinco puntos.", body="1. Lee.\n2. Resume.\n"):
    return f"---\nname: {name}\ndescription: {description}\n---\n{body}"


def entries(tmp_path):
    mcp_config = tmp_path / "mcp.toml"
    mcp_config.write_text("# orbit-mcp validates its own file\n")
    return (
        {"id": "atlas", "name": "Atlas", "kind": "hermes", "profile": PROFILE},
        {
            "id": "lyra",
            "name": "Lyra",
            "kind": "openai",
            "base_url": "https://ia.example/v1",
            "model": "modelo-sintetico",
            "orbit_mcp_config": str(mcp_config),
        },
        {
            "id": "vega",
            "name": "Vega",
            "kind": "codex",
            # Executable and never run: no session opens here.
            "command": [sys.executable],
            "orbit_mcp_config": str(mcp_config),
        },
    )


@pytest.fixture
def hub(environment, tmp_path):
    """The hub with one agent of each kind, and the fake Hermes panel behind Atlas."""
    settings, store, _ = environment
    credential = tmp_path / "hermes.credential"
    credential.write_text(TOKEN + "\n")
    os.chmod(credential, 0o600)
    panel = FakeHermesPanel(PROFILE)
    with serve(panel.app) as url:
        settings = replace(
            settings,
            hermes_url=url,
            hermes_credential_file=credential,
            allow_insecure_localhost=True,
            agents=entries(tmp_path),
        )
        client = TestClient(
            create_app(settings, store=store, now=lambda: 1001), base_url="https://localhost"
        )
        owner = device(store)
        yield client, {"Authorization": "Bearer " + owner["token"]}, panel, store


def refusal(response, status, code):
    assert response.status_code == status, response.text
    payload = response.json()
    assert payload["code"] == code
    numbers = {k: v for k, v in payload["refusal"].items() if k not in {"code", "retry"}}
    # The Mac decodes refusal numbers as integers, strings or booleans: never a list.
    assert all(isinstance(value, str | int | bool) for value in numbers.values()), numbers
    return numbers


# Agents whose personality and skills the hub keeps -----------------------------


@pytest.mark.parametrize(("agent_id", "kind"), [("lyra", "openai"), ("vega", "codex")])
def test_an_agent_nobody_configured_reads_empty_and_with_every_group_on(hub, agent_id, kind):
    client, headers, _, _ = hub
    response = client.get(f"/v1/agents/{agent_id}/config", headers=headers)
    assert response.status_code == 200, response.text
    assert response.json() == {
        "agent_id": agent_id,
        "kind": kind,
        **({"working_folder": response.json()["working_folder"]} if kind == "codex" else {}),
        "personality": {"text": "", "editable": True, "max_bytes": MAX_PERSONALITY_BYTES},
        "access": {"groups": EVERY_GROUP, "editable": True},
        "skills": {
            "editable": True,
            "max_skills": MAX_SKILLS,
            "max_skill_bytes": MAX_SKILL_BYTES,
            "items": [],
        },
    }


def test_the_routes_need_a_paired_device_and_a_known_agent(hub):
    client, headers, _, _ = hub
    assert client.get("/v1/agents/lyra/config").status_code == 401
    numbers = refusal(
        client.get("/v1/agents/orion/config", headers=headers), 404, "agent_not_found"
    )
    assert numbers == {"agent_id": "orion", "agent_ids": "atlas,lyra,vega"}


def test_a_personality_is_kept_and_refused_past_its_ceiling(hub):
    client, headers, _, store = hub
    text = "Eres Lyra. Hablas claro y breve.\n\n- Tuteas.\n"
    written = client.put("/v1/agents/lyra/personality", headers=headers, json={"text": text})
    assert written.status_code == 200, written.text
    assert written.json()["personality"]["text"] == text
    assert (
        client.get("/v1/agents/lyra/config", headers=headers).json()["personality"]["text"] == text
    )
    # Another agent's is its own.
    assert client.get("/v1/agents/vega/config", headers=headers).json()["personality"]["text"] == ""
    large = "é" * (MAX_PERSONALITY_BYTES // 2 + 1)
    numbers = refusal(
        client.put("/v1/agents/lyra/personality", headers=headers, json={"text": large}),
        413,
        "personality_too_large",
    )
    assert numbers == {"bytes": len(large.encode()), "max_bytes": MAX_PERSONALITY_BYTES}
    assert AgentConfigStore(store).personality("lyra") == text
    # An empty text clears it.
    client.put("/v1/agents/lyra/personality", headers=headers, json={"text": ""})
    assert AgentConfigStore(store).personality("lyra") == ""


def test_a_skill_is_created_read_replaced_switched_and_deleted(hub):
    client, headers, _, _ = hub
    created = client.put(
        "/v1/agents/lyra/skills/resumen-semanal",
        headers=headers,
        json={"content": skill_md("resumen-semanal")},
    )
    assert created.status_code == 200, created.text
    assert created.json()["skills"]["items"] == [
        {
            "name": "resumen-semanal",
            "description": "Resume la semana en cinco puntos.",
            "enabled": True,
        }
    ]
    read = client.get("/v1/agents/lyra/skills/resumen-semanal", headers=headers)
    assert read.json() == {
        "name": "resumen-semanal",
        "description": "Resume la semana en cinco puntos.",
        "enabled": True,
        "content": skill_md("resumen-semanal"),
    }
    switched = client.put(
        "/v1/agents/lyra/skills/resumen-semanal/enabled", headers=headers, json={"enabled": False}
    )
    assert switched.json()["skills"]["items"][0]["enabled"] is False
    replaced = client.put(
        "/v1/agents/lyra/skills/resumen-semanal",
        headers=headers,
        json={"content": skill_md("resumen-semanal", description="Otra descripción.")},
    )
    # Replacing the file keeps whether it was enabled.
    assert replaced.json()["skills"]["items"] == [
        {"name": "resumen-semanal", "description": "Otra descripción.", "enabled": False}
    ]
    assert client.get("/v1/agents/vega/config", headers=headers).json()["skills"]["items"] == []
    deleted = client.delete("/v1/agents/lyra/skills/resumen-semanal", headers=headers)
    assert deleted.json()["skills"]["items"] == []
    for response in (
        client.get("/v1/agents/lyra/skills/resumen-semanal", headers=headers),
        client.delete("/v1/agents/lyra/skills/resumen-semanal", headers=headers),
        client.put(
            "/v1/agents/lyra/skills/resumen-semanal/enabled",
            headers=headers,
            json={"enabled": True},
        ),
    ):
        assert refusal(response, 404, "skill_not_found") == {"name": "resumen-semanal"}


def test_an_invalid_skill_names_its_rule_and_changes_nothing(hub):
    client, headers, _, store = hub
    mismatched = client.put(
        "/v1/agents/vega/skills/resumen-semanal",
        headers=headers,
        json={"content": skill_md("otra")},
    )
    assert refusal(mismatched, 422, "skill_invalid") == {
        "name": "resumen-semanal",
        "rule": "name_mismatch",
    }
    bad_name = client.put(
        "/v1/agents/vega/skills/Resumen_Semanal",
        headers=headers,
        json={"content": skill_md("Resumen_Semanal")},
    )
    assert refusal(bad_name, 422, "skill_invalid") == {
        "name": "Resumen_Semanal",
        "rule": "name_characters",
    }
    no_description = client.put(
        "/v1/agents/vega/skills/sin-descripcion",
        headers=headers,
        json={"content": "---\nname: sin-descripcion\n---\ncuerpo"},
    )
    assert refusal(no_description, 422, "skill_invalid")["rule"] == "description_missing"
    assert AgentConfigStore(store).skills("vega") == []


def test_a_skill_name_with_a_slash_is_refused_with_its_rule(hub):
    """The Mac sends the name the file declares as one encoded segment
    (`mis%2Fskill`); the hub reads it as a name, not as two path segments, and
    refuses it with the rule it breaks rather than an uncoded 404, which the
    Mac takes for a hub without agent configuration."""
    client, headers, _, store = hub
    response = client.put(
        "/v1/agents/vega/skills/mis%2Fskill",
        headers=headers,
        json={"content": skill_md("mis/skill")},
    )
    assert refusal(response, 422, "skill_invalid") == {
        "name": "mis/skill",
        "rule": "name_characters",
    }
    assert AgentConfigStore(store).skills("vega") == []


def test_a_skill_over_its_ceiling_is_refused_with_its_bytes(hub):
    client, headers, _, _ = hub
    content = skill_md("larga", body="x" * MAX_SKILL_BYTES)
    numbers = refusal(
        client.put("/v1/agents/lyra/skills/larga", headers=headers, json={"content": content}),
        413,
        "skill_too_large",
    )
    assert numbers == {
        "name": "larga",
        "bytes": len(content.encode()),
        "max_bytes": MAX_SKILL_BYTES,
    }


def test_one_skill_past_the_ceiling_is_refused_with_the_count(hub):
    client, headers, _, store = hub
    configs = AgentConfigStore(store)
    for index in range(MAX_SKILLS):
        configs.put_skill("lyra", parse(f"s{index}", skill_md(f"s{index}")))
    numbers = refusal(
        client.put(
            "/v1/agents/lyra/skills/una-mas", headers=headers, json={"content": skill_md("una-mas")}
        ),
        409,
        "too_many_skills",
    )
    assert numbers == {"count": MAX_SKILLS, "max": MAX_SKILLS}
    # Replacing one at the ceiling is not one more.
    replaced = client.put(
        "/v1/agents/lyra/skills/s0",
        headers=headers,
        json={"content": skill_md("s0", description="Nueva.")},
    )
    assert replaced.status_code == 200, replaced.text


@pytest.mark.parametrize("agent_id", ["atlas", "lyra", "vega"])
def test_access_groups_are_kept_per_agent_for_every_kind(hub, agent_id):
    client, headers, _, store = hub
    groups = {**EVERY_GROUP, "screen": False, "files": False}
    written = client.put(f"/v1/agents/{agent_id}/access", headers=headers, json={"groups": groups})
    assert written.status_code == 200, written.text
    assert written.json()["access"] == {"groups": groups, "editable": True}
    assert AgentConfigStore(store).access(agent_id) == groups
    other = next(other for other in ("lyra", "vega") if other != agent_id)
    assert AgentConfigStore(store).access(other) == EVERY_GROUP
    # The original five remain required booleans; a partial update is refused.
    partial = client.put(
        f"/v1/agents/{agent_id}/access", headers=headers, json={"groups": {"screen": True}}
    )
    assert partial.status_code == 400
    assert partial.json()["code"] == "invalid_agent_config"
    assert AgentConfigStore(store).access(agent_id) == groups


# A Hermes agent, through Hermes's panel ----------------------------------------


def test_a_hermes_agent_reads_and_writes_its_soul_through_the_panel(hub):
    client, headers, panel, store = hub
    panel.soul = "Eres Atlas."
    config = client.get("/v1/agents/atlas/config", headers=headers)
    assert config.status_code == 200, config.text
    assert config.json()["personality"]["text"] == "Eres Atlas."
    assert config.json()["kind"] == "hermes"
    written = client.put(
        "/v1/agents/atlas/personality", headers=headers, json={"text": "Eres Atlas, sereno."}
    )
    assert written.status_code == 200, written.text
    assert panel.soul == "Eres Atlas, sereno."
    assert ("PUT", "/api/profiles/{name}/soul", {"content": "Eres Atlas, sereno."}) in panel.calls
    # Never in the hub's own database.
    assert AgentConfigStore(store).personality("atlas") == ""


def test_a_hermes_agent_creates_replaces_and_switches_skills_through_the_panel(hub):
    client, headers, panel, _ = hub
    created = client.put(
        "/v1/agents/atlas/skills/notas-diarias",
        headers=headers,
        json={"content": skill_md("notas-diarias")},
    )
    assert created.status_code == 200, created.text
    assert ("POST", "/api/skills", {"profile": PROFILE, "name": "notas-diarias", "content": skill_md("notas-diarias")}) in panel.calls  # fmt: skip
    replaced = client.put(
        "/v1/agents/atlas/skills/notas-diarias",
        headers=headers,
        json={"content": skill_md("notas-diarias", description="Otra.")},
    )
    assert replaced.status_code == 200, replaced.text
    assert panel.calls[-3][:2] == ("PUT", "/api/skills/content")
    assert panel.skills["notas-diarias"]["content"] == skill_md(
        "notas-diarias", description="Otra."
    )
    switched = client.put(
        "/v1/agents/atlas/skills/notas-diarias/enabled", headers=headers, json={"enabled": False}
    )
    assert switched.json()["skills"]["items"] == [
        {"name": "notas-diarias", "description": "Otra.", "enabled": False}
    ]
    read = client.get("/v1/agents/atlas/skills/notas-diarias", headers=headers)
    assert read.json()["content"] == skill_md("notas-diarias", description="Otra.")
    # Hermes's panel cannot delete a skill: the refusal says that deleting is
    # what it cannot do, while this agent's skills are still edited here.
    numbers = refusal(
        client.delete("/v1/agents/atlas/skills/notas-diarias", headers=headers),
        403,
        "skill_delete_unsupported",
    )
    assert numbers == {"agent_id": "atlas", "kind": "hermes"}
    # Orbit's own rules apply before anything reaches Hermes.
    calls = len(panel.calls)
    invalid = client.put(
        "/v1/agents/atlas/skills/notas-diarias", headers=headers, json={"content": "sin cabecera"}
    )
    assert refusal(invalid, 422, "skill_invalid")["rule"] == "frontmatter_missing"
    assert len(panel.calls) == calls


@pytest.mark.parametrize(
    ("method", "path", "body", "route", "status"),
    [
        ("GET", "/v1/agents/atlas/config", None, "GET /api/profiles/{name}/soul", 500),
        (
            "PUT",
            "/v1/agents/atlas/personality",
            {"text": "Eres Atlas."},
            "PUT /api/profiles/{name}/soul",
            403,
        ),
        ("GET", "/v1/agents/atlas/config", None, "GET /api/skills", 401),
        (
            "PUT",
            "/v1/agents/atlas/skills/nueva",
            {"content": skill_md("nueva")},
            "POST /api/skills",
            400,
        ),
    ],
)
def test_a_route_hermes_refuses_is_named_with_its_status(hub, method, path, body, route, status):
    client, headers, panel, _ = hub
    panel.refuse[route] = status
    numbers = refusal(
        client.request(method, path, headers=headers, json=body), 502, "assistant_admin_refused"
    )
    assert numbers == {"route": route, "status": status}


def test_an_unreachable_hermes_is_named_apart_from_a_refusal(environment, tmp_path):
    settings, store, _ = environment
    credential = tmp_path / "hermes.credential"
    credential.write_text(TOKEN + "\n")
    os.chmod(credential, 0o600)
    settings = replace(
        settings,
        hermes_url=f"http://127.0.0.1:{closed_port()}",
        hermes_credential_file=credential,
        allow_insecure_localhost=True,
        agents=entries(tmp_path),
    )
    client = TestClient(
        create_app(settings, store=store, now=lambda: 1001), base_url="https://localhost"
    )
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    numbers = refusal(
        client.get("/v1/agents/atlas/config", headers=headers), 503, "assistant_admin_unreachable"
    )
    assert numbers == {"route": "GET /api/profiles/{name}/soul"}
    # The agents the hub keeps itself do not depend on Hermes.
    assert client.get("/v1/agents/lyra/config", headers=headers).status_code == 200


def test_an_older_mac_cannot_reenable_notes_by_omitting_the_new_group(hub):
    client, headers, _, store = hub
    configs = AgentConfigStore(store)
    configs.set_access("lyra", {**configs.access("lyra"), "notes": False})
    groups = {key: True for key in configs.access("lyra") if key != "notes"}
    response = client.put("/v1/agents/lyra/access", headers=headers, json={"groups": groups})
    assert response.status_code == 200, response.text
    assert response.json()["access"]["groups"]["notes"] is False
