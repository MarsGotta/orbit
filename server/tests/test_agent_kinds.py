"""The three kinds of agent side by side: Hermes, the generic `openai` and official vendor.

What each kind's own module does is tested beside it (`test_hermes`,
`test_openai_assistant`, `test_official_assistants`); here, what the hub does with
several of them at once: one `orbit.toml` with every kind, the `ready` list,
whose voice the speech routes use, a conversation resumed with or without
the hub's stored history, and two agents asking for approval with the same
request id. Names, ids, paths and providers are invented; nothing is started
and no network is used.
"""

import asyncio
import json
import subprocess
import sys
from dataclasses import replace
from pathlib import Path
from uuid import uuid4

import httpx
import pytest
from conftest import device
from fastapi.testclient import TestClient
from support_audio import wav_bytes

from orbit_hub.app import create_app
from orbit_hub.assistant import openai
from orbit_hub.assistant.agents import KINDS, Agent, Agents
from orbit_hub.assistant.backend import (
    Capabilities,
    ConnectionLost,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    PermissionRequest,
    PermissionRequested,
    RequestExpired,
    ResumedSession,
    StoredConversation,
    TurnError,
)
from orbit_hub.assistant.hermes import HermesAgent, HermesSpeech
from orbit_hub.assistant.tool_settings import ToolSettings
from orbit_hub.protocol import ApprovalParams, HubError, SendParams
from orbit_hub.runtime import DeviceRuntime

ORIGINAL_ASYNC_CLIENT = httpx.AsyncClient


def every_kind(tmp_path, *, speech=False):
    """One entry of each kind; the files they name exist, as at startup."""
    mcp_config = tmp_path / "mcp.toml"
    mcp_config.write_text("# orbit-mcp validates its own file\n")
    generic = {
        "id": "lyra",
        "name": "Lyra",
        "kind": "openai",
        "base_url": "https://ia.example/v1",
        "model": "modelo-sintetico",
        "orbit_mcp_config": str(mcp_config),
    }
    if speech:
        generic["speech"] = {
            "base_url": "https://voz.example/v1",
            "model": "voz-sintetica",
            "voice": "alloy",
        }
    return (
        {"id": "atlas", "name": "Atlas", "kind": "hermes", "profile": "atlas-profile"},
        generic,
        {
            "id": "vega",
            "name": "Vega",
            "kind": "claude-code",
            # Executable and never run: no session is opened here.
            "command": [sys.executable],
            "orbit_mcp_config": str(mcp_config),
        },
    )


def hub(settings, store):
    """The hub with each agent on its own kind's backend and voice."""
    return TestClient(
        create_app(settings, store=store, now=lambda: 1001), base_url="https://localhost"
    )


# Configuration ---------------------------------------------------------------


def test_every_kind_is_registered():
    assert sorted(KINDS) == ["claude-code", "codex", "hermes", "openai"]


def test_one_file_configures_every_kind_without_reading_any_file(environment, tmp_path):
    """Each kind's `parse` gets the entry without `id`, `name` and `kind`, and
    the configuration loads without the files it names: they are the backend's
    to check when the hub starts."""
    entries = every_kind(tmp_path)
    missing = str(tmp_path / "missing.toml")
    entries = (
        entries[0],
        {**entries[1], "orbit_mcp_config": missing, "api_key_file": str(tmp_path / "no.key")},
        {**entries[2], "orbit_mcp_config": missing, "command": ["/no/such/adapter"]},
    )
    settings = replace(environment[0], agents=entries, default_agent="lyra")
    configured, default = settings.agent_configurations()
    assert default == "lyra"
    assert [(agent.id, agent.name, agent.kind) for agent in configured] == [
        ("atlas", "Atlas", "hermes"),
        ("lyra", "Lyra", "openai"),
        ("vega", "Vega", "claude-code"),
    ]
    hermes, generic, coding = (agent.settings for agent in configured)
    assert hermes == HermesAgent("atlas-profile")
    assert isinstance(generic, openai.OpenAIAgentSettings)
    assert (generic.base_url, generic.orbit_mcp_config) == ("https://ia.example/v1", tmp_path / "missing.toml")  # fmt: skip
    assert coding == ToolSettings(
        agent="claude-code",
        command=("/no/such/adapter",),
        orbit_mcp_config=tmp_path / "missing.toml",
    )


