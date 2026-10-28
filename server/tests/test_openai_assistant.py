"""The `kind = "openai"` assistant, against a provider and a hub played by the test.

The provider is `support_openai_compatible.FakeProvider`: an OpenAI-compatible
Chat Completions service that streams and calls tools, on a loopback port of
this process. Orbit's tools are the real `orbit-mcp`, launched by the backend
over stdio, talking to `support_orbit_mcp.FakeHub` with real authentication.
Everything on the hub's side runs for real: the app, the device WebSocket, the
runtime and the store. No Hermes and no network beyond loopback.
"""

import asyncio
import base64
import json
import logging
import os
from dataclasses import replace
from uuid import uuid4

import pytest
from conftest import configured, device, said
from fastapi import FastAPI, Request
from fastapi.testclient import TestClient
from support_audio import wav_bytes
from support_openai_compatible import (
    FakeProvider,
    Hold,
    Refusal,
    Text,
    Thought,
    ToolCall,
    closed_port,
    serve,
)
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    OBSERVATION_REFUSED_NOTICE,
    OTHER_DEVICE,
    FakeHub,
    device_refused,
    write_key,
    write_mcp_config,
)
from support_synthetic_stt import SyntheticStt
from test_attachments import PNG, uploaded

from orbit_hub.app import create_app
from orbit_hub.assistant import openai as backend_module
from orbit_hub.assistant import openai_tools
from orbit_hub.assistant.backend import (
    ActivityItem,
    ImageBlock,
    MessageComplete,
    StatusNotice,
    StoredConversation,
    StoredMessage,
    TextBlock,
    TurnFailed,
)
from orbit_hub.assistant.openai import (
    KIND,
    OrbitTools,
    build_backend,
    build_speech,
)
from orbit_hub.meeting_transcription import MeetingTranscriber
from orbit_hub.protocol import HubError

# The secret the provider must receive and nothing else may ever show.
API_KEY = "clave-sintetica-de-prueba"
# A turn here is two short model requests and one orbit-mcp call; launching
# orbit-mcp is about a second. The bound only keeps a broken turn from
# holding the suite: it is never what a passing test waits for.
TURN_DEADLINE_SECONDS = 60


def api_key_file(tmp_path, value=API_KEY, mode=0o600):
    path = tmp_path / f"clave-{uuid4()}"
    path.write_text(value + "\n")
    os.chmod(path, mode)
    return path


def entry(provider_url, mcp_config, **extra):
    return {
        "id": "lyra",
        "name": "Lyra",
        "kind": KIND,
        "base_url": provider_url + "/v1",
        "model": "modelo-sintetico",
        "orbit_mcp_config": str(mcp_config),
        **extra,
    }


@pytest.fixture
def orbit_hub_for_tools(tmp_path):
    """The hub's private routes orbit-mcp signs its requests to, and its TOML."""
    hub = FakeHub()
    with serve(hub.app) as url:
        yield hub, write_mcp_config(tmp_path, url)


def next_event(ws):
    while True:
        frame = ws.receive_json()
        if frame["event"] != "state.snapshot":
            return frame


def turn_frames(ws, turn):
    """The turn's frames, from its first to the one that ends it."""
    frames = []
    while True:
        frame = next_event(ws)
        frames.append(frame)
        if frame["id"] == turn and (
            frame["event"] == "chat.completed" or frame["event"] == "error"
        ):
            return frames


def send(ws, text, session_id=None):
    turn = str(uuid4())
    params = {"text": text}
    if session_id is not None:
        params["session_id"] = session_id
    ws.send_json({"v": 1, "id": turn, "method": "chat.send", "params": params})
    return turn


