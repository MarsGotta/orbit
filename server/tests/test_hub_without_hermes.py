"""A hub whose agents are not Hermes: no Hermes address, health and voice without it.

Hermes is one kind of agent among several. A configuration with no Hermes agent
needs neither `hermes_url` nor `hermes_credential_file`; one with a Hermes agent
still needs both, and is refused naming the key and the agent when one is
missing. Dictation is the hub's: an agent kind without dictation of its own is
heard by the hub's `[meeting_transcription]`. Every service here is synthetic,
on a port this process binds; names are invented.
"""

import sys
from contextlib import AsyncExitStack
from dataclasses import replace

import httpx
import pytest
from conftest import device
from fastapi.testclient import TestClient
from support_audio import wav_bytes
from support_openai_compatible import FakeProvider, Text, serve
from support_orbit_mcp import FakeHub, write_mcp_config
from support_synthetic_stt import SyntheticStt
from test_health_deps import Dependency, served
from test_openai_assistant import next_event, send, turn_frames

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.routes.health import probes_for

AUDIO = wav_bytes(b"\x10\x00" * 16000)


def toml_string(value) -> str:
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'


def write_config(path, settings, stt_endpoint, stt_model, *, agents="", top=""):
    """An `orbit.toml` with the fixture's data and key, and `agents` verbatim."""
    path.write_text(
        f"data_dir = {toml_string(settings.data_dir)}\n"
        f"key_file = {toml_string(settings.key_file)}\n"
        'allowed_hosts = ["localhost"]\n'
        "allow_insecure_localhost = true\n"
        f"{top}"
        "\n[meeting_transcription]\n"
        f"endpoint = {toml_string(stt_endpoint)}\n"
        f"model = {toml_string(stt_model)}\n"
        f"{agents}"
    )
    return path


def generic_agent(provider_url, mcp_config) -> str:
    return (
        "\n[[agents]]\n"
        'id = "lyra"\n'
        'name = "Lyra"\n'
        'kind = "openai"\n'
        f"base_url = {toml_string(provider_url + '/v1')}\n"
        'model = "modelo-sintetico"\n'
        f"orbit_mcp_config = {toml_string(mcp_config)}\n"
    )


HERMES_AGENT = (
    '\n[[agents]]\nid = "atlas"\nname = "Atlas"\nkind = "hermes"\nprofile = "atlas-profile"\n'
)


@pytest.fixture
def mcp_config(tmp_path):
    """The hub's private routes orbit-mcp signs its requests to, and its TOML."""
    hub = FakeHub()
    with serve(hub.app) as url:
        yield write_mcp_config(tmp_path, url)


# Configuration ---------------------------------------------------------------


def test_a_configuration_without_hermes_needs_no_hermes_address(environment, tmp_path):
    settings, _, _ = environment
    path = write_config(
        tmp_path / "orbit.toml",
        settings,
        "http://127.0.0.1:9",
        "modelo-de-actas",
        agents=generic_agent("https://ia.example", tmp_path / "mcp.toml"),
    )
    (tmp_path / "mcp.toml").write_text("# orbit-mcp validates its own file\n")

    loaded = Settings.load(path)

    assert loaded.hermes_url is None
    assert loaded.hermes_credential_file is None
    agents, default = loaded.agent_configurations()
    assert [(agent.id, agent.kind) for agent in agents] == [("lyra", "openai")]
    assert default == "lyra"


def test_a_hermes_agent_without_hermes_url_is_refused_naming_the_key_and_the_agent(
    environment, tmp_path
):
    settings, _, _ = environment
    credential = tmp_path / "hermes-credential"
    path = write_config(
        tmp_path / "orbit.toml",
        settings,
        "http://127.0.0.1:9",
        "modelo-de-actas",
        top=f"hermes_credential_file = {toml_string(credential)}\n",
        agents=HERMES_AGENT,
    )

    with pytest.raises(ValueError) as refused:
        Settings.load(path)

    message = str(refused.value)
    assert "hermes_url" in message
    assert '"atlas"' in message and "hermes" in message


def test_a_hermes_agent_without_its_credential_is_refused_naming_the_key_and_the_agent(
    environment, tmp_path
):
    settings, _, _ = environment
    path = write_config(
        tmp_path / "orbit.toml",
        settings,
        "http://127.0.0.1:9",
        "modelo-de-actas",
        top='hermes_url = "https://hermes.example"\n',
        agents=HERMES_AGENT,
    )

    with pytest.raises(ValueError) as refused:
        Settings.load(path)

    message = str(refused.value)
    assert "hermes_credential_file" in message
    assert '"atlas"' in message


def test_the_implicit_agent_is_hermes_and_still_needs_hermes_url(environment, tmp_path):
    settings, _, _ = environment
    credential = tmp_path / "hermes-credential"
    path = write_config(
        tmp_path / "orbit.toml",
        settings,
        "http://127.0.0.1:9",
        "modelo-de-actas",
        top=f"hermes_credential_file = {toml_string(credential)}\n",
    )

    with pytest.raises(ValueError) as refused:
        Settings.load(path)

    message = str(refused.value)
    assert "hermes_url" in message
    assert '"default"' in message and "hermes" in message