@pytest.mark.parametrize(
    ("entry", "refusal"),
    [
        (1, {"orbit_mcp_config": "/no/such/mcp.toml"}),
        (2, {"command": ["/no/such/adapter"]}),
    ],
)
def test_a_file_an_agent_needs_is_refused_when_the_hub_starts(
    environment, tmp_path, entry, refusal
):
    settings, store, _ = environment
    entries = list(every_kind(tmp_path))
    entries[entry] = {**entries[entry], **refusal}
    configured = replace(settings, profile="deimos", agents=tuple(entries))
    with pytest.raises(ValueError) as refused:
        create_app(configured, store=store)
    agent_id, [(key, value)] = entries[entry]["id"], refusal.items()
    path = value if isinstance(value, str) else value[0]
    assert repr(agent_id) in str(refused.value) or f'"{agent_id}"' in str(refused.value)
    assert key in str(refused.value) and path in str(refused.value)


@pytest.mark.parametrize(
    ("change", "refusal"),
    [
        ({"kind": "openai", "profile": "x"}, r'entry 2 \(id "lyra"\): unknown keys \(1\): profile'),
        ({"kind": "codex"}, r'entry 2 \(id "lyra"\): unknown keys for codex'),
        (
            {"provider": "gemini"},
            r'entry 2 \(id "lyra"\): provider: must be "openai" or "anthropic"',
        ),
    ],
)
def test_a_kind_refuses_the_keys_of_another_naming_the_entry(
    environment, tmp_path, change, refusal
):
    entries = list(every_kind(tmp_path))
    entries[1] = {**entries[1], **change}
    with pytest.raises(ValueError, match=refusal):
        replace(environment[0], agents=tuple(entries))


# ready -----------------------------------------------------------------------


@pytest.mark.parametrize("speech", [False, True])
def test_ready_lists_every_kind_with_what_it_accepts(environment, tmp_path, speech):
    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path, speech=speech))
    paired = device(store)
    with (
        hub(configured, store) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws,
    ):
        ready = ws.receive_json()["data"]
    assert ready["default_agent_id"] == "atlas"
    assert ready["assistant_name"] == "Atlas"
    # Each with its profile (none chosen yet: no description and the colours
    # in turn), and an official vendor agent with the account Orbit can sign in.
    profile = {"description": "", "removable": False}
    assert ready["agents"] == [
        {
            "id": "atlas",
            "name": "Atlas",
            "kind": "hermes",
            "capabilities": ["images", "voice", "resume"],
            "service": "hermes",
            "color": "c1",
            **profile,
        },
        {
            "id": "lyra",
            "name": "Lyra",
            "kind": "openai",
            # Voice is read-aloud, which only `[agents.speech]` gives it; images
            # only `images = true`, false by default.
            "capabilities": ["voice", "resume"] if speech else ["resume"],
            "service": "openai",
            "color": "c2",
            **profile,
        },
        # An official vendor agent that declares no image input is named the files instead.
        {
            "id": "vega",
            "name": "Vega",
            "kind": "claude-code",
            "capabilities": ["images", "resume"],
            "service": "claude-code",
            "color": "c3",
            "account": "unknown",
            **profile,
        },
    ]


# Speech ----------------------------------------------------------------------


def test_each_agent_has_its_own_kind_of_voice(environment, tmp_path):
    settings, _, _ = environment
    agents = Agents.from_settings(replace(settings, agents=every_kind(tmp_path)))
    atlas, lyra, vega = (agents.by_id[agent] for agent in ("atlas", "lyra", "vega"))
    assert isinstance(atlas.speech, HermesSpeech) and atlas.speech.profile == "atlas-profile"
    assert isinstance(lyra.speech, openai.OpenAISpeech) and lyra.speech.agent.id == "lyra"
    assert vega.speech is None


