"""Several agents on one hub.

`orbit.toml` may list `[[agents]]`; without it the hub is the one implicit
agent of 0.9.0. Every agent here is Hermes, each with its own profile, and the
Hermes is a synthetic gateway on a loopback port of this process: what reaches
it shows which profile, and so which agent, a turn went to. Names, ids and
profiles are invented.
"""

import itertools
import json
import os
import sqlite3
import threading
from dataclasses import replace
from pathlib import Path
from uuid import uuid4

import pytest
from conftest import device
from fastapi.testclient import TestClient
from websockets.sync.server import serve

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.storage import Store


def config_file(tmp_path, head="", entries=()):
    """An `orbit.toml` with the top-level keys `head` and one `[[agents]]` per entry."""
    tables = "".join(
        "\n[[agents]]\n" + "".join(f"{key} = {json.dumps(value)}\n" for key, value in entry.items())
        for entry in entries
    )
    path = tmp_path / "orbit.toml"
    path.write_text(
        head
        + """data_dir = "/fixture/data"
key_file = "/fixture/key"
hermes_url = "https://hermes.example.test"
hermes_credential_file = "/fixture/token"
"""
        + tables
        + """
[meeting_transcription]
endpoint = "https://transcriber.example.test"
model = "fixture-model"
"""
    )
    return path


TWO_AGENTS = (
    {"id": "deimos", "name": "Deimos", "kind": "hermes", "profile": "deimos-profile"},
    {"id": "lyra", "name": "Lyra", "kind": "hermes", "profile": "lyra-profile"},
)
HERMES_CAPABILITIES = ["images", "voice", "resume"]


class SyntheticHermes:
    """Hermes's gateway as far as the hub uses it, on a thread of its own so
    the synchronous TestClient can drive the hub at the same time. Each live
    session id names the profile it was opened with."""

    def __init__(self, tmp_path):
        self.calls: list[tuple[int, str, dict]] = []
        self.connections = itertools.count(1)
        self.numbers = itertools.count(1)
        self.profiles: dict[str, str] = {}
        # Hermes's code for every `image.attach_bytes` (4018: over its
        # 25 MiB); None stages each image as tui_gateway does.
        self.image_refusal: int | None = None
        # Hermes's reply to `image.attach_bytes` in place of tui_gateway's
        # staging one: an answer that does not confirm the image.
        self.image_reply: dict | None = None
        self.server = serve(self.handle, "127.0.0.1", 0)
        self.port = self.server.socket.getsockname()[1]
        self.closing = False
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.credential = tmp_path / "synthetic-hermes-credential"
        fd = os.open(self.credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            os.write(fd, b"synthetic-token-only")
        finally:
            os.close(fd)

    def __enter__(self):
        self.thread.start()
        return self

    def run(self):
        try:
            self.server.serve_forever()
        except OSError:
            # Shut down before it began listening: the test was already over.
            if not self.closing:
                raise

    def __exit__(self, *_):
        self.closing = True
        self.server.shutdown()
        self.thread.join(5)

    def settings(self, settings: Settings, **changes) -> Settings:
        return replace(
            settings,
            hermes_url=f"http://127.0.0.1:{self.port}",
            allow_insecure_localhost=True,
            hermes_credential_file=self.credential,
            **changes,
        )

    def handle(self, ws):
        connection = next(self.connections)
        ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": f"epoch-{connection}"},
                    },
                }
            )
        )
        for raw in ws:
            request = json.loads(raw)
            method, params = request["method"], request["params"]
            self.calls.append((connection, method, params))
            number = next(self.numbers)
            if method == "image.attach_bytes" and self.image_refusal is not None:
                ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": {"code": self.image_refusal, "message": "image refused"},
                        }
                    )
                )
                continue
            if method == "image.attach_bytes":
                # tui_gateway's reply: the image is queued for the next turn.
                staged = f"/synthetic/images/upload_{number}.{params['ext']}"
                result = {"attached": True, "path": staged, "count": 1, "remainder": ""}
                if self.image_reply is not None:
                    result = self.image_reply
            elif method == "image.detach":
                result = {"detached": True, "count": 0}
            elif method == "session.create":
                live = f"{params['profile']}-live-{number}"
                self.profiles[live] = params["profile"]
                result = {"session_id": live, "stored_session_id": f"{params['profile']}-stored"}
            elif method == "session.resume":
                live = f"{params['profile']}-resumed-{number}"
                self.profiles[live] = params["profile"]
                result = {"session_id": live, "resumed": params["session_id"], "hydrating": False}
            else:
                result = {"status": "streaming"}
            ws.send(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            if method == "prompt.submit":
                profile = self.profiles[params["session_id"]]
                ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": {
                                "type": "message.complete",
                                "session_id": params["session_id"],
                                "payload": {"text": f"Responde {profile}", "status": "complete"},
                            },
                        }
                    )
                )

    def created_profiles(self) -> list[str]:
        return [params["profile"] for _, method, params in self.calls if method == "session.create"]


