"""How each kind of agent receives the personality and skills Orbit keeps for it.

An `acp` agent reads them from its own folder, written before a session
opens; an `openai` agent is told them at every turn, and loads a skill's
instructions with Orbit's `orbit_skill` tool when its model asks. The ACP
agent is `support_acp_agent.py` and the model `support_openai_compatible`'s
fake provider; no Claude Code, Codex or real provider runs.
"""

import asyncio
import json
import os
import stat
import sys
from pathlib import Path

import pytest
from conftest import configured
from support_openai_compatible import FakeProvider, Text, ToolCall, serve
from support_orbit_mcp import FakeHub, write_mcp_config

from orbit_hub.assistant import acp, openai
from orbit_hub.assistant.agent_config import AgentConfigStore, AgentInstructions
from orbit_hub.assistant.backend import MessageComplete, TextBlock, TurnFailed
from orbit_hub.assistant.skills import Skill, parse
from orbit_hub.protocol import HubError

AGENT = Path(__file__).with_name("support_acp_agent.py")
WAIT_SECONDS = 60


def skill_md(name, description="Resume la semana en cinco puntos.", body="1. Lee.\n2. Resume.\n"):
    return f"---\nname: {name}\ndescription: {description}\n---\n{body}"


SUMMARY = parse("resumen-semanal", skill_md("resumen-semanal"))
TRIP = parse("viaje", skill_md("viaje", description="Prepara un viaje.", body="Mira el tren.\n"))


def mode(path):
    return stat.S_IMODE(path.stat().st_mode)


# ACP: the agent's folder -----------------------------------------------------


@pytest.mark.parametrize(
    ("agent", "instructions_file", "skills_directory"),
    [("claude-code", "CLAUDE.md", ".claude/skills"), ("codex", "AGENTS.md", ".codex/skills")],
)
def test_the_folder_gets_the_personality_and_each_enabled_skill(
    tmp_path, agent, instructions_file, skills_directory
):
    folder = tmp_path / "agente"
    folder.mkdir(mode=0o700)
    acp.write_instructions(folder, agent, AgentInstructions("Eres Vega.", (SUMMARY, TRIP)))
    assert (folder / instructions_file).read_text() == "Eres Vega."
    # Readable by an agent run as its own user; the folder itself is private.
    assert mode(folder / instructions_file) == 0o644
    skills = folder / skills_directory
    assert sorted(path.name for path in skills.iterdir()) == ["resumen-semanal", "viaje"]
    assert (skills / "viaje" / "SKILL.md").read_text() == TRIP.content
    assert mode(skills) == mode(skills / "viaje") == 0o755
    assert mode(skills / "viaje" / "SKILL.md") == 0o644
    # Nothing half-written is left behind.
    assert not [path for path in folder.rglob("*.tmp")]

    # A skill no longer enabled and a personality now empty leave the folder.
    acp.write_instructions(folder, agent, AgentInstructions("", (TRIP,)))
    assert not (folder / instructions_file).exists()
    assert [path.name for path in skills.iterdir()] == ["viaje"]


def test_a_skill_name_that_escapes_the_folder_is_refused_and_writes_nothing(tmp_path):
    folder = tmp_path / "agente"
    folder.mkdir(mode=0o700)
    escaping = Skill("../../fuera", "Sale de la carpeta.", skill_md("fuera"))
    with pytest.raises(HubError) as raised:
        acp.write_instructions(folder, "codex", AgentInstructions("", (escaping,)))
    assert raised.value.code == "agent_folder_invalid"
    assert raised.value.numbers == {"name": "../../fuera"}
    assert not (tmp_path / "fuera").exists()
    assert not (folder / "fuera").exists()


def test_a_link_where_a_directory_should_be_is_refused_rather_than_followed(tmp_path):
    folder, outside = tmp_path / "agente", tmp_path / "ajena"
    folder.mkdir(mode=0o700)
    outside.mkdir()
    (folder / ".claude").symlink_to(outside)
    with pytest.raises(HubError) as raised:
        acp.write_instructions(folder, "claude-code", AgentInstructions("", (SUMMARY,)))
    assert raised.value.code == "agent_folder_invalid"
    assert list(outside.iterdir()) == []