def test_the_implicit_agent_keeps_the_voice_of_its_profile(environment):
    settings, _, _ = environment
    agents = Agents.from_settings(replace(settings, profile="atlas-profile"))
    assert isinstance(agents.default.speech, HermesSpeech)
    assert agents.default.speech.profile == "atlas-profile"


def atlas_voice(tmp_path, monkeypatch) -> tuple[list, object]:
    """Atlas's Hermes voice on a synthetic transport: the requests it gets,
    and the credential file a Hermes agent needs."""
    requests = []

    def reply(request):
        requests.append(request)
        return httpx.Response(
            200,
            json={
                "ok": True,
                "data_url": "data:audio/mpeg;base64,c2ludGV0aWNv",
                "mime_type": "audio/mpeg",
                "provider": "voz-de-atlas",
            },
        )

    monkeypatch.setattr(
        "orbit_hub.assistant.hermes_speech.httpx.AsyncClient",
        lambda **kwargs: ORIGINAL_ASYNC_CLIENT(transport=httpx.MockTransport(reply), **kwargs),
    )
    token = tmp_path / "token"
    token.write_text("synthetic-token-only")
    token.chmod(0o600)
    return requests, token


def test_read_aloud_without_an_agent_uses_the_default_agent_voice(
    environment, tmp_path, monkeypatch
):
    """A request that names no agent (a 0.9.0 Mac's) is the default agent's:
    here Hermes's, with its profile, beside agents that have none of theirs."""
    settings, store, _ = environment
    requests, token = atlas_voice(tmp_path, monkeypatch)
    configured = replace(
        settings,
        agents=tuple(reversed(every_kind(tmp_path))),
        default_agent="atlas",
        hermes_credential_file=token,
    )
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        response = http.post("/v1/audio/speech", headers=headers, json={"text": "frase"})
    assert response.status_code == 200, response.text
    assert response.headers["X-Orbit-Speech-Provider"] == "voz-de-atlas"
    assert [request.url.path for request in requests] == ["/api/audio/speak"]
    assert b'"profile":"atlas-profile"' in requests[0].content.replace(b" ", b"")


def test_read_aloud_uses_the_voice_of_the_agent_it_names(environment, tmp_path, monkeypatch):
    """The default agent has no voice; the conversation's agent has one, and
    its answer is read with it."""
    settings, store, _ = environment
    requests, token = atlas_voice(tmp_path, monkeypatch)
    configured = replace(
        settings, agents=every_kind(tmp_path), default_agent="vega", hermes_credential_file=token
    )
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        response = http.post(
            "/v1/audio/speech", headers=headers, json={"text": "frase", "agent_id": "atlas"}
        )
    assert response.status_code == 200, response.text
    assert response.headers["X-Orbit-Speech-Provider"] == "voz-de-atlas"
    assert b'"profile":"atlas-profile"' in requests[0].content.replace(b" ", b"")


def test_read_aloud_for_an_unknown_agent_is_refused_with_its_id(environment, tmp_path):
    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path), default_agent="atlas")
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        response = http.post(
            "/v1/audio/speech", headers=headers, json={"text": "frase", "agent_id": "orion"}
        )
    assert response.status_code == 404
    body = response.json()
    assert body["code"] == "agent_not_found"
    assert body["refusal"]["agent_id"] == "orion"
    assert body["refusal"]["agent_ids"] == "atlas,lyra,vega"


def test_read_aloud_for_an_agent_without_a_voice_is_in_the_default_voice(
    environment, tmp_path, monkeypatch
):
    """The default agent has a voice; the one named has none of its own, and
    its answer is read with the default agent's."""
    settings, store, _ = environment
    requests, token = atlas_voice(tmp_path, monkeypatch)
    configured = replace(
        settings, agents=every_kind(tmp_path), default_agent="atlas", hermes_credential_file=token
    )
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        response = http.post(
            "/v1/audio/speech", headers=headers, json={"text": "frase", "agent_id": "vega"}
        )
    assert response.status_code == 200, response.text
    assert response.headers["X-Orbit-Speech-Provider"] == "voz-de-atlas"
    assert len(requests) == 1