def test_a_whole_turn_streams_and_calls_an_orbit_mcp_tool(environment, orbit_hub_for_tools):
    settings, store, speech = environment
    hub, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(
        ToolCall("orbit_devices"), Text(("Tienes ", "dos equipos: ", "portatil y sobremesa."))
    )
    with serve(provider.app) as provider_url:
        agent = configured(
            entry(provider_url, mcp_config, api_key_file=str(api_key_file(settings.data_dir)))
        )
        assistant = build_backend(settings, agent)
        paired = device(store, name="Portátil de Prueba")
        headers = {"Authorization": "Bearer " + paired["token"]}
        app = create_app(settings, store=store, assistant=assistant, speech=speech)
        with (
            TestClient(app, base_url="https://localhost") as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            assert next_event(ws)["event"] == "ready"
            turn = send(ws, "¿Qué equipos tengo?")
            frames = turn_frames(ws, turn)

    assert all(frame["id"] == turn for frame in frames)
    session = frames[0]["data"]["session_id"]
    assert frames[0] == {
        "v": 1,
        "id": turn,
        "event": "chat.session",
        "data": {"session_id": session},
    }
    by_event = {}
    for frame in frames:
        by_event.setdefault(frame["event"], []).append(frame["data"])
    # The tool call is a step of the agent's work, not a status line.
    assert "chat.status" not in by_event
    look = {
        "session_id": session,
        "item_id": "call-sintetico-1",
        "kind": "tool",
        "label": "Looking at the Orbit Macs",
        "subject": "the Macs",
        "label_copy": {"code": "orbit_devices"},
        "subject_code": "macs",
    }
    assert by_event["chat.item"] == [
        {**look, "phase": "started"},
        {**look, "phase": "completed"},
    ]
    assert [delta["text"] for delta in by_event["chat.delta"]] == [
        "Tienes ",
        "dos equipos: ",
        "portatil y sobremesa.",
    ]
    assert {"session_id": session, "status": "streaming"} in by_event["chat.session"]
    assert frames[-1]["data"] == {
        "session_id": session,
        "text": "Tienes dos equipos: portatil y sobremesa.",
        "status": "completed",
        "persisted": True,
    }

    # The model was asked twice: with the turn and Orbit's tools, then with
    # what orbit-mcp answered, read from the hub's private routes.
    first, second = provider.requests
    assert first["model"] == "modelo-sintetico" and first["stream"] is True
    assert {tool["function"]["name"] for tool in first["tools"]} >= {
        "orbit_devices",
        "orbit_windows",
        "orbit_screenshot",
    }
    system, user = first["messages"]
    assert system["role"] == "system" and "You are Lyra" in system["content"]
    assert user == {
        "role": "user",
        "content": "[Orbit · Portátil de Prueba · text]\n¿Qué equipos tengo?",
    }
    tool_result = second["messages"][-1]
    assert tool_result["role"] == "tool"
    assert DEFAULT_DEVICE in tool_result["content"] and OTHER_DEVICE in tool_result["content"]
    assert hub.device_list_requests == 1
    assert provider.headers[0]["authorization"] == f"Bearer {API_KEY}"
    assert said(store, paired["device_id"], session) == [
        {"role": "user", "text": "¿Qué equipos tengo?"},
        {"role": "assistant", "text": "Tienes dos equipos: portatil y sobremesa."},
    ]


def test_a_conversation_continues_from_the_hub_after_a_restart(environment, orbit_hub_for_tools):
    """The backend keeps nothing durable: a new runtime resumes the stored
    conversation, and the model receives it as the Mac shows it."""
    settings, store, speech = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(Text(("Apuntado.",)), Text(("Era el martes.",)))
    paired = device(store, name="Portátil de Prueba")
    headers = {"Authorization": "Bearer " + paired["token"]}
    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config))
        session = None
        for text in ("La excursión es el martes.", "¿Qué día era?"):
            # A new app each time: a new runtime and connection, as after a restart.
            app = create_app(
                settings, store=store, assistant=build_backend(settings, agent), speech=speech
            )
            with (
                TestClient(app, base_url="https://localhost") as http,
                http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
            ):
                assert next_event(ws)["event"] == "ready"
                frames = turn_frames(ws, send(ws, text, session))
                assert frames[-1]["event"] == "chat.completed", frames[-1]
                session = frames[-1]["data"]["session_id"]

    later = provider.requests[1]["messages"]
    assert [(message["role"], message["content"]) for message in later[1:]] == [
        ("user", "La excursión es el martes."),
        ("assistant", "Apuntado."),
        ("user", "[Orbit · Portátil de Prueba · text]\n¿Qué día era?"),
    ]
    # No key file: an OpenAI-compatible server that needs none gets the placeholder.
    assert provider.headers[0]["authorization"] == f"Bearer {backend_module.NO_API_KEY}"


def test_the_ceiling_leaves_out_the_oldest_messages_and_says_how_many(monkeypatch):
    monkeypatch.setattr(backend_module, "HISTORY_MAX_CHARS", 49)
    session = backend_module._Session(
        "guardada",
        [("user", "u" * 30), ("assistant", "a" * 30), ("user", "v" * 10), ("assistant", "b" * 10)],
        left_out=3,
    )
    messages, size = session.history()
    assert size == 20
    note, *kept = messages
    assert note.parts[0].content == (
        "[Orbit · omitted the 5 oldest messages of this conversation: "
        "the model receives at most 49 history characters]"
    )
    assert [part.content for message in kept for part in message.parts] == ["v" * 10, "b" * 10]
    # What was left out is no longer held: the next turn trims from here.
    assert session.held == [("user", "v" * 10), ("assistant", "b" * 10)]
    assert session.left_out == 5