def test_todays_configuration_with_hermes_loads_as_before(environment, tmp_path):
    """Unchanged behaviour: a file written for today's hub keeps meaning the same."""
    settings, _, _ = environment
    credential = tmp_path / "hermes-credential"
    path = write_config(
        tmp_path / "orbit.toml",
        settings,
        "http://127.0.0.1:9",
        "modelo-de-actas",
        top=(
            'hermes_url = "https://hermes.example"\n'
            f"hermes_credential_file = {toml_string(credential)}\n"
            'profile = "perfil-sintetico"\n'
            'assistant_name = "Nereida"\n'
        ),
    )

    loaded = Settings.load(path)

    assert loaded.hermes_url == "https://hermes.example"
    assert loaded.hermes_credential_file == credential
    agents, default = loaded.agent_configurations()
    assert [(agent.id, agent.name, agent.kind) for agent in agents] == [
        ("default", "Nereida", "hermes")
    ]
    assert agents[0].settings.profile == "perfil-sintetico"
    assert default == "default"
    assert [probe.name for probe in probes_for(loaded)] == ["whisper", "hermes"]


# Health ----------------------------------------------------------------------


def test_health_does_not_probe_hermes_when_no_agent_is_hermes(environment, tmp_path):
    settings, _, _ = environment
    (tmp_path / "mcp.toml").write_text("# orbit-mcp validates its own file\n")
    generic = {
        "id": "lyra",
        "name": "Lyra",
        "kind": "openai",
        "base_url": "https://ia.example/v1",
        "model": "modelo-sintetico",
        "orbit_mcp_config": str(tmp_path / "mcp.toml"),
    }
    # A leftover hermes_url from an older file is not a dependency of this hub.
    without_hermes_agents = replace(settings, agents=(generic,))

    assert [probe.name for probe in probes_for(without_hermes_agents)] == ["whisper"]


async def test_a_hub_without_hermes_reports_healthy(environment, tmp_path):
    settings, store, adapter = environment
    (tmp_path / "mcp.toml").write_text("# orbit-mcp validates its own file\n")
    whisper = Dependency(200)
    async with AsyncExitStack() as stack:
        whisper_url = await served(stack, whisper)
        path = write_config(
            tmp_path / "orbit.toml",
            settings,
            whisper_url,
            "modelo-de-actas",
            agents=generic_agent("https://ia.example", tmp_path / "mcp.toml"),
        )
        loaded = Settings.load(path)
        app = create_app(loaded, store=store, assistant=adapter, speech=adapter, now=lambda: 1001)
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as http:
            response = await http.get("/health/deps")
    assert response.status_code == 200, response.text
    assert response.json() == {"ok": True}
    assert whisper.requests == 1


# Chat and dictation, end to end ----------------------------------------------


def test_a_hub_without_hermes_chats_and_dictates_with_a_generic_agent(
    environment, tmp_path, mcp_config
):
    settings, store, _ = environment
    provider = FakeProvider(Text(("Hola, ", "soy Lyra.")))
    stt = SyntheticStt("Apunta la excursión al faro")
    with serve(provider.app) as provider_url, serve(stt.app) as stt_url:
        path = write_config(
            tmp_path / "orbit.toml",
            settings,
            stt_url,
            stt.model,
            agents=generic_agent(provider_url, mcp_config),
        )
        loaded = Settings.load(path)
        paired = device(store, name="Portátil de Prueba")
        headers = {"Authorization": "Bearer " + paired["token"]}
        app = create_app(loaded, store=store, now=lambda: 1001)
        with (
            TestClient(app, base_url="https://localhost") as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            ready = next_event(ws)
            assert ready["event"] == "ready"
            assert [agent["id"] for agent in ready["data"]["agents"]] == ["lyra"]
            turn = send(ws, "¿Quién eres?")
            frames = turn_frames(ws, turn)
            heard = http.post("/v1/audio/transcribe", headers=headers, content=AUDIO)

    assert frames[-1]["event"] == "chat.completed", frames[-1]
    assert len(provider.requests) == 1
    assert "¿Quién eres?" in str(provider.requests[0]["messages"])
    assert heard.status_code == 200, heard.text
    assert heard.json() == {"text": "Apunta la excursión al faro"}
    assert len(stt.audio) == 1


def test_dictation_does_not_need_the_default_agent_to_be_hermes(environment, tmp_path):
    """A Claude Code default agent has no dictation of its own: the hub's
    `[meeting_transcription]` hears it, without any choice in Settings."""
    settings, store, _ = environment
    (tmp_path / "mcp.toml").write_text("# orbit-mcp validates its own file\n")
    claude = {
        "id": "vega",
        "name": "Vega",
        "kind": "claude-code",
        # Executable and never run: no session is opened here.
        "command": [sys.executable],
        "orbit_mcp_config": str(tmp_path / "mcp.toml"),
    }
    stt = SyntheticStt("Llama a la tintorería")
    with serve(stt.app) as stt_url:
        configured = replace(
            settings, agents=(claude,), meeting_transcription=stt.configuration(stt_url)
        )
        paired = device(store, name="Portátil de Prueba")
        headers = {"Authorization": "Bearer " + paired["token"]}
        with TestClient(
            create_app(configured, store=store, now=lambda: 1001), base_url="https://localhost"
        ) as http:
            heard = http.post("/v1/audio/transcribe", headers=headers, content=AUDIO)

    assert heard.status_code == 200, heard.text
    assert heard.json() == {"text": "Llama a la tintorería"}
    assert len(stt.audio) == 1
