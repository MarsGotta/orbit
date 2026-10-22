"""How each kind of agent receives the personality and skills Orbit keeps for it.

Every kind is told the same text (`agent_config.instructions_text`): its
personality, then each enabled skill's name and description, and loads a
skill's instructions with orbit-mcp's `orbit_skill` when it needs them. An
`openai` agent is told at every turn; Claude Code in each session's
`_meta.systemPrompt.append`, because its lock-down (`settingSources: []`,
`tools: []`) reads no `CLAUDE.md` and has no tool to open a skill file; Codex
in the `AGENTS.md` of its folder, which it reads from its `cwd`. The ACP
agent is `support_acp_agent.py` and the model `support_openai_compatible`'s
fake provider; no Claude Code, Codex or real provider runs.
"""

import asyncio
import json
import stat
import sys
from pathlib import Path

import pytest
from conftest import configured
from support_openai_compatible import FakeProvider, Text, ToolCall, serve
from support_orbit_mcp import FakeHub, write_mcp_config

from orbit_hub.assistant import acp, openai
from orbit_hub.assistant.agent_config import (
    MAX_INSTRUCTIONS_TEXT_BYTES,
    MAX_PERSONALITY_BYTES,
    SKILL_TOOL,
    AgentConfigStore,
    AgentInstructions,
    instructions_text,
)
from orbit_hub.assistant.backend import MessageComplete, TextBlock, TurnFailed
from orbit_hub.assistant.skills import (
    MAX_DESCRIPTION_CHARS,
    MAX_NAME_CHARS,
    MAX_SKILLS,
    Skill,
    parse,
)
from orbit_hub.protocol import HubError

AGENT = Path(__file__).with_name("support_acp_agent.py")
WAIT_SECONDS = 60
CLAUDE_LOCK_DOWN = {"claudeCode": {"options": {"tools": [], "strictMcpConfig": True, "settingSources": []}}}  # fmt: skip


def skill_md(name, description="Resume la semana en cinco puntos.", body="1. Lee.\n2. Resume.\n"):
    return f"---\nname: {name}\ndescription: {description}\n---\n{body}"


SUMMARY = parse("resumen-semanal", skill_md("resumen-semanal"))
TRIP = parse("viaje", skill_md("viaje", description="Prepara un viaje.", body="Mira el tren.\n"))


def mode(path):
    return stat.S_IMODE(path.stat().st_mode)


def records(path, kind):
    seen = map(json.loads, path.read_text().splitlines())
    return [entry for entry in seen if entry["kind"] == kind]


async def open_one_session(backend):
    async def ignore(_event):
        return None

    connection = backend.connection("equipo-1", "Portátil de Prueba", ignore)
    await connection.open()
    try:
        await connection.create_session()
    finally:
        await connection.close()


def acp_agent(tmp_path, agent_id, agent, record):
    return configured(
        {
            "id": agent_id,
            "name": agent_id.capitalize(),
            "kind": "acp",
            "agent": agent,
            "command": [sys.executable, str(AGENT), "--record", str(record)],
            "orbit_mcp_config": str(tmp_path / "mcp.toml"),
        }
    )


# The text every kind is told -------------------------------------------------


def test_the_text_is_the_personality_then_each_skill_and_how_to_load_it():
    text = instructions_text(AgentInstructions("Eres Vega.", (SUMMARY, TRIP)))
    assert text is not None
    personality, skills = text.split("\n\n", 1)
    assert personality == "Eres Vega."
    assert SKILL_TOOL in skills
    assert skills.endswith(
        "\n- resumen-semanal: Resume la semana en cinco puntos.\n- viaje: Prepara un viaje."
    )
    # Neither part is invented when it is missing.
    assert instructions_text(AgentInstructions("", ())) is None
    assert instructions_text(AgentInstructions("   ", ())) is None
    assert instructions_text(AgentInstructions("Eres Vega.", ())) == "Eres Vega."
    assert instructions_text(AgentInstructions("", (TRIP,))).startswith("Your skills")