def test_a_resumed_session_counts_what_the_hub_page_left_out():
    connection = backend_module.OpenAIConnection.__new__(backend_module.OpenAIConnection)
    connection._sessions, connection.live_sessions = {}, set()
    resumed = asyncio.run(
        connection.resume_session(
            "guardada",
            StoredConversation(
                (StoredMessage("user", "hola"), StoredMessage("assistant", "buenas")), total=230
            ),
        )
    )
    assert resumed.message_count == 2 and resumed.loading_history is False
    held = connection._sessions[resumed.session_id]
    assert held.held == [("user", "hola"), ("assistant", "buenas")] and held.left_out == 228


def test_orbit_mcp_is_launched_with_the_agent_identity_and_nothing_of_the_hub(mcp_toml):
    agent = configured(entry("https://ia.example", mcp_toml))
    parameters = OrbitTools(agent).parameters()
    assert parameters.args == ["-m", "orbit_mcp.cli"]
    assert parameters.env == {
        "ORBIT_MCP_CONFIG": str(mcp_toml),
        "ORBIT_MCP_AGENT": "lyra",
    }


@pytest.fixture
def mcp_toml(tmp_path):
    path = tmp_path / "mcp.toml"
    path.write_text("# orbit-mcp validates its own file\n")
    return path


async def run_turn(agent, hub_settings, text="Hola", blocks=None):
    """One turn on a connection of its own; the events it produced."""
    events = []

    async def on_event(event):
        events.append(event)

    connection = build_backend(hub_settings, agent).connection(
        "dispositivo", "Mac de Prueba", on_event
    )
    await connection.open()
    try:
        opened = await connection.create_session()
        await connection.prompt(opened.session_id, blocks or (TextBlock(text),))
        async with asyncio.timeout(TURN_DEADLINE_SECONDS):
            while not events or not isinstance(events[-1], MessageComplete | TurnFailed):
                await asyncio.sleep(0.01)
    finally:
        await connection.close()
    return events


async def failure_of(environment, orbit_hub_for_tools, *replies, base_url=None, **extra):
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(*replies)
    with serve(provider.app) as provider_url:
        raw = entry(base_url or provider_url, mcp_config, **extra)
        events = await run_turn(configured(raw), settings)
    failed = events[-1]
    assert isinstance(failed, TurnFailed), events
    return failed, provider


async def test_an_unreachable_provider_names_its_url_and_deadline(environment, orbit_hub_for_tools):
    nowhere = f"http://127.0.0.1:{closed_port()}"
    failed, _ = await failure_of(environment, orbit_hub_for_tools, Text(("",)), base_url=nowhere)
    assert failed.code == "assistant_unreachable"
    assert f"{nowhere}/v1" in failed.message and "10000 ms" in failed.message
    assert failed.numbers == {"connect_timeout_ms": 10_000}


async def test_a_provider_that_stops_answering_names_the_deadline(
    environment, orbit_hub_for_tools, monkeypatch
):
    monkeypatch.setattr(backend_module, "MODEL_READ_TIMEOUT_SECONDS", 1)
    failed, provider = await failure_of(environment, orbit_hub_for_tools, Hold())
    assert failed.code == "assistant_timeout"
    assert failed.numbers == {"timeout_ms": 1000}
    assert "1000 ms" in failed.message and "/v1" in failed.message
    assert len(provider.requests) == 1, "a request that timed out is not sent again"