def test_read_aloud_without_any_voice_names_the_agent_asked_for(environment, tmp_path):
    """Neither the agent named nor the default one has a voice."""
    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path), default_agent="lyra")
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        response = http.post(
            "/v1/audio/speech", headers=headers, json={"text": "frase", "agent_id": "vega"}
        )
    assert response.status_code == 503
    body = response.json()
    assert body["code"] == "speech_not_configured"
    assert body["refusal"]["agent_id"] == "vega"
    assert "Vega" in body["message"]


@pytest.mark.parametrize("route", ["/v1/audio/speech", "/v1/audio/transcribe"])
def test_a_default_agent_without_a_voice_refuses_both_routes_naming_it(
    environment, tmp_path, route
):
    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path), default_agent="vega")
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        if route.endswith("speech"):
            response = http.post(route, headers=headers, json={"text": "frase"})
        else:
            response = http.post(route, headers=headers, content=wav_bytes(b"\x00\x00" * 160))
    assert response.status_code == 503
    body = response.json()
    assert body["code"] == "speech_not_configured"
    assert body["refusal"]["agent_id"] == "vega"
    assert "Vega" in body["message"] and "claude-code" in body["message"]


def test_a_generic_default_agent_dictates_but_without_speech_does_not_read_aloud(
    environment, tmp_path
):
    settings, store, _ = environment
    configured = replace(settings, agents=every_kind(tmp_path), default_agent="lyra")
    headers = {"Authorization": "Bearer " + device(store)["token"]}
    with hub(configured, store) as http:
        dictated = http.post(
            "/v1/audio/transcribe", headers=headers, content=wav_bytes(b"\x00\x00" * 1600)
        )
        read = http.post("/v1/audio/speech", headers=headers, json={"text": "frase"})
    # Dictation is the hub's meeting transcriber, here the suite's synthetic one.
    assert dictated.status_code == 200, dictated.text
    assert read.status_code == 503
    assert read.json()["code"] == "speech_not_configured"
    assert read.json()["refusal"]["agent_id"] == "lyra"
    assert "Lyra" in read.json()["message"] and "[agents.speech]" in read.json()["message"]


# Two agents in one runtime -----------------------------------------------------


class TwoWayConnection:
    """A connection of an invented agent: every turn asks for approval with
    the request id `"1"`, as two JSON-RPC peers both numbering from one would."""

    def __init__(self, backend, on_event):
        self.backend, self.on_event = backend, on_event
        self.epoch = str(uuid4())
        self.live_sessions = set()
        self.connected = False

    async def open(self):
        self.connected = True

    async def close(self):
        self.connected = False

    async def create_session(self):
        live = f"{self.backend.name}-live-{uuid4()}"
        self.live_sessions.add(live)
        return OpenedSession(live, f"{self.backend.name}-stored-{uuid4()}")

    async def resume_session(self, stored_id, history=None):
        self.backend.resumes.append((stored_id, history))
        live = f"{self.backend.name}-resumed-{uuid4()}"
        self.live_sessions.add(live)
        return ResumedSession(live, stored_id, None, False)

    async def prompt(self, session_id, blocks):
        if self.backend.asks:
            await self.on_event(
                PermissionRequested(
                    session_id,
                    PermissionRequest(
                        "1", f"{self.backend.name} pide permiso", ("once", "session", "deny")
                    ),
                )
            )
        else:
            await self.on_event(MessageComplete(session_id, f"Responde {self.backend.name}", "end"))

    async def cancel(self, session_id):
        pass

    async def respond_permission(self, session_id, request_id, *, choice):
        self.backend.answers.append((session_id, request_id, choice))
        return PermissionReceipt(applied=True)

    async def respond_clarification(self, session_id, request_id, answer, question_id):
        raise AssertionError("no clarification is asked here")

    async def turn_stopped(self, session_id):
        return True