async def test_a_session_opens_on_a_folder_already_written(environment, tmp_path):
    settings, store, _ = environment
    configs = AgentConfigStore(store)
    configs.set_personality("vega", "Eres Vega, breve.")
    configs.put_skill("vega", SUMMARY)
    configs.put_skill("vega", TRIP)
    configs.set_skill_enabled("vega", "viaje", False)
    record = tmp_path / "record.jsonl"
    agent = configured(
        {
            "id": "vega",
            "name": "Vega",
            "kind": "acp",
            "agent": "codex",
            "command": [sys.executable, str(AGENT), "--record", str(record)],
            "orbit_mcp_config": str(tmp_path / "mcp.toml"),
        }
    )
    backend = acp.build_backend(settings, agent, lambda: configs.instructions("vega"))

    async def ignore(_event):
        return None

    connection = backend.connection("equipo-1", "Portátil de Prueba", ignore)
    await connection.open()
    try:
        await connection.create_session()
    finally:
        await connection.close()
    [initialize] = [
        entry
        for entry in map(json.loads, record.read_text().splitlines())
        if entry["kind"] == "initialize"
    ]
    # What the agent found in its folder when it started: only what is enabled.
    assert initialize["instructions"] == {
        "AGENTS.md": "Eres Vega, breve.",
        os.path.join(".codex", "skills", "resumen-semanal", "SKILL.md"): SUMMARY.content,
    }


# openai: told at every turn, a skill loaded on request ---------------------------


async def test_an_openai_agent_is_told_its_personality_and_loads_a_skill_when_asked(
    environment, tmp_path
):
    settings, store, _ = environment
    configs = AgentConfigStore(store)
    configs.set_personality("lyra", "Eres Lyra. Respondes en tres frases.")
    configs.put_skill("lyra", SUMMARY)
    configs.put_skill("lyra", TRIP)
    configs.set_skill_enabled("lyra", "viaje", False)
    provider = FakeProvider(
        ToolCall(openai.SKILL_TOOL, {"name": "resumen-semanal"}), Text(("Hecho.",))
    )
    with serve(FakeHub().app) as hub_url, serve(provider.app) as provider_url:
        agent = configured(
            {
                "id": "lyra",
                "name": "Lyra",
                "kind": "openai",
                "base_url": provider_url + "/v1",
                "model": "modelo-sintetico",
                "orbit_mcp_config": str(write_mcp_config(tmp_path, hub_url)),
            }
        )
        backend = openai.build_backend(settings, agent, lambda: configs.instructions("lyra"))
        events = []

        async def on_event(event):
            events.append(event)

        connection = backend.connection("equipo-1", "Portátil de Prueba", on_event)
        await connection.open()
        try:
            opened = await connection.create_session()
            await connection.prompt(opened.session_id, (TextBlock("Resume mi semana"),))
            async with asyncio.timeout(WAIT_SECONDS):
                while not events or not isinstance(events[-1], MessageComplete | TurnFailed):
                    await asyncio.sleep(0.01)
        finally:
            await connection.close()

    assert events[-1] == MessageComplete(events[-1].session_id, "Hecho.", "completed")
    first, second = provider.requests
    system = "\n".join(
        message["content"] for message in first["messages"] if message["role"] == "system"
    )
    assert "Eres Lyra. Respondes en tres frases." in system
    assert "- resumen-semanal: Resume la semana en cinco puntos." in system
    # A disabled skill is not offered.
    assert "viaje" not in system
    assert openai.SKILL_TOOL in {tool["function"]["name"] for tool in first["tools"]}
    tool_result = second["messages"][-1]
    assert tool_result["role"] == "tool"
    assert tool_result["content"] == SUMMARY.body


async def test_the_skill_tool_refuses_a_skill_that_is_not_enabled():
    from pydantic_ai import ToolFailed

    toolset = openai.skill_toolset((SUMMARY,))
    [tool] = toolset.tools.values()
    with pytest.raises(ToolFailed, match="viaje.*resumen-semanal"):
        tool.function(name="viaje")
    assert tool.function(name="resumen-semanal") == SUMMARY.body


def test_without_personality_or_skills_the_model_is_told_nothing_more():
    assert openai.turn_instructions(AgentInstructions("", ())) is None
    assert openai.turn_instructions(AgentInstructions("   ", ())) is None