@pytest.mark.parametrize("status", [401, 403])
async def test_a_refused_key_is_named_and_never_shown(
    environment, orbit_hub_for_tools, status, caplog
):
    settings, _, _ = environment
    refusal = Refusal(
        status,
        {"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}},
    )
    with caplog.at_level(logging.DEBUG):
        failed, provider = await failure_of(
            environment,
            orbit_hub_for_tools,
            refusal,
            api_key_file=str(api_key_file(settings.data_dir)),
        )
    assert failed.code == "assistant_unauthorized"
    assert f"HTTP {status}" in failed.message and "api_key_file" in failed.message
    assert failed.numbers == {"provider_status": status}
    assert provider.headers[0]["authorization"] == f"Bearer {API_KEY}"
    assert API_KEY not in failed.message and API_KEY not in caplog.text


async def test_a_rate_limit_carries_the_wait_the_provider_asked_for(
    environment, orbit_hub_for_tools
):
    refusal = Refusal(
        429,
        {
            "error": {
                "message": "Rate limit reached",
                "type": "requests",
                "code": "rate_limit_exceeded",
            }
        },
        headers={"retry-after": "7"},
    )
    failed, provider = await failure_of(environment, orbit_hub_for_tools, refusal)
    assert failed.code == "assistant_rate_limited"
    assert failed.numbers == {"provider_status": 429, "retry_after_ms": 7000}
    assert "7000 ms" in failed.message
    assert len(provider.requests) == 1, "the hub does not retry behind the user's back"


async def test_a_rate_limit_without_a_wait_says_so(environment, orbit_hub_for_tools):
    refusal = Refusal(429, {"error": {"message": "Too many requests", "type": "requests"}})
    failed, _ = await failure_of(environment, orbit_hub_for_tools, refusal)
    assert failed.code == "assistant_rate_limited"
    assert failed.numbers == {"provider_status": 429}
    assert "did not say how long to wait" in failed.message


async def test_an_exceeded_context_carries_the_tokens_the_provider_gave(
    environment, orbit_hub_for_tools
):
    refusal = Refusal(
        400,
        {
            "error": {
                "message": "This model's maximum context length is 8192 tokens. However, your "
                "messages resulted in 9100 tokens. Please reduce the length of the messages.",
                "type": "invalid_request_error",
                "code": "context_length_exceeded",
            }
        },
    )
    failed, _ = await failure_of(environment, orbit_hub_for_tools, refusal)
    assert failed.code == "assistant_context_exceeded"
    assert failed.numbers == {
        "history_chars": 0,
        "max_history_chars": backend_module.HISTORY_MAX_CHARS,
        "max_tokens": 8192,
        "tokens": 9100,
    }
    assert "9100 tokens out of a maximum of 8192" in failed.message


async def test_a_tool_that_never_answers_fails_the_turn_with_its_name(
    environment, tmp_path, monkeypatch
):
    """orbit-mcp is reached, but the hub behind it holds every request."""
    settings, _, _ = environment
    monkeypatch.setattr(openai_tools, "TOOL_CALL_TIMEOUT_SECONDS", 1)
    silent_hub = FastAPI()

    @silent_hub.api_route("/{path:path}", methods=["GET", "POST"])
    async def hold(path: str, request: Request):
        await asyncio.Future()

    provider = FakeProvider(ToolCall("orbit_devices"), Text(("no debería llegar",)))
    with serve(silent_hub) as hub_url, serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, write_mcp_config(tmp_path, hub_url)))
        events = await run_turn(agent, settings)
    failed = events[-1]
    assert isinstance(failed, TurnFailed), events
    assert failed.code == "assistant_tool_failed"
    assert "orbit_devices" in failed.message and "1000 ms" in failed.message
    assert "will not be repeated" in failed.message
    assert failed.numbers == {"timeout_ms": 1000}
    assert len(provider.requests) == 1, "the model is not asked again after a lost tool call"


async def test_a_tool_refusal_reaches_the_model_to_read(environment, tmp_path):
    """orbit-mcp's own refusals are answers for the model, not failed turns."""
    settings, _, _ = environment
    hub = FakeHub(device_replies=[device_refused(OBSERVATION_REFUSED_NOTICE)])
    provider = FakeProvider(
        ToolCall("orbit_windows", {"device": DEFAULT_DEVICE}),
        Text(("macOS no deja ver la pantalla.",)),
    )
    with serve(hub.app) as hub_url, serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, write_mcp_config(tmp_path, hub_url)))
        events = await run_turn(agent, settings)
    assert events[-1] == MessageComplete(
        events[-1].session_id, "macOS no deja ver la pantalla.", "completed"
    )
    tool_result = provider.requests[1]["messages"][-1]
    assert tool_result["role"] == "tool"
    assert "Screen Recording" in tool_result["content"]


@pytest.mark.parametrize("images", [True, False])
async def test_a_screenshot_reaches_the_model_only_if_it_reads_images(
    environment, orbit_hub_for_tools, images
):
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(
        ToolCall("orbit_screenshot", {"device": DEFAULT_DEVICE}), Text(("Veo Xcode.",))
    )
    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config, images=images))
        events = await run_turn(agent, settings)
    assert isinstance(events[-1], MessageComplete), events
    sent = json.dumps(provider.requests[1]["messages"], ensure_ascii=False)
    assert ("data:image/png;base64," in sent) is images
    assert ("this model does not support images" in sent) is not images


def items(events):
    return [event for event in events if isinstance(event, ActivityItem)]


async def test_a_tool_call_is_one_item_started_then_completed(environment, orbit_hub_for_tools):
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(
        ToolCall("orbit_screenshot", {"device": DEFAULT_DEVICE, "application": "Xcode"}),
        Text(("Veo Xcode.",)),
    )
    with serve(provider.app) as provider_url:
        events = await run_turn(configured(entry(provider_url, mcp_config)), settings)
    assert isinstance(events[-1], MessageComplete), events
    session = events[-1].session_id
    arguments = {"device": DEFAULT_DEVICE, "application": "Xcode"}
    assert items(events) == [
        ActivityItem(session, "call-sintetico-1", "tool", "started", "orbit_screenshot", arguments),
        ActivityItem(
            session, "call-sintetico-1", "tool", "completed", "orbit_screenshot", arguments
        ),
    ]
    assert not [event for event in events if isinstance(event, StatusNotice)]