def test_the_longest_text_fits_its_bound():
    """A personality at its bound and every skill at the standard's maxima,
    four bytes a character: what Codex is told to read whole."""
    wide = "𝄞" * MAX_DESCRIPTION_CHARS
    skills = tuple(Skill(f"{index:0{MAX_NAME_CHARS}d}", wide, "") for index in range(MAX_SKILLS))
    longest = instructions_text(AgentInstructions("𝄞" * (MAX_PERSONALITY_BYTES // 4), skills))
    assert len(longest.encode("utf-8")) <= MAX_INSTRUCTIONS_TEXT_BYTES


# Codex: AGENTS.md in its folder ---------------------------------------------


def test_codex_gets_its_agents_md_and_no_skill_folder(tmp_path):
    folder = tmp_path / "agente"
    folder.mkdir(mode=0o700)
    current = AgentInstructions("Eres Vega.", (SUMMARY, TRIP))
    acp.write_instructions(folder, current)
    assert (folder / "AGENTS.md").read_text() == instructions_text(current)
    # Readable by an agent run as its own user; the folder itself is private.
    assert mode(folder / "AGENTS.md") == 0o644
    # Codex lists a folder's skills by path and opens them with its shell,
    # which the lock-down removes: the skills reach it by `orbit_skill`.
    assert sorted(path.name for path in folder.iterdir()) == ["AGENTS.md"]

    # Nothing to tell leaves no file behind.
    acp.write_instructions(folder, AgentInstructions("", ()))
    assert list(folder.iterdir()) == []


def test_a_link_where_agents_md_should_be_is_refused_rather_than_followed(tmp_path):
    folder, outside = tmp_path / "agente", tmp_path / "ajeno.md"
    folder.mkdir(mode=0o700)
    outside.write_text("de otro")
    (folder / "AGENTS.md").symlink_to(outside)
    with pytest.raises(HubError) as raised:
        acp.write_instructions(folder, AgentInstructions("Eres Vega.", ()))
    assert raised.value.code == "agent_folder_invalid"
    assert outside.read_text() == "de otro"


async def test_a_codex_session_opens_on_its_agents_md_already_written(environment, tmp_path):
    settings, store, _ = environment
    configs = AgentConfigStore(store)
    configs.set_personality("vega", "Eres Vega, breve.")
    configs.put_skill("vega", SUMMARY)
    configs.put_skill("vega", TRIP)
    configs.set_skill_enabled("vega", "viaje", False)
    record = tmp_path / "record.jsonl"
    agent = acp_agent(tmp_path, "vega", "codex", record)
    backend = acp.build_backend(settings, agent, lambda: configs.instructions("vega"))
    await open_one_session(backend)

    [initialize] = records(record, "initialize")
    # What the agent found in its folder when it started: only what is enabled.
    told = instructions_text(AgentInstructions("Eres Vega, breve.", (SUMMARY,)))
    assert initialize["instructions"] == {"AGENTS.md": told}
    assert "viaje" not in told
    # Codex reads at most `project_doc_max_bytes` of it (32 KiB by default).
    codex_config = json.loads(initialize["environment"]["CODEX_CONFIG"])
    assert codex_config["project_doc_max_bytes"] == MAX_INSTRUCTIONS_TEXT_BYTES
    [created] = records(record, "session/new")
    assert created["meta"] == {}


# Claude Code: told in the session ---------------------------------------------


async def test_claude_code_is_told_in_its_session_and_nothing_is_written(environment, tmp_path):
    settings, store, _ = environment
    configs = AgentConfigStore(store)
    configs.set_personality("orion", "Eres Orión. Contestas en dos frases.")
    configs.put_skill("orion", SUMMARY)
    configs.put_skill("orion", TRIP)
    configs.set_skill_enabled("orion", "viaje", False)
    record = tmp_path / "record.jsonl"
    agent = acp_agent(tmp_path, "orion", "claude-code", record)
    backend = acp.build_backend(settings, agent, lambda: configs.instructions("orion"))
    await open_one_session(backend)

    told = instructions_text(AgentInstructions("Eres Orión. Contestas en dos frases.", (SUMMARY,)))
    [created] = records(record, "session/new")
    # claude-agent-acp 0.94.0 turns `_meta.systemPrompt` into the Agent SDK's
    # `{type: "preset", preset: "claude_code", append}`; the lock-down stays.
    assert created["meta"] == {**CLAUDE_LOCK_DOWN, "systemPrompt": {"append": told}}
    # No CLAUDE.md and no .claude/skills: with `settingSources: []` and
    # `tools: []` Claude Code would read neither.
    [initialize] = records(record, "initialize")
    assert initialize["instructions"] == {}


# openai: told at every turn, a skill loaded from orbit-mcp -----------------------


async def test_an_openai_agent_is_told_its_personality_and_loads_a_skill_when_asked(
    environment, tmp_path
):
    settings, store, _ = environment
    configs = AgentConfigStore(store)
    configs.set_personality("lyra", "Eres Lyra. Respondes en tres frases.")
    configs.put_skill("lyra", SUMMARY)
    configs.put_skill("lyra", TRIP)
    configs.set_skill_enabled("lyra", "viaje", False)
    provider = FakeProvider(ToolCall(SKILL_TOOL, {"name": "resumen-semanal"}), Text(("Hecho.",)))
    # The fake hub hands orbit-mcp the skills the real one keeps enabled.
    hub = FakeHub(skills={"lyra": {"resumen-semanal": SUMMARY.body}})
    with serve(hub.app) as hub_url, serve(provider.app) as provider_url:
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
    told = instructions_text(AgentInstructions("Eres Lyra. Respondes en tres frases.", (SUMMARY,)))
    assert told in system
    # A disabled skill is not offered.
    assert "viaje" not in system
    # One `orbit_skill`, orbit-mcp's: the hub no longer adds its own.
    names = [tool["function"]["name"] for tool in first["tools"]]
    assert names.count(SKILL_TOOL) == 1
    tool_result = second["messages"][-1]
    assert tool_result["role"] == "tool"
    assert tool_result["content"] == SUMMARY.body
    assert hub.skill_requests == [("lyra", "resumen-semanal")]
