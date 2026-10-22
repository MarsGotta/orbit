"""Isolated synthetic upstream: never uses Hermes, credentials or private audio."""

import asyncio
import json
import shutil
from dataclasses import replace
from uuid import uuid4

import pytest
from support_audio import wav_bytes
from support_network_fence import install
from support_synthetic_stt import start_session_stt, stop_session_stt

from orbit_hub.app import create_app
from orbit_hub.assistant.agents import Agent, Agents
from orbit_hub.assistant.backend import OpenedSession
from orbit_hub.assistant.hermes import HermesAgent, HermesBackend, HermesConnection
from orbit_hub.assistant.speech import SpeechClip
from orbit_hub.bridge_route import PATH, PROTOCOL, BridgeConfiguration
from orbit_hub.config import Settings
from orbit_hub.private_auth import sign_request
from orbit_hub.runtime import DeviceRuntime
from orbit_hub.storage import SESSION_MESSAGES_PAGE_LIMIT, Store, initialize

BRIDGE_KEY = bytes(range(32))
BRIDGE_KEY_ID = "bridge-test-key"
BRIDGE_NOW_MS = 1_000_000

# Installed on import, before any test opens a socket. A test may only
# connect to a port this process bound; see support_network_fence.
FENCE = install()


@pytest.fixture(scope="session", autouse=True)
def unit_stt():
    """The suite's own meeting STT, for fixtures that must name one."""
    stt = start_session_stt()
    yield stt
    stop_session_stt()


@pytest.fixture(autouse=True)
def only_ports_this_process_bound():
    """Fail the test that tried to reach a port it did not open, even if swallowed."""
    mark = FENCE.refusals()
    yield
    refused = FENCE.refused_since(mark)
    assert not refused, (
        f"This test tried to connect to {', '.join(refused)}: they are not ports this "
        "process opened, so they were real services on this machine or someone else's. "
        "Use a synthetic service on an ephemeral port of its own (support_client_e2e.tcp_server)"
    )


class FakeConnection(HermesConnection):
    """Hermes's own connection over a synthetic transport: what Hermes says is
    read by the real translation, and `callback` takes Hermes events."""

    connected = False

    def __init__(self, callback, device="fixture-device", device_name="fixture"):
        super().__init__(None, callback, device, device_name)
        self.callback = self.receive
        self.epoch = "synthetic-epoch"
        self.calls = []

    async def open(self):
        self.connected = True

    async def create_session(self):
        runtime = str(uuid4())
        self.live_sessions.add(runtime)
        return OpenedSession(runtime, str(uuid4()))

    async def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "approval.respond":
            return {"resolved": 1}
        if method == "prompt.submit":
            await self.callback(
                {
                    "type": "message.delta",
                    "session_id": params["session_id"],
                    "payload": {"text": "respuesta sintética"},
                }
            )
            await self.callback(
                {
                    "type": "message.complete",
                    "session_id": params["session_id"],
                    "payload": {"text": "respuesta sintética", "status": "success"},
                }
            )
        return {"status": "streaming" if method == "prompt.submit" else "ok"}

    async def close(self):
        self.connected = False


class FakeAdapter:
    # It stands for Hermes, so it declares what Hermes does.
    capabilities = HermesBackend.capabilities

    def __init__(self):
        self.connections = []
        self.audio = []

    def connection(self, device, device_name, callback):
        connection = FakeConnection(callback, device, device_name)
        self.connections.append(connection)
        return connection

    async def transcribe(self, body):
        self.audio.append(body)
        return "Transcripción sintética"

    async def synthesize(self, text):
        return SpeechClip("audio/wav", "synthetic", wav_bytes(b"\x00\x00" * 32, rate=32000))


def only(backend, name="Deimos"):
    """The one implicit agent, on `backend`, for a test that builds a runtime itself."""
    return Agents([Agent("default", name, "hermes", HermesAgent("deimos"), backend)], "default")


@pytest.fixture
def environment(tmp_path, unit_stt, storage_template):
    data, key = tmp_path / "data", tmp_path / "key"
    template_data, template_key = storage_template
    shutil.copytree(template_data, data)
    shutil.copy2(template_key, key)
    settings = Settings(
        profile="deimos",
        data_dir=data,
        key_file=key,
        hermes_url="https://hermes.invalid",
        hermes_credential_file=tmp_path / "never-read",
        meeting_transcription=unit_stt.configuration(),
        allowed_hosts=("localhost",),
    )
    store = Store(data, key)
    adapter = FakeAdapter()
    yield settings, store, adapter
    store.close()


@pytest.fixture(scope="session")
def storage_template(tmp_path_factory):
    # initialize + Store cost 30 fsyncs; close once before independent copies.
    root = tmp_path_factory.mktemp("hub-schema-template")
    data, key = root / "data", root / "key"
    initialize(data, key)
    store = Store(data, key)
    store.close()
    return data, key


def device(store, name="fixture"):
    return store.pair(store.create_code(1000), name, 1001)


def said(store, owner, session):
    """What the conversation keeps, role and text, oldest first."""
    page = store.session_messages(owner, session, None, SESSION_MESSAGES_PAGE_LIMIT)
    return [{"role": message["role"], "text": message["text"]} for message in page["messages"]]


def bridge_body(capability="screen_sources", device_id="unset", **parameters):
    return json.dumps(
        {
            "protocol": PROTOCOL,
            "device_id": device_id,
            "capability": capability,
            "parameters": parameters,
        },
        separators=(",", ":"),
    ).encode()


def bridge_signed(
    payload,
    *,
    key=BRIDGE_KEY,
    key_id=BRIDGE_KEY_ID,
    issued=BRIDGE_NOW_MS,
    nonce=None,
    path=PATH,
):
    nonce = nonce or str(uuid4())
    return {
        "content-type": "application/json",
        **sign_request(PROTOCOL, path, key_id, key, payload, issued_at_ms=issued, nonce=nonce),
    }


async def answer_bridge_once(service, reply):
    """Resolve one bridge question through the same claim as the websocket."""
    for _ in range(200):
        pending = list(service.bridge.pending.values())
        if pending:
            entry = pending[0]
            claimed = service.bridge.resolve(entry.request_id, entry.device)
            waiter = service.waiters.get(entry.request_id)
            if claimed is not None and waiter is not None and not waiter.done():
                waiter.set_result(reply)
            return entry
        await asyncio.sleep(0.01)
    raise AssertionError("the question never reached the Mac")


@pytest.fixture
def bridged(environment, tmp_path):
    settings, store, adapter = environment
    path = tmp_path / "bridge-key"
    path.write_bytes(BRIDGE_KEY)
    path.chmod(0o600)
    settings = replace(
        settings,
        device_bridge=BridgeConfiguration(
            key_id=BRIDGE_KEY_ID,
            credential_file=str(path),
        ),
    )
    owner = device(store)
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: BRIDGE_NOW_MS / 1000
    )
    return app, store, adapter, owner


async def connect_bridge(app, store, adapter, owner):
    runtime = DeviceRuntime(owner["device_id"], store, only(adapter), lambda: BRIDGE_NOW_MS / 1000)
    await runtime.connect("default")
    runtime.subscribe()
    app.state.device_bridge.runtimes[owner["device_id"]] = runtime
    return runtime