def hub(settings, store, speech):
    return TestClient(
        create_app(settings, store=store, speech=speech, now=lambda: 1001),
        base_url="https://localhost",
    )


def send(ws, text, **params):
    turn = str(uuid4())
    ws.send_json({"v": 1, "id": turn, "method": "chat.send", "params": {"text": text, **params}})
    return turn


def until(ws, turn, *events):
    """The first frame of this turn that is one of `events`."""
    while True:
        frame = ws.receive_json()
        if frame["id"] == turn and frame["event"] in events:
            return frame


def answered(ws, turn) -> str:
    """The session of a turn the synthetic Hermes answered; its text is checked by the caller."""
    completed = until(ws, turn, "chat.completed", "error")
    assert completed["event"] == "chat.completed", completed
    return completed["data"]


def socket(http, paired):
    return http.websocket_connect(
        "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
    )


# Configuration ---------------------------------------------------------------


def test_a_0_9_0_configuration_loads_as_the_one_implicit_agent(tmp_path, monkeypatch):
    monkeypatch.delenv("ORBIT_HUB_PROFILE", raising=False)
    monkeypatch.delenv("ORBIT_HUB_ASSISTANT_NAME", raising=False)
    settings = Settings.load(config_file(tmp_path, 'profile = "atlas"\nassistant_name = "Atlas"\n'))
    assert (settings.profile, settings.assistant_name) == ("atlas", "Atlas")
    assert settings.agents is None
    assert settings.default_agent is None


def test_agents_load_from_toml(tmp_path):
    settings = Settings.load(config_file(tmp_path, 'default_agent = "lyra"\n', TWO_AGENTS))
    assert [entry["id"] for entry in settings.agents] == ["deimos", "lyra"]
    assert settings.default_agent == "lyra"


