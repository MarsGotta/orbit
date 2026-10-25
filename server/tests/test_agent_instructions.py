"""The shared personality/skills text and the generic API backend.

Official vendor delivery is covered through the public hub in
`test_official_assistants`. No real provider runs.
"""

import asyncio

from conftest import configured
from support_openai_compatible import FakeProvider, Text, ToolCall, serve
from support_orbit_mcp import FakeHub, write_mcp_config

from orbit_hub.assistant import openai
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

WAIT_SECONDS = 60


def skill_md(name, description="Resume la semana en cinco puntos.", body="1. Lee.\n2. Resume.\n"):
    return f"---\nname: {name}\ndescription: {description}\n---\n{body}"


SUMMARY = parse("resumen-semanal", skill_md("resumen-semanal"))
TRIP = parse("viaje", skill_md("viaje", description="Prepara un viaje.", body="Mira el tren.\n"))


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