class TwoWayBackend:
    def __init__(self, name, *, history=True, asks=True):
        self.name, self.asks = name, asks
        self.capabilities = Capabilities(images=False, resume=True, history=history, voice=False)
        self.connections, self.answers, self.resumes = [], [], []

    def connection(self, device, device_name, on_event):
        connection = TwoWayConnection(self, on_event)
        self.connections.append(connection)
        return connection


def two_agents(first, second):
    return Agents(
        [
            Agent("atlas", "Atlas", "invented", None, first),
            Agent("vega", "Vega", "invented", None, second),
        ],
        "atlas",
    )


def drain(queue):
    frames = []
    while not queue.empty():
        frames.append(queue.get_nowait())
    return frames


async def test_two_agents_asking_with_the_same_request_id_never_collide(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    atlas, vega = TwoWayBackend("atlas"), TwoWayBackend("vega")
    runtime = DeviceRuntime(owner, store, two_agents(atlas, vega), lambda: 1000)
    frames = runtime.subscribe()
    _, first = await runtime.handle("chat.send", SendParams(text="Hola"), str(uuid4()))
    _, second = await runtime.handle(
        "chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4())
    )
    asked = [frame["data"] for frame in drain(frames) if frame["event"] == "approval.required"]
    assert [entry["session_id"] for entry in asked] == [first["session_id"], second["session_id"]]
    # Unique on the wire for this device; the first keeps the agent's own id.
    to_atlas, to_vega = (entry["request_id"] for entry in asked)
    assert to_atlas == "1" and to_vega != "1"
    pending = [
        item["request_id"]
        for session in runtime.snapshot()["sessions"]
        for item in session["pending"]
    ]
    assert sorted(pending) == sorted([to_atlas, to_vega])

    # Each answer reaches the agent that asked, with that agent's own id and
    # the choice the Mac made among the ones it offered.
    kind, resolved = await runtime.handle(
        "approval.respond",
        ApprovalParams(
            session_id=second["session_id"], request_id=to_vega, approved=True, choice="session"
        ),
        str(uuid4()),
    )
    assert (kind, resolved["request_id"], resolved["status"]) == (
        "interaction.resolved",
        to_vega,
        "ok",
    )
    [(vega_session, vega_request, chosen)] = vega.answers
    assert (vega_request, chosen) == ("1", "session") and vega_session.startswith("vega-live")
    assert atlas.answers == []
    await runtime.handle(
        "approval.respond",
        ApprovalParams(session_id=first["session_id"], request_id=to_atlas, approved=True),
        str(uuid4()),
    )
    assert [(request, chosen) for _, request, chosen in atlas.answers] == [("1", "once")]
    assert not runtime.pending


async def test_an_expiry_retires_only_the_request_of_the_agent_that_sent_it(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    atlas, vega = TwoWayBackend("atlas"), TwoWayBackend("vega")
    runtime = DeviceRuntime(owner, store, two_agents(atlas, vega), lambda: 1000)
    frames = runtime.subscribe()
    await runtime.handle("chat.send", SendParams(text="Hola"), str(uuid4()))
    _, second = await runtime.handle(
        "chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4())
    )
    to_atlas, to_vega = (
        frame["data"]["request_id"]
        for frame in drain(frames)
        if frame["event"] == "approval.required"
    )
    [vega_live] = vega.connections[0].live_sessions
    await vega.connections[0].on_event(RequestExpired(vega_live, "1"))
    expired = [frame["data"] for frame in drain(frames) if frame["event"] == "interaction.expired"]
    assert expired == [{"session_id": second["session_id"], "request_id": to_vega}]
    assert list(runtime.pending) == [to_atlas]


async def test_a_resume_hands_the_stored_history_only_to_an_agent_that_keeps_none(environment):
    """Hermes and official vendor agents reload their own conversation; the generic one
    keeps none and continues the one the hub stores."""
    _, store, _ = environment
    owner = device(store)["device_id"]
    keeps, forgets = (
        TwoWayBackend("atlas", asks=False),
        TwoWayBackend("vega", history=False, asks=False),
    )
    agents = two_agents(keeps, forgets)
    runtime = DeviceRuntime(owner, store, agents, lambda: 1000)
    _, first = await runtime.handle("chat.send", SendParams(text="Primero"), str(uuid4()))
    _, second = await runtime.handle(
        "chat.send", SendParams(text="Primero", agent_id="vega"), str(uuid4())
    )
    await runtime.close()

    # A new runtime opens new connections: both sessions must be resumed.
    again = DeviceRuntime(owner, store, agents, lambda: 1001)
    for session in (first, second):
        await asyncio.wait_for(
            again.handle(
                "chat.send",
                SendParams(text="Segundo", session_id=session["session_id"]),
                str(uuid4()),
            ),
            5,
        )
    [(_, kept)] = keeps.resumes
    [(_, handed)] = forgets.resumes
    assert kept is None
    assert isinstance(handed, StoredConversation)
    assert [(message.role, message.text) for message in handed.messages] == [
        ("user", "Primero"),
        ("assistant", "Responde vega"),
    ]
    assert handed.total == 2


# What the runtime says names the conversation's agent ----------------------------


def error_frames(frames):
    return [frame["data"] for frame in drain(frames) if frame["event"] == "error"]


async def test_a_lost_connection_names_the_agent_of_the_conversation(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    vega = TwoWayBackend("vega")
    runtime = DeviceRuntime(owner, store, two_agents(TwoWayBackend("atlas"), vega), lambda: 1000)
    frames = runtime.subscribe()
    await runtime.handle("chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4()))
    await vega.connections[0].on_event(ConnectionLost())
    [lost] = error_frames(frames)
    assert lost["code"] == "outcome_unknown"
    assert lost["message"] == (
        "The connection was lost with Vega; check the history before continuing"
    )


async def test_a_turn_ended_without_an_answer_names_the_agent_of_the_conversation(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    vega = TwoWayBackend("vega")
    runtime = DeviceRuntime(owner, store, two_agents(TwoWayBackend("atlas"), vega), lambda: 1000)
    frames = runtime.subscribe()
    await runtime.handle("chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4()))
    [live] = vega.connections[0].live_sessions
    await vega.connections[0].on_event(TurnError(live, "terminal"))
    [failed] = error_frames(frames)
    assert failed["code"] == "upstream_turn_failed"
    assert (
        failed["message"]
        == "Vega finished the turn without a final response; it will not be repeated"
    )


async def test_an_agent_the_configuration_no_longer_has_is_called_the_assistant(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    before = DeviceRuntime(
        owner, store, two_agents(TwoWayBackend("atlas"), TwoWayBackend("vega")), lambda: 1000
    )
    _, sent = await before.handle(
        "chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4())
    )
    await before.close()
    only_atlas = Agents(
        [Agent("atlas", "Atlas", "invented", None, TwoWayBackend("atlas"))], "atlas"
    )
    after = DeviceRuntime(owner, store, only_atlas, lambda: 1001)
    frames = after.subscribe()
    turn = str(uuid4())
    after.turn_ids[sent["session_id"]] = turn
    await after.fail_turn("vega", sent["session_id"], turn)
    [failed] = error_frames(frames)
    assert failed["message"] == (
        "The assistant finished the turn without a final response; it will not be repeated"
    )


class HydratingConnection(TwoWayConnection):
    """A resume whose history is still loading until the test says otherwise."""

    async def resume_session(self, stored_id, history=None):
        resumed = await super().resume_session(stored_id, history)
        return replace(resumed, loading_history=True)


class HydratingBackend(TwoWayBackend):
    def connection(self, device, device_name, on_event):
        connection = HydratingConnection(self, on_event)
        self.connections.append(connection)
        return connection


async def test_a_resume_in_progress_names_the_agent_loading_the_history(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    agents = two_agents(TwoWayBackend("atlas"), HydratingBackend("vega", asks=False))
    before = DeviceRuntime(owner, store, agents, lambda: 1000)
    sessions = [
        (await before.handle("chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4())))[
            1
        ]["session_id"]
        for _ in range(2)
    ]
    await before.close()

    after = DeviceRuntime(owner, store, agents, lambda: 1001)
    loading = asyncio.create_task(
        after.handle("chat.send", SendParams(text="Sigo", session_id=sessions[0]), str(uuid4()))
    )
    async with asyncio.timeout(5):
        while after.resume_waiter is None:
            await asyncio.sleep(0)
    with pytest.raises(HubError) as same:
        await after.handle("chat.send", SendParams(text="Otra", session_id=sessions[0]), "x")
    with pytest.raises(HubError) as other:
        await after.handle("chat.send", SendParams(text="Otra", session_id=sessions[1]), "y")
    await after.close()
    with pytest.raises(HubError):
        await loading
    assert same.value.code == other.value.code == "session_resume_in_progress"
    assert same.value.message.startswith("Vega is still loading this conversation's history; wait")
    assert other.value.message.startswith(
        "Vega is already loading another history on this device; "
    )


class LosingConnection(TwoWayConnection):
    """The connection drops while the agent resumes the conversation."""

    async def resume_session(self, stored_id, history=None):
        resumed = await super().resume_session(stored_id, history)
        self.connected = False
        return resumed


class LosingBackend(TwoWayBackend):
    def connection(self, device, device_name, on_event):
        connection = LosingConnection(self, on_event)
        self.connections.append(connection)
        return connection


async def test_a_connection_lost_while_resuming_names_the_agent(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    agents = two_agents(TwoWayBackend("atlas"), LosingBackend("vega", asks=False))
    before = DeviceRuntime(owner, store, agents, lambda: 1000)
    _, sent = await before.handle(
        "chat.send", SendParams(text="Hola", agent_id="vega"), str(uuid4())
    )
    await before.close()
    after = DeviceRuntime(owner, store, agents, lambda: 1001)
    with pytest.raises(HubError) as lost:
        await after.handle(
            "chat.send", SendParams(text="Sigo", session_id=sent["session_id"]), str(uuid4())
        )
    assert lost.value.code == "session_resume_connection_lost"
    assert lost.value.message == (
        "The connection was lost with Vega while resuming; the message was not sent"
    )


# Startup -----------------------------------------------------------------------


def test_loading_the_configuration_imports_no_model_or_agent_library(tmp_path):
    """The administrator's commands (`pair-code`, `devices`, `revoke`) load the
    configuration and nothing more. The generic agent's model libraries take
    more than a second to import, and the official vendor SDK of Claude Code and Codex a
    quarter of one: only building that kind's backend imports them."""
    config = tmp_path / "orbit.toml"
    entries = "".join(
        "\n[[agents]]\n" + "".join(f"{key} = {json.dumps(value)}\n" for key, value in entry.items())
        for entry in every_kind(tmp_path)
    )
    config.write_text(
        f'data_dir = "{tmp_path / "data"}"\n'
        f'key_file = "{tmp_path / "key"}"\n'
        'hermes_url = "https://hermes.invalid"\n'
        f'hermes_credential_file = "{tmp_path / "never-read"}"\n'
        "\n[meeting_transcription]\n"
        'endpoint = "https://stt.example/v1"\n'
        'model = "modelo-sintetico"\n' + entries
    )
    script = (
        "import sys\n"
        "from pathlib import Path\n"
        "import orbit_hub.cli\n"
        "from orbit_hub.config import Settings\n"
        "configured, _ = Settings.load(Path(sys.argv[1])).agent_configurations()\n"
        "print(','.join(agent.kind for agent in configured))\n"
        "print(','.join(sorted({'pydantic_ai', 'openai', 'anthropic', 'claude_agent_sdk'} & set(sys.modules))))\n"
    )
    loaded = subprocess.run(
        [sys.executable, "-c", script, str(config)],
        cwd=Path(__file__).parents[1],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert loaded.returncode == 0, loaded.stderr
    assert loaded.stdout.splitlines() == ["hermes,openai,claude-code", ""]
