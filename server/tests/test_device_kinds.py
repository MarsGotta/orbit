"""What each paired device is, what it handles, and the hub's own id.

At pairing a device may say what it is (`mac` or `phone`) and which groups
of orders it handles, and send the id of its Orbit installation. A device
that says nothing is a Mac that handles everything, as every Orbit before
this one. A request for a group the device does not handle is refused by the
hub, naming the device and what was asked, and never reaches the device.

The hub keeps an id of its own, made the first time it opens its database,
and says it in `/health` and in `ready`. Every name here is invented.
"""

import asyncio
import uuid
from dataclasses import replace

import httpx
from conftest import answer_bridge_once as answer_once
from conftest import bridge_body as body
from conftest import bridge_signed as signed
from conftest import connect_bridge as connect
from conftest import device
from support_app import client

from orbit_hub.bridge_route import DEVICES_PATH, PATH
from orbit_hub.storage import Store, initialize

EVERY_GROUP = {
    "system_audio",
    "screen",
    "selected_text",
    "folders",
    "calendar",
    "call_detection",
    "shortcuts",
    "control",
}
INSTALLATION = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"


def pair(store, name, now=1001, **declared):
    """Pair one device through the store, as `/v1/pair` does."""
    return store.devices.pair(store.devices.create_code(now - 1), name, now, **declared)


async def inventory(app):
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost"
    ) as http:
        response = await http.get(DEVICES_PATH, headers=signed(b"", path=DEVICES_PATH))
    assert response.status_code == 200, response.text
    return {row["id"]: row for row in response.json()["devices"]}


def test_pairing_accepts_what_a_device_says_it_is_and_refuses_an_unknown_kind(environment):
    _, store, _ = environment
    with client(environment) as http:
        code = store.devices.create_code(1000)
        phone = http.post(
            "/v1/pair",
            json={
                "code": code,
                "name": "Teléfono Sirio",
                "kind": "phone",
                "capabilities": ["folders", "calendar"],
                "installation_id": INSTALLATION,
            },
        )
        assert phone.status_code == 200, phone.text
        assert set(phone.json()) == {"device_id", "token"}

        code = store.devices.create_code(1000)
        unknown = http.post(
            "/v1/pair", json={"code": code, "name": "Tostadora Vega", "kind": "toaster"}
        )
        assert unknown.status_code == 400
        assert unknown.json()["code"] == "invalid_request"
        # The refused request did not spend the code.
        again = http.post("/v1/pair", json={"code": code, "name": "Tostadora Vega"})
        assert again.status_code == 200