async def test_an_orbit_mcp_refusal_is_a_failed_tool_item_with_its_cause(environment, tmp_path):
    """orbit-mcp's refusal reaches the model as a failed tool result: the step
    failed, though the turn goes on."""
    settings, _, _ = environment
    hub = FakeHub(device_replies=[device_refused(OBSERVATION_REFUSED_NOTICE)])
    provider = FakeProvider(
        ToolCall("orbit_windows", {"device": DEFAULT_DEVICE}),
        Text(("macOS no deja ver la pantalla.",)),
    )
    with serve(hub.app) as hub_url, serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, write_mcp_config(tmp_path, hub_url)))
        events = await run_turn(agent, settings)
    assert isinstance(events[-1], MessageComplete), events
    started, failed = items(events)
    assert (started.phase, failed.phase) == ("started", "failed")
    assert failed.item_id == started.item_id == "call-sintetico-1"
    assert failed.tool == "orbit_windows"
    assert "Screen Recording" in failed.cause


async def test_a_call_the_model_must_retry_is_a_failed_item_with_the_cause(
    environment, orbit_hub_for_tools
):
    """A tool that does not exist: pydantic-ai asks the model to retry
    (`RetryPromptPart`), and the step failed with that cause."""
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(ToolCall("orbit_inventada"), Text(("No tengo esa herramienta.",)))
    with serve(provider.app) as provider_url:
        events = await run_turn(configured(entry(provider_url, mcp_config)), settings)
    assert isinstance(events[-1], MessageComplete), events
    started, failed = items(events)
    assert (started.phase, failed.phase) == ("started", "failed")
    assert (failed.item_id, failed.tool) == ("call-sintetico-1", "orbit_inventada")
    assert "orbit_inventada" in failed.cause


async def test_a_thought_is_one_reasoning_item_completed_with_the_whole_thought(
    environment, orbit_hub_for_tools
):
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(
        Thought(("Primero miro ", "los equipos."), ToolCall("orbit_devices")),
        Thought(("Ya los tengo.",), Text(("Tienes dos.",))),
    )
    with serve(provider.app) as provider_url:
        events = await run_turn(configured(entry(provider_url, mcp_config)), settings)
    assert events[-1] == MessageComplete(events[-1].session_id, "Tienes dos.", "completed")
    steps = items(events)
    assert [(step.kind, step.phase) for step in steps] == [
        ("reasoning", "started"),
        ("reasoning", "completed"),
        ("tool", "started"),
        ("tool", "completed"),
        ("reasoning", "started"),
        ("reasoning", "completed"),
    ]
    first, thought, _, _, second, last = steps
    assert first.item_id == thought.item_id != second.item_id == last.item_id
    assert (first.text, thought.text) == ("", "Primero miro los equipos.")
    assert last.text == "Ya los tengo."


async def test_this_assistant_never_asks_permission_and_answers_by_choice(environment, mcp_toml):
    settings, _, _ = environment
    agent = configured(entry("http://127.0.0.1:9", mcp_toml))

    async def ignore(event):
        pass

    connection = build_backend(settings, agent).connection("dispositivo", "Mac", ignore)
    for choice in ("once", "deny"):
        with pytest.raises(HubError) as refused:
            await connection.respond_permission("sesion", "peticion", choice=choice)
        assert refused.value.code == "interaction_not_found"


async def test_orbit_mcp_that_cannot_start_refuses_the_connection(environment, mcp_toml):
    settings, _, _ = environment
    agent = configured(entry("https://ia.example", mcp_toml))
    connection = build_backend(settings, agent).connection("dispositivo", "Mac", None)
    with pytest.raises(HubError) as refused:
        await connection.open()
    assert refused.value.code == "assistant_tools_unavailable"
    assert not connection.connected


async def test_interrupting_ends_the_turn_with_what_was_said(environment, orbit_hub_for_tools):
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(Hold())
    events = []

    async def on_event(event):
        events.append(event)

    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config))
        connection = build_backend(settings, agent).connection("dispositivo", "Mac", on_event)
        await connection.open()
        try:
            opened = await connection.create_session()
            await connection.prompt(opened.session_id, (TextBlock("Cuéntame algo largo"),))
            async with asyncio.timeout(TURN_DEADLINE_SECONDS):
                while not provider.requests:
                    await asyncio.sleep(0.01)
            assert not await connection.turn_stopped(opened.session_id)
            await connection.cancel(opened.session_id)
            async with asyncio.timeout(TURN_DEADLINE_SECONDS):
                while not events:
                    await asyncio.sleep(0.01)
            assert await connection.turn_stopped(opened.session_id)
        finally:
            await connection.close()
    assert events == [MessageComplete(opened.session_id, "", "interrupted")]


