"""The bridge hands the assistant the bytes the Mac uploaded over HTTP.

No real websocket and no real Mac: a fake Mac picks the question up where a
real one would read it, uploads the capture through the captures route with the
device's pairing and answers with its identifier through the same future the
websocket handler resolves. The assistant's signature is the one
test_bridge_route uses: the real Authenticator, no shortcuts.
"""

import asyncio
import base64
import hashlib
import json
from dataclasses import replace
from uuid import uuid4

import httpx
import pytest
from conftest import device

from orbit_hub.app import create_app
from orbit_hub.bridge_route import PATH, PROTOCOL, BridgeConfiguration
from orbit_hub.private_auth import sign_request
from orbit_hub.runtime import DeviceRuntime

KEY = bytes(range(32))
KEY_ID = "bridge-capture-key"
NOW_MS = 2_000_000
PNG = b"\x89PNG\r\n\x1a\n" + b"captura sintetica del techo" * 32


class Clock:
    """The hub's injected clock, in seconds, moved by hand."""

    def __init__(self, seconds):
        self.seconds = seconds

    def __call__(self):
        return self.seconds

    def advance(self, seconds):
        self.seconds += seconds


def body(capability="screenshot", device_id="unset", **parameters):
    return json.dumps(
        {
            "protocol": PROTOCOL,
            "device_id": device_id,
            "capability": capability,
            "parameters": parameters,
        },
        separators=(",", ":"),
    ).encode()


def signed(payload, *, issued):
    nonce = str(uuid4())
    return {
        "content-type": "application/json",
        **sign_request(PROTOCOL, PATH, KEY_ID, KEY, payload, issued_at_ms=issued, nonce=nonce),
    }


@pytest.fixture
def bridged(environment, tmp_path):
    settings, store, adapter = environment
    path = tmp_path / "bridge-key"
    path.write_bytes(KEY)
    path.chmod(0o600)
    settings = replace(
        settings,
        device_bridge=BridgeConfiguration(key_id=KEY_ID, credential_file=str(path)),
    )
    owner = device(store)
    clock = Clock(NOW_MS / 1000)
    app = create_app(settings, store=store, adapter=adapter, now=clock)
    return app, store, adapter, owner, clock


async def connect(app, store, adapter, owner, clock):
    runtime = DeviceRuntime(owner["device_id"], store, adapter, clock)
    await runtime.connect()
    runtime.subscribe()
    app.state.device_bridge.runtimes[owner["device_id"]] = runtime
    return runtime


async def next_question(service):
    """Wait, as a connected Mac would, for the question to reach the bridge."""
    for _ in range(200):
        pending = list(service.bridge.pending.values())
        if pending:
            return pending[0]
        await asyncio.sleep(0.01)
    raise AssertionError("the question never reached the Mac")


def resolve(service, entry, reply):
    """Answer through the same path as the websocket handler."""
    claimed = service.bridge.resolve(entry.request_id, entry.device)
    waiter = service.waiters.get(entry.request_id)
    if claimed is not None and waiter is not None and not waiter.done():
        waiter.set_result(reply)


async def upload_capture(http, owner, request_id, capture_id, body=PNG):
    """The PUT the daemon makes: the device's bearer, type and digest."""
    response = await http.put(
        f"/v1/captures/{capture_id}",
        content=body,
        headers={
            "Authorization": "Bearer " + owner["token"],
            "Content-Type": "image/png",
            "X-Orbit-Request": request_id,
            "X-Orbit-Sha256": hashlib.sha256(body).hexdigest(),
        },
    )
    assert response.status_code == 200, response.text
    return response


def capture_reply(capture_id):
    return {
        "refused": None,
        "result": {
            "capture_id": capture_id,
            "application": "TextEdit",
            "title": "Presupuesto 2027",
            "width": 2,
            "height": 1,
            "mime_type": "image/png",
            "sha256": hashlib.sha256(PNG).hexdigest(),
        },
    }


async def test_a_replied_capture_is_delivered_as_exact_bytes_without_the_id(bridged):
    app, store, adapter, owner, clock = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner, clock)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        capture_id = str(uuid4())
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:

            async def mac():
                entry = await next_question(service)
                await upload_capture(client, owner, entry.request_id, capture_id)
                resolve(service, entry, capture_reply(capture_id))

            asking = asyncio.create_task(
                client.post(
                    PATH, content=payload, headers=signed(payload, issued=int(clock.seconds * 1000))
                )
            )
            await mac()
            response = await asking
        assert response.status_code == 200, response.text
        result = response.json()["result"]
        # The assistant still sees the original shape: the bytes, not a pointer.
        assert result["data_base64"] == base64.b64encode(PNG).decode()
        assert "capture_id" not in result
        assert result["application"] == "TextEdit"
        assert result["title"] == "Presupuesto 2027"
        assert result["sha256"] == hashlib.sha256(PNG).hexdigest()
        assert not service.bridge.pending and not service.waiters


async def test_an_unknown_capture_id_is_refused_by_name(bridged):
    app, store, adapter, owner, clock = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner, clock)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:

            async def mac():
                entry = await next_question(service)
                # The Mac answers with an identifier nobody uploaded.
                resolve(service, entry, capture_reply(str(uuid4())))

            asking = asyncio.create_task(
                client.post(
                    PATH, content=payload, headers=signed(payload, issued=int(clock.seconds * 1000))
                )
            )
            await mac()
            response = await asking
        assert response.status_code == 403, response.text
        assert response.json()["state"] == "refused"
        assert "capture_missing" in response.json()["notice"]
        assert response.json()["refusal"]["code"] == "capture_missing"
        assert "cause" not in response.json()["refusal"]


async def test_the_capture_is_consumed_on_delivery_and_cannot_answer_twice(bridged):
    app, store, adapter, owner, clock = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner, clock)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        capture_id = str(uuid4())
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:

            async def mac():
                entry = await next_question(service)
                await upload_capture(client, owner, entry.request_id, capture_id)
                resolve(service, entry, capture_reply(capture_id))

            asking = asyncio.create_task(
                client.post(
                    PATH, content=payload, headers=signed(payload, issued=int(clock.seconds * 1000))
                )
            )
            await mac()
            first = await asking
        assert first.status_code == 200, first.text

        # The capture was deleted when handed over: the same answer, or the same
        # capture resent for another question, has no bytes left to carry.
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:

            async def mac():
                entry = await next_question(service)
                resolve(service, entry, capture_reply(capture_id))

            asking = asyncio.create_task(
                client.post(
                    PATH, content=payload, headers=signed(payload, issued=int(clock.seconds * 1000))
                )
            )
            await mac()
            second = await asking
        assert second.status_code == 403, second.text
        assert "capture_missing" in second.json()["notice"]


async def test_an_expired_capture_is_refused_like_a_missing_one(bridged):
    app, store, adapter, owner, clock = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner, clock)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        capture_id = str(uuid4())
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            asking = asyncio.create_task(
                client.post(
                    PATH,
                    content=payload,
                    headers=signed(payload, issued=int(clock.seconds * 1000)),
                )
            )
            entry = await next_question(service)
            await upload_capture(client, owner, entry.request_id, capture_id)
            # Five minutes of TTL: the capture expires before the Mac answers.
            clock.advance(301)
            resolve(service, entry, capture_reply(capture_id))
            response = await asking
        assert response.status_code == 403, response.text
        assert "capture_missing" in response.json()["notice"]