async def test_a_device_that_says_nothing_is_a_mac_that_handles_everything(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        rows = await inventory(app)
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body("screenshot", owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as http:
            mac = asyncio.create_task(answer_once(service, {"refused": None, "result": {}}))
            response = await http.post(PATH, content=payload, headers=signed(payload))
            entry = await mac

    listed = rows[owner["device_id"]]
    assert listed["kind"] == "mac"
    assert set(listed["capabilities"]) == EVERY_GROUP
    assert entry.capability == "screenshot"
    assert response.status_code == 200, response.text


async def test_a_phone_without_a_screen_refuses_a_screenshot_naming_the_device_and_the_request(
    bridged,
):
    app, store, adapter, _owner = bridged
    phone = pair(store, "Teléfono Sirio", kind="phone", capabilities=["folders", "calendar"])
    async with app.router.lifespan_context(app):
        runtime = await connect(app, store, adapter, phone)
        service = app.state.device_bridge
        payload = body("screenshot", phone["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as http:
            response = await http.post(PATH, content=payload, headers=signed(payload))
        # Nothing was handed to the phone: no question open, none queued.
        assert not service.bridge.pending and not service.waiters
        queued = [
            listener.get_nowait() for listener in runtime.listeners for _ in range(listener.qsize())
        ]
        assert all(frame.get("event") != "device.request" for frame in queued)

    assert response.status_code == 403, response.text
    refused = response.json()
    assert refused["code"] == "device_capability_missing"
    numbers = {k: v for k, v in refused["refusal"].items() if k not in {"code", "retry"}}
    assert numbers == {
        "device_id": phone["device_id"],
        "kind": "phone",
        "capability": "screenshot",
        "group": "screen",
    }
    assert refused["refusal"]["retry"] == {"kind": "never"}
    message = refused["message"]
    assert "that device" in message.lower()
    assert "screen" in message and "screenshot" in message
    assert "Mac" not in message


async def test_the_inventory_says_what_each_device_is_and_handles(bridged):
    app, store, _adapter, owner = bridged
    phone = pair(store, "Teléfono Sirio", kind="phone", capabilities=["folders", "calendar"])
    quiet = pair(store, "Teléfono Lira", kind="phone", capabilities=[])
    async with app.router.lifespan_context(app):
        rows = await inventory(app)

    assert rows[owner["device_id"]]["kind"] == "mac"
    assert set(rows[owner["device_id"]]["capabilities"]) == EVERY_GROUP
    assert rows[phone["device_id"]]["kind"] == "phone"
    assert sorted(rows[phone["device_id"]]["capabilities"]) == ["calendar", "folders"]
    # An empty list is a device that handles nothing, not one that said nothing.
    assert rows[quiet["device_id"]]["capabilities"] == []
    assert all("installation_id" not in row for row in rows.values())


async def test_a_device_row_written_by_todays_hub_reads_as_a_mac_that_handles_everything(
    bridged,
):
    # Today's hub writes devices with these columns only. A row it wrote, in a
    # database it shares with the new code, is a Mac that handles everything.
    app, store, _adapter, _owner = bridged
    device_id = str(uuid.uuid4())
    with store.transaction() as db:
        db.execute(
            "INSERT INTO devices(id,name,token_hash,created,last_seen) VALUES (?,?,?,?,?)",
            (
                device_id,
                store.encrypt(b"Mac de pruebas", f"device:{device_id}"),
                "0" * 64,
                900.0,
                900.0,
            ),
        )
    async with app.router.lifespan_context(app):
        rows = await inventory(app)

    assert rows[device_id]["kind"] == "mac"
    assert set(rows[device_id]["capabilities"]) == EVERY_GROUP


def test_the_installation_id_is_kept_when_pairing(environment):
    _, store, _ = environment
    with client(environment) as http:
        sent = http.post(
            "/v1/pair",
            json={
                "code": store.devices.create_code(1000),
                "name": "Teléfono Sirio",
                "installation_id": INSTALLATION,
            },
        )
        silent = http.post(
            "/v1/pair",
            json={"code": store.devices.create_code(1000), "name": "Mac de pruebas"},
        )
        malformed = http.post(
            "/v1/pair",
            json={
                "code": store.devices.create_code(1000),
                "name": "Mac de pruebas",
                "installation_id": "not-an-installation",
            },
        )
    assert sent.status_code == silent.status_code == 200
    assert malformed.status_code == 400 and malformed.json()["code"] == "invalid_request"
    with store.reading() as db:
        kept = dict(db.execute("SELECT id, installation_id FROM devices").fetchall())
    assert kept[sent.json()["device_id"]] == INSTALLATION
    assert kept[silent.json()["device_id"]] is None


def hub_id_of(environment) -> str:
    with client(environment) as http:
        health = http.get("/health")
    assert health.status_code == 200
    answer = health.json()
    assert answer["ok"] is True
    return answer["hub_id"]


def test_the_hub_id_is_stable_across_restarts_and_shown_in_health(environment, tmp_path):
    settings, store, adapter = environment
    first = hub_id_of(environment)
    assert str(uuid.UUID(first)) == first
    store.close()
    reopened = Store(settings.data_dir, settings.key_file)
    try:
        assert hub_id_of((settings, reopened, adapter)) == first
    finally:
        reopened.close()

    other_data, other_key = tmp_path / "other-data", tmp_path / "other-key"
    initialize(other_data, other_key)
    other = Store(other_data, other_key)
    try:
        other_settings = replace(settings, data_dir=other_data, key_file=other_key)
        assert hub_id_of((other_settings, other, adapter)) != first
    finally:
        other.close()


def test_ready_says_the_hub_id(environment):
    _, store, _ = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with client(environment) as http:
        health = http.get("/health").json()
        with http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
            ready = ws.receive_json()
    assert ready["event"] == "ready"
    assert ready["data"]["hub_id"] == health["hub_id"]