def test_a_turn_failure_reaches_the_mac_with_its_cause_and_numbers(
    environment, orbit_hub_for_tools
):
    settings, store, speech = environment
    _, mcp_config = orbit_hub_for_tools
    refusal = Refusal(
        429, {"error": {"message": "Rate limit reached", "type": "requests"}}, {"retry-after": "3"}
    )
    provider = FakeProvider(refusal)
    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config))
        paired = device(store, name="Portátil de Prueba")
        headers = {"Authorization": "Bearer " + paired["token"]}
        app = create_app(
            settings, store=store, assistant=build_backend(settings, agent), speech=speech
        )
        with (
            TestClient(app, base_url="https://localhost") as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            assert next_event(ws)["event"] == "ready"
            turn = send(ws, "Hola")
            frames = turn_frames(ws, turn)
    error = frames[-1]
    session = frames[0]["data"]["session_id"]
    assert error["event"] == "error" and error["id"] == turn
    assert error["data"]["session_id"] == session
    assert error["data"]["code"] == "assistant_rate_limited"
    assert "3000 ms" in error["data"]["message"]
    assert error["data"]["refusal"]["code"] == "assistant_rate_limited"
    assert error["data"]["refusal"]["retry_after_ms"] == 3000
    assert store.chat.session(paired["device_id"], session)["status"] == "failed"
    assert said(store, paired["device_id"], session) == [{"role": "user", "text": "Hola"}]


# ---------------------------------------------------------------- attachments


def attached_turns(environment, mcp_config, provider, *turns, **extra):
    """Turns of one conversation, each `(text, file_name)` with a PNG attached
    when it names one, on a hub with this agent; `ready` and each turn's frames."""
    settings, store, speech = environment
    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config, **extra))
        paired = device(store, name="Portátil de Prueba")
        headers = {"Authorization": "Bearer " + paired["token"]}
        app = create_app(
            settings, store=store, assistant=build_backend(settings, agent), speech=speech
        )
        with (
            TestClient(app, base_url="https://localhost") as http,
            http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
        ):
            ready = next_event(ws)
            assert ready["event"] == "ready"
            session, done = None, []
            for text, name in turns:
                ids = [uploaded(http, paired, name, PNG)["id"]] if name else []
                turn = str(uuid4())
                params = {"text": text, "attachments": ids}
                if session is not None:
                    params["session_id"] = session
                ws.send_json({"v": 1, "id": turn, "method": "chat.send", "params": params})
                frames = turn_frames(ws, turn)
                session = frames[0]["data"]["session_id"]
                done.append((ids, frames))
    return ready["data"], done


def test_an_attached_image_reaches_the_model_after_the_text_and_then_by_its_id(
    environment, orbit_hub_for_tools
):
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(Text(("Marca el sendero rojo.",)), Text(("Sí, el rojo.",)))
    ready, [([image], first), (_, second)] = attached_turns(
        environment,
        mcp_config,
        provider,
        ("¿Qué ruta marca?", "mapa de la excursión.png"),
        ("¿Seguro?", None),
        images=True,
    )
    assert "images" in ready["agents"][0]["capabilities"]
    assert first[-1]["event"] == "chat.completed", first[-1]
    assert second[-1]["event"] == "chat.completed", second[-1]
    text, picture = provider.requests[0]["messages"][-1]["content"]
    assert text["type"] == "text"
    assert "¿Qué ruta marca?" in text["text"]
    assert (
        f"«mapa de la excursión.png» · image image/png · {len(PNG)} B · id {image}"
        in (text["text"])
    )
    assert picture == {
        "type": "image_url",
        "image_url": {"url": "data:image/png;base64," + base64.b64encode(PNG).decode()},
    }
    # The next turn carries the image as a line naming its attachment, never
    # its bytes again: the model looks at it once more with orbit_attachment.
    later = json.dumps(provider.requests[1]["messages"], ensure_ascii=False)
    assert "data:image" not in later
    held = provider.requests[1]["messages"][1]
    assert held["role"] == "user"
    assert (
        f"[Orbit · image «mapa de la excursión.png» (attachment {image}) was included in this message and is not resent; to view it again use orbit_attachment with that ID]"
    ) in held["content"]


def test_without_images_an_image_is_named_as_one_the_agent_cannot_see(
    environment, orbit_hub_for_tools
):
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(Text(("La miro.",)))
    ready, [([image], frames)] = attached_turns(
        environment, mcp_config, provider, ("Mira esto", "foto.png")
    )
    assert "images" not in ready["agents"][0]["capabilities"]
    assert frames[-1]["event"] == "chat.completed", frames[-1]
    content = provider.requests[0]["messages"][-1]["content"]
    assert isinstance(content, str), "no image part goes to a model without images"
    assert f"«foto.png» · image image/png · {len(PNG)} B · id {image}: " in content
    # orbit_attachment would hand it a note in place of the image: not sent there.
    assert "míralo con orbit_attachment" not in content
    assert "it cannot view it" in content