@pytest.mark.parametrize(
    "entries,head,refusal",
    [
        (
            (TWO_AGENTS[0], {**TWO_AGENTS[1], "id": "deimos"}),
            "",
            r'\[\[agents\]\] entry 2 \(id "deimos"\): duplicate id; entry 1 already uses it',
        ),
        (
            (TWO_AGENTS[0], {**TWO_AGENTS[1], "name": "DÉIMOS"}),
            "",
            (
                r'\[\[agents\]\] entry 2 \(id "lyra"\): duplicate name "DÉIMOS": entry 1 \("Deimos"\)'
                r" is said the same, ignoring case and accents"
            ),
        ),
        (
            ({**TWO_AGENTS[0], "kind": "telepathy"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): unknown kind "telepathy"; known kinds: acp, hermes, openai',
        ),
        (
            ({"id": "deimos", "name": "Deimos", "kind": "hermes"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): kind "hermes" needs profile',
        ),
        (
            ({**TWO_AGENTS[0], "model": "example-model"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): kind "hermes" does not take model',
        ),
        (
            ({**TWO_AGENTS[0], "profile": "a/b"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): profile "a/b" must be 1 to 64 characters without /',
        ),
        (
            ({**TWO_AGENTS[0], "id": "Deimos"},),
            "",
            r'\[\[agents\]\] entry 1: id "Deimos" must be 1 to 32 characters of a-z, 0-9 and -',
        ),
        (
            ({"name": "Deimos", "kind": "hermes", "profile": "deimos-profile"},),
            "",
            r"\[\[agents\]\] entry 1: missing id",
        ),
        (
            ({"id": "deimos", "kind": "hermes", "profile": "deimos-profile"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): missing name',
        ),
        (
            ({"id": "deimos", "name": "Deimos", "profile": "deimos-profile"},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): missing kind',
        ),
        (
            ({**TWO_AGENTS[0], "name": "D" * 49},),
            "",
            r'\[\[agents\]\] entry 1 \(id "deimos"\): name is too long: 49 characters; maximum 48',
        ),
        (
            TWO_AGENTS,
            'default_agent = "zeta"\n',
            r'default_agent "zeta" is not an agent; the agents are deimos, lyra',
        ),
        (
            TWO_AGENTS,
            'profile = "deimos-profile"\n',
            r"profile configures the implicit agent; with \[\[agents\]\] each entry has its own",
        ),
        (
            TWO_AGENTS,
            'assistant_name = "Deimos"\n',
            r"assistant_name configures the implicit agent; with \[\[agents\]\] each entry",
        ),
    ],
    ids=[
        "duplicate-id",
        "duplicate-spoken-name",
        "unknown-kind",
        "missing-kind-setting",
        "unknown-kind-setting",
        "invalid-profile",
        "invalid-id",
        "missing-id",
        "missing-name",
        "missing-kind",
        "long-name",
        "unknown-default",
        "implicit-profile-beside-agents",
        "implicit-name-beside-agents",
    ],
)
def test_invalid_agents_refuse_startup_naming_the_entry_and_the_cause(
    tmp_path, monkeypatch, entries, head, refusal
):
    monkeypatch.delenv("ORBIT_HUB_PROFILE", raising=False)
    monkeypatch.delenv("ORBIT_HUB_ASSISTANT_NAME", raising=False)
    path = config_file(tmp_path, head, entries)
    with pytest.raises(ValueError, match=refusal):
        Settings.load(path)


def test_an_empty_agent_list_is_refused(tmp_path):
    path = config_file(tmp_path, "agents = []\n")
    with pytest.raises(ValueError, match=r"agents is empty"):
        Settings.load(path)


def test_the_environment_overrides_of_the_implicit_agent_are_refused_beside_agents(
    tmp_path, monkeypatch
):
    path = config_file(tmp_path, "", TWO_AGENTS)
    monkeypatch.setenv("ORBIT_HUB_PROFILE", "deimos-profile")
    with pytest.raises(ValueError, match=r"ORBIT_HUB_PROFILE configures the implicit agent"):
        Settings.load(path)


# ready ---------------------------------------------------------------------


def test_ready_lists_the_implicit_agent_for_a_0_9_0_configuration(environment):
    settings, store, adapter = environment
    settings = replace(settings, assistant_name="Atlas")
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ready = ws.receive_json()
    assert ready["event"] == "ready"
    assert ready["data"]["assistant_name"] == "Atlas"
    assert ready["data"]["default_agent_id"] == "default"
    assert ready["data"]["agents"] == [
        {"id": "default", "name": "Atlas", "kind": "hermes", "capabilities": HERMES_CAPABILITIES}
    ]


def test_ready_lists_every_agent_and_names_the_default(environment):
    settings, store, adapter = environment
    settings = replace(settings, agents=TWO_AGENTS, default_agent="lyra")
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ready = ws.receive_json()["data"]
    assert ready["assistant_name"] == "Lyra"
    assert ready["default_agent_id"] == "lyra"
    assert ready["agents"] == [
        {"id": "deimos", "name": "Deimos", "kind": "hermes", "capabilities": HERMES_CAPABILITIES},
        {"id": "lyra", "name": "Lyra", "kind": "hermes", "capabilities": HERMES_CAPABILITIES},
    ]


def test_ready_agents_have_the_shape_the_mac_decodes(environment):
    """The Mac decodes `ready` with the types that write the shared fixture
    (`contracts/fixtures/hub/events/ready.json`, from `crates/orbit-core`); an
    agent entry of another shape makes it refuse the whole frame. Each side
    once tested only its own shape, and they disagreed on `capabilities`."""
    fixture = Path(__file__).resolve().parents[2] / "contracts/fixtures/hub/events/ready.json"
    shared = json.loads(fixture.read_text(encoding="utf-8"))["data"]
    settings, store, adapter = environment
    settings = replace(settings, agents=TWO_AGENTS, default_agent="lyra")
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        ready = ws.receive_json()["data"]
    for agent in ready["agents"]:
        assert set(agent) == set(shared["agents"][0])
        assert all(isinstance(agent[key], str) for key in ("id", "name", "kind"))
        assert isinstance(agent["capabilities"], list)
        assert set(agent["capabilities"]) <= {"images", "voice", "resume"}
    assert isinstance(ready["default_agent_id"], str)


# Turns ---------------------------------------------------------------------


def test_the_implicit_agent_is_the_configured_profile(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings, profile="atlas")
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            turn = send(ws, "Hola")
            assert answered(ws, turn)["text"] == "Responde atlas"
            rows = http.get(
                "/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]}
            ).json()["sessions"]
    assert hermes.created_profiles() == ["atlas"]
    assert [row["agent_id"] for row in rows] == ["default"]


def test_two_hermes_agents_answer_from_their_own_profiles(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings, agents=TWO_AGENTS)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            first = send(ws, "Hola")
            to_default = answered(ws, first)
            second = send(ws, "Hola", agent_id="lyra")
            to_lyra = answered(ws, second)
            # A session's next turn goes to its own agent without naming it.
            third = send(ws, "¿Sigues ahí?", session_id=to_lyra["session_id"])
            again = answered(ws, third)
            listed = http.get("/v1/sessions", headers=headers).json()["sessions"]
            read = http.get(
                f"/v1/sessions/{to_lyra['session_id']}/messages", headers=headers
            ).json()
    assert to_default["text"] == "Responde deimos-profile"
    assert to_lyra["text"] == "Responde lyra-profile"
    assert again["text"] == "Responde lyra-profile"
    assert again["session_id"] == to_lyra["session_id"]
    assert hermes.created_profiles() == ["deimos-profile", "lyra-profile"]
    # One gateway connection per agent: the default's prompt never rides Lyra's.
    by_connection = {}
    for connection, method, params in hermes.calls:
        if method == "prompt.submit":
            by_connection.setdefault(connection, set()).add(params["session_id"].split("-live")[0])
    assert sorted(by_connection.values(), key=sorted) == [{"deimos-profile"}, {"lyra-profile"}]
    assert {row["id"]: row["agent_id"] for row in listed} == {
        to_default["session_id"]: "deimos",
        to_lyra["session_id"]: "lyra",
    }
    assert read["agent_id"] == "lyra"


def test_a_session_resumes_with_its_own_agent_after_a_restart(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings, agents=TWO_AGENTS)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            session = answered(ws, send(ws, "Hola", agent_id="lyra"))["session_id"]
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            again = answered(ws, send(ws, "Seguimos", session_id=session))
    assert again["text"] == "Responde lyra-profile"
    [resume] = [params for _, method, params in hermes.calls if method == "session.resume"]
    assert (resume["profile"], resume["session_id"]) == ("lyra-profile", "lyra-profile-stored")


def test_a_session_refuses_another_agent_with_both_ids(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings, agents=TWO_AGENTS)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            session = answered(ws, send(ws, "Hola", agent_id="lyra"))["session_id"]
            refused = until(ws, send(ws, "Hola", session_id=session, agent_id="deimos"), "error")
    assert refused["data"]["code"] == "agent_mismatch"
    assert refused["data"]["refusal"] == {
        "code": "agent_mismatch",
        "session_agent_id": "lyra",
        "agent_id": "deimos",
        "retry": {"kind": "after_fix"},
    }
    assert "Lyra" in refused["data"]["message"]
    # Nothing reached Hermes for the refused turn.
    assert [method for _, method, _ in hermes.calls] == ["session.create", "prompt.submit"]


@pytest.mark.parametrize("with_session", [False, True])
def test_an_unknown_agent_is_refused_with_the_agents_that_exist(
    environment, tmp_path, with_session
):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings, agents=TWO_AGENTS)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            extra = {}
            if with_session:
                extra["session_id"] = answered(ws, send(ws, "Hola"))["session_id"]
            refused = until(ws, send(ws, "Hola", agent_id="zeta", **extra), "error")
    assert refused["data"]["code"] == "agent_not_found"
    assert refused["data"]["refusal"] == {
        "code": "agent_not_found",
        "agent_id": "zeta",
        "agent_ids": "deimos,lyra",
        "retry": {"kind": "after_fix"},
    }
    assert hermes.created_profiles() == (["deimos-profile"] if with_session else [])


# Migration -------------------------------------------------------------------


def test_sessions_stored_before_agents_belong_to_the_default_agent(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    owner, headers = paired["device_id"], {"Authorization": "Bearer " + paired["token"]}
    store.close()
    # The sessions table of 0.9.0: no agent column, one stored conversation.
    database = sqlite3.connect(settings.data_dir / "hub.sqlite3")
    columns = {row[1] for row in database.execute("PRAGMA table_info(sessions)")}
    if "agent_id" in columns:
        database.execute("ALTER TABLE sessions DROP COLUMN agent_id")
    legacy = str(uuid4())
    database.execute(
        "INSERT INTO sessions VALUES (?,?,?,?,?,?,?)",
        (legacy, owner, "old-live", "deimos-profile-stored", "old-epoch", "completed", 900.0),
    )
    database.commit()
    database.close()

    migrated = Store(settings.data_dir, settings.key_file)
    try:
        with SyntheticHermes(tmp_path) as hermes:
            configured = hermes.settings(settings, agents=TWO_AGENTS)
            with hub(configured, migrated, adapter) as http, socket(http, paired) as ws:
                [row] = http.get("/v1/sessions", headers=headers).json()["sessions"]
                read = http.get(f"/v1/sessions/{legacy}/messages", headers=headers).json()
                refused = until(ws, send(ws, "Hola", session_id=legacy, agent_id="lyra"), "error")
                again = answered(ws, send(ws, "Seguimos", session_id=legacy))
    finally:
        migrated.close()
    assert row["agent_id"] == "deimos"
    assert read["agent_id"] == "deimos"
    assert refused["data"]["refusal"]["session_agent_id"] == "deimos"
    assert again["text"] == "Responde deimos-profile"
    [resume] = [params for _, method, params in hermes.calls if method == "session.resume"]
    assert resume["profile"] == "deimos-profile"