def test_an_image_the_provider_refuses_fails_the_turn_naming_the_file(
    environment, orbit_hub_for_tools, caplog
):
    _, mcp_config = orbit_hub_for_tools
    refusal = Refusal(
        400,
        {
            "error": {
                "message": "Invalid image data",
                "type": "invalid_request_error",
                "code": "invalid_image",
            }
        },
    )
    provider = FakeProvider(refusal, Text(("Hola.",)))
    with caplog.at_level(logging.DEBUG):
        _, [(_, refused), (_, retried)] = attached_turns(
            environment,
            mcp_config,
            provider,
            ("Mira", "panorámica del valle.png"),
            ("Sin la foto, entonces", None),
            images=True,
        )
    error = refused[-1]
    assert error["event"] == "error", error
    assert error["data"]["code"] == "attachment_refused"
    assert "«panorámica del valle.png»" in error["data"]["message"]
    assert "HTTP 400" in error["data"]["message"]
    assert error["data"]["refusal"]["name"] == "panorámica del valle.png"
    assert error["data"]["refusal"]["provider_status"] == 400
    assert error["data"]["refusal"]["retry"] == {"kind": "after_fix"}
    # The model never read the refused message: the next turn starts without it.
    assert retried[-1]["event"] == "chat.completed", retried[-1]
    assert [message["role"] for message in provider.requests[1]["messages"]] == ["system", "user"]
    # The file's name goes to the Mac, never to the hub's log.
    assert "panorámica" not in caplog.text


async def test_a_refusal_after_the_model_answered_is_not_blamed_on_the_image(
    environment, orbit_hub_for_tools
):
    """Only the provider's first answer can be about the turn's images: a
    refusal after a tool call keeps its own cause."""
    settings, _, _ = environment
    _, mcp_config = orbit_hub_for_tools
    provider = FakeProvider(
        ToolCall("orbit_devices"),
        Refusal(400, {"error": {"message": "Bad request", "type": "invalid_request_error"}}),
    )
    with serve(provider.app) as provider_url:
        agent = configured(entry(provider_url, mcp_config, images=True))
        events = await run_turn(
            agent,
            settings,
            blocks=(TextBlock("Mira"), ImageBlock("adjunto-1", "a.png", "image/png", PNG)),
        )
    failed = events[-1]
    assert isinstance(failed, TurnFailed), events
    assert failed.code == "assistant_http_error"
    assert failed.numbers == {"provider_status": 400}


# ---------------------------------------------------------------- voice


async def test_dictation_uses_the_hub_meeting_transcriber(environment, mcp_toml):
    settings, _, _ = environment
    stt = SyntheticStt("Apunta la excursión")
    with serve(stt.app) as stt_url:
        hub_settings = replace(settings, meeting_transcription=stt.configuration(stt_url))
        speech = build_speech(hub_settings, configured(entry("https://ia.example", mcp_toml)))
        text = await speech.transcribe(wav_bytes(b"\x10\x00" * 1600))
    assert text == "Apunta la excursión"
    assert isinstance(speech.transcriber, MeetingTranscriber) and len(stt.audio) == 1


async def test_without_a_speech_service_there_is_no_voice_and_reading_aloud_says_why(
    environment, mcp_toml
):
    settings, _, _ = environment
    agent = configured(entry("https://ia.example", mcp_toml))
    assert build_backend(settings, agent).capabilities.voice is False
    with pytest.raises(HubError) as refused:
        await build_speech(settings, agent).synthesize("Hola")
    assert refused.value.code == "speech_not_configured"
    assert "Lyra" in refused.value.message and "[agents.speech]" in refused.value.message


async def test_an_unreachable_speech_service_is_named(environment, mcp_toml):
    settings, _, _ = environment
    nowhere = f"http://127.0.0.1:{closed_port()}/v1"
    speech_entry = {"base_url": nowhere, "model": "voz-sintetica", "voice": "alloy"}
    agent = configured(entry("https://ia.example", mcp_toml, speech=speech_entry))
    with pytest.raises(HubError) as refused:
        await build_speech(settings, agent).synthesize("Hola")
    assert refused.value.code == "speech_unreachable"
    assert nowhere in refused.value.message


async def test_a_speech_service_reads_aloud(environment, mcp_toml):
    settings, _, _ = environment
    provider = FakeProvider(Text(("",)), speech=b"ID3 clip sintetico")
    with serve(provider.app) as provider_url:
        speech_entry = {
            "base_url": provider_url + "/v1",
            "model": "voz-sintetica",
            "voice": "alloy",
            "api_key_file": str(api_key_file(settings.data_dir)),
        }
        agent = configured(entry("https://ia.example", mcp_toml, speech=speech_entry))
        assert build_backend(settings, agent).capabilities.voice is True
        clip = await build_speech(settings, agent).synthesize("Hola, Lyra al habla")
    assert (clip.mime_type, clip.provider, clip.body) == (
        "audio/mpeg",
        "voz-sintetica",
        b"ID3 clip sintetico",
    )
    assert provider.speech_requests == [
        {
            "model": "voz-sintetica",
            "voice": "alloy",
            "input": "Hola, Lyra al habla",
            "response_format": "mp3",
        }
    ]


# ---------------------------------------------------------------- configuration


@pytest.mark.parametrize(
    ("change", "refusal"),
    [
        ({"provider": "gemini"}, 'provider: must be "openai" or "anthropic"'),
        ({"base_url": None}, "missing keys (1): base_url"),
        ({"profile": "deimos"}, "unknown keys (1): profile"),
        ({"model": ""}, "model: is empty"),
        ({"base_url": "http://ia.example/v1"}, "base_url: URL requires HTTPS"),
        (
            {"base_url": "https://usuario:secreto@ia.example/v1"},
            "base_url: A plain URL must not contain credentials",
        ),
        ({"images": "sí"}, "images: must be true or false"),
        ({"orbit_mcp_config": "mcp.toml"}, "orbit_mcp_config: must be an absolute path"),
        (
            {"speech": {"base_url": "https://voz.example/v1", "model": "tts"}},
            "speech: missing keys (1): voice",
        ),
        ({"provider": "anthropic"}, "missing keys (1): api_key_file"),
    ],
)
def test_an_invalid_entry_is_refused_naming_the_key_and_why(mcp_toml, change, refusal):
    raw = entry("https://ia.example", mcp_toml)
    for key, value in change.items():
        if value is None:
            raw.pop(key)
        else:
            raw[key] = value
    with pytest.raises(ValueError) as refused:
        configured(raw)
    assert str(refused.value).startswith(f'[[agents]] entry 1 (id "lyra"): {refusal}'), str(
        refused.value
    )


@pytest.mark.parametrize(
    ("change", "refusal"),
    [
        (
            {"orbit_mcp_config": "/no/existe/mcp.toml"},
            'agent "lyra".orbit_mcp_config: /no/existe/mcp.toml does not exist',
        ),
        (
            {
                "speech": {
                    "base_url": "https://voz.example/v1",
                    "model": "tts",
                    "voice": "alloy",
                    "api_key_file": "/no/existe.key",
                }
            },
            'agent "lyra".speech.api_key_file: /no/existe.key does not exist',
        ),
    ],
)
def test_a_file_the_agent_needs_is_refused_when_the_hub_starts(
    environment, mcp_toml, change, refusal
):
    agent = configured({**entry("https://ia.example", mcp_toml), **change})
    with pytest.raises(ValueError) as refused:
        build_backend(environment[0], agent)
    assert str(refused.value).startswith(refusal), str(refused.value)


@pytest.mark.parametrize(
    ("mode", "content", "refusal"),
    [
        (0o644, API_KEY, "permissions must be 0600"),
        (0o600, "   ", "is empty"),
    ],
)
def test_a_key_file_is_refused_with_its_cause_and_never_its_content(
    environment, tmp_path, mcp_toml, mode, content, refusal
):
    path = api_key_file(tmp_path, content, mode)
    agent = configured(entry("https://ia.example", mcp_toml, api_key_file=str(path)))
    with pytest.raises(ValueError) as refused:
        build_backend(environment[0], agent)
    message = str(refused.value)
    assert message.startswith(f'agent "lyra".api_key_file: {path}') and refusal in message
    assert API_KEY not in message


def test_a_missing_key_file_is_named(environment, tmp_path, mcp_toml):
    agent = configured(entry("https://ia.example", mcp_toml, api_key_file=str(tmp_path / "nada")))
    with pytest.raises(ValueError, match="does not exist"):
        build_backend(environment[0], agent)


def test_a_valid_entry_for_anthropic_and_a_local_server(tmp_path, mcp_toml):
    key = write_key(tmp_path, "anthropic.key", API_KEY.encode())
    claude = configured(
        {
            "id": "claude",
            "name": "Clío",
            "kind": "openai",
            "provider": "anthropic",
            "model": "modelo-de-prueba",
            "api_key_file": str(key),
            "orbit_mcp_config": str(mcp_toml),
            "images": True,
        }
    ).settings
    assert (claude.provider, claude.base_url, claude.images) == ("anthropic", None, True)
    assert backend_module.provider_url(claude) == "https://api.anthropic.com"
    local = configured(entry("http://127.0.0.1:11434", mcp_toml)).settings
    assert (local.provider, local.base_url, local.api_key_file) == (
        "openai",
        "http://127.0.0.1:11434/v1",
        None,
    )
