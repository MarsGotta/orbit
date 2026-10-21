"""The private route an assistant uses to ask a Mac something.

No real device and no websocket: a fake Mac reads the pushed question off the
same listener queue a real one would and answers through the same future the
websocket handler resolves.
"""

import asyncio
import json
from uuid import uuid4

import httpx
import pytest
from conftest import BRIDGE_KEY_ID as KEY_ID
from conftest import BRIDGE_NOW_MS as NOW_MS
from conftest import answer_bridge_once as answer_once
from conftest import bridge_body as body
from conftest import bridge_signed as signed
from conftest import connect_bridge as connect

from orbit_hub.app import create_app
from orbit_hub.bridge_route import DEVICES_PATH, PATH, BridgeConfiguration
from orbit_hub.runtime import DeviceRuntime


async def test_bridge_request_envelope_limit_names_bytes_and_ceiling(bridged):
    from orbit_hub.bridge_route import REQUEST_BYTES

    app, _store, _adapter, owner = bridged
    payload = body("perform_action", owner["device_id"], action={"data": ""})
    request = json.loads(payload)
    request["parameters"]["action"]["data"] = "x" * (REQUEST_BYTES - len(payload))
    payload = json.dumps(request, separators=(",", ":")).encode()
    assert len(payload) == REQUEST_BYTES
    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client,
    ):
        admitted = await client.post(PATH, content=payload, headers=signed(payload))
        oversized = payload + b" "
        refused = await client.post(PATH, content=oversized, headers=signed(oversized))
    assert admitted.status_code == 404
    assert refused.status_code == 400
    assert refused.json()["refusal"]["bytes"] == REQUEST_BYTES + 1
    assert refused.json()["refusal"]["max_bytes"] == REQUEST_BYTES


async def test_global_bridge_waiter_refusal_reaches_caller_with_count(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        for index in range(4):
            service.bridge.open(f"synthetic-{index}", owner["device_id"], "screenshot", 0.0)
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=signed(payload))
        assert response.status_code == 429
        assert response.json()["refusal"]["code"] == "too_many_requests"
        assert response.json()["refusal"]["count"] == 5
        assert response.json()["refusal"]["max"] == 4


@pytest.mark.parametrize(
    "parameters,field",
    [
        ({"application": 5}, "application"),
        ({"title": {}}, "title"),
        ({"application": None}, "application"),
    ],
)
def test_wrong_screenshot_filter_is_named_by_mcp(parameters, field):
    from orbit_mcp.hub import HubClientError, validate_parameters

    with pytest.raises(HubClientError) as error:
        validate_parameters("screenshot", parameters)
    refusal = error.value.detail["refusal"]
    assert refusal["code"] == "invalid_parameters"
    assert refusal["field"] == field
    assert refusal["expected"] == "texto"


@pytest.mark.parametrize(
    "capability,parameters,field,expected",
    [
        ("calendar_read", {"start_ms": "hoy", "end_ms": 2}, "start_ms", "entero"),
        ("control_tools", {"after_index": "uno"}, "after_index", "entero"),
        ("calendar_read", {"start_ms": 2, "end_ms": 1}, "end_ms", "posterior a start_ms"),
        (
            "calendar_read",
            {"start_ms": 1, "end_ms": 2, "read_id": "r"},
            "after_ms",
            "entero en milisegundos",
        ),
        (
            "calendar_read",
            {"start_ms": 1, "end_ms": 2, "read_id": "r", "after_ms": 1},
            "after_index",
            "entero no negativo",
        ),
        ("perform_action", {"action": {"tool": 7, "arguments": {}}}, "action.tool", "texto"),
        (
            "perform_action",
            {"action": {"tool": "x", "arguments": []}},
            "action.arguments",
            "objeto",
        ),
        ("perform_action", {"action": {}}, "action.tool", "texto"),
        ("calendar_read", {"end_ms": 2}, "start_ms", "entero"),
        ("control_tools", {"after_index": -1}, "after_index", "entero no negativo"),
    ],
)
def test_invalid_bridge_parameters_name_the_field_and_expected_type(
    capability, parameters, field, expected
):
    from orbit_mcp.hub import HubClientError, validate_parameters

    with pytest.raises(HubClientError) as error:
        validate_parameters(capability, parameters)
    refusal = error.value.detail["refusal"]
    assert refusal["code"] == "invalid_parameters"
    assert refusal["field"] == field
    assert refusal["expected"] == expected


async def test_parameters_that_are_not_an_object_are_named(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        request = json.loads(body("screen_sources", owner["device_id"]))
        request["parameters"] = 5
        payload = json.dumps(request, separators=(",", ":")).encode()
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=signed(payload))
        assert response.status_code == 400
        refusal = response.json()["refusal"]
        assert refusal["code"] == "invalid_request"
        assert refusal["field"] == "parameters"
        assert refusal["expected"] == "objeto"
        assert not app.state.device_bridge.bridge.pending


async def test_a_signed_question_reaches_the_mac_and_its_answer_comes_back(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        clock_calls = 0

        def activity_clock():
            nonlocal clock_calls
            clock_calls += 1
            return NOW_MS if clock_calls == 1 else 1_020_000

        service.timestamp = activity_clock
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(
                answer_once(service, {"refused": None, "result": {"windows": 3}})
            )
            response = await client.post(PATH, content=payload, headers=signed(payload))
            entry = await mac
            devices = await client.get(
                DEVICES_PATH,
                headers=signed(b"", issued=1_020_000, path=DEVICES_PATH),
            )
        assert response.status_code == 200
        assert response.json()["result"] == {"windows": 3}
        current = next(row for row in devices.json()["devices"] if row["id"] == owner["device_id"])
        assert current["last_seen"] == 1_020.0
        # The id was minted here, never taken from the caller: a caller choosing
        # its own could reuse one while its question is still open.
        assert entry.request_id not in payload.decode()
        assert not service.bridge.pending and not service.waiters


async def test_a_refusal_from_the_mac_is_not_reported_as_an_error(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(
                answer_once(
                    service,
                    {
                        "refusal": {
                            "code": "permission_denied",
                            "permission": "screen_recording",
                            "retry": {"kind": "after_fix"},
                        }
                    },
                )
            )
            response = await client.post(PATH, content=payload, headers=signed(payload))
            await mac
        assert response.status_code == 403
        assert response.json()["state"] == "refused"
        assert response.json()["refusal"] == {
            "code": "permission_denied",
            "permission": "screen_recording",
            "retry": {"kind": "after_fix"},
        }


async def test_named_action_refusal_survives_in_the_pending_operation(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body(
            "perform_action",
            owner["device_id"],
            action={"tool": "fixture_action", "arguments": {}},
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(
                answer_once(
                    service,
                    {
                        "refusal": {
                            "code": "permission_denied",
                            "permission": "accessibility",
                            "retry": {"kind": "after_fix"},
                        },
                    },
                )
            )
            response = await client.post(PATH, content=payload, headers=signed(payload))
            await mac
            operation_id = response.json()["operation_id"]
            for _ in range(100):
                stored = store.operation(owner["device_id"], operation_id)
                if stored is not None and stored["status"] == "completed":
                    break
                await asyncio.sleep(0.01)
    assert response.status_code == 202
    assert response.json()["state"] == "pending"
    assert stored["response"]["outcome"] == "failed"
    assert stored["response"]["code"] == "permission_denied"


async def test_silence_from_the_mac_is_named_apart_from_a_refusal(bridged, monkeypatch):
    # A caller that cannot tell "the Mac said no" from "the Mac said nothing"
    # will retry a refusal forever.
    import orbit_hub.bridge_route as module

    monkeypatch.setattr(module, "REQUEST_TIMEOUT_SECONDS", 0.05)
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        ticks = iter((10.0, 10.0, 20.0, 20.075))
        service.monotonic = lambda: next(ticks)
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=signed(payload))
        assert response.status_code == 504
        assert response.json()["code"] == "no_answer"
        assert response.json()["refusal"] == {
            "code": "timeout",
            "operation": "interaction",
            "waited_ms": 75,
            "deadline_ms": 50,
            "retry": {"kind": "check_outcome"},
        }
        # And the abandoned question does not keep a slot for ever.
        assert not service.bridge.pending and not service.waiters


async def test_an_unsigned_or_tampered_request_is_refused(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        payload = body(device_id=owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            bare = await client.post(
                PATH, content=payload, headers={"content-type": "application/json"}
            )
            assert bare.status_code == 401
            wrong_key = await client.post(
                PATH, content=payload, headers=signed(payload, key=bytes(32))
            )
            assert wrong_key.status_code == 401
            # Signed correctly, then the body swapped: the signature covers the
            # exact bytes, so this must not be admitted either.
            headers = signed(payload)
            swapped = await client.post(
                PATH, content=body(device_id=owner["device_id"], token="otro"), headers=headers
            )
            assert swapped.status_code == 401


async def test_signed_device_inventory_is_decrypted_live_and_excludes_revoked(bridged):
    app, store, adapter, connected = bridged
    disconnected = store.pair(store.create_code(700), "Mac de salón", 701)
    revoked = store.pair(store.create_code(800), "Mac retirado", 801)
    store.mark_device_seen(disconnected["device_id"], 850)
    store.revoke(revoked["device_id"])

    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, connected)
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            unsigned = await client.get(DEVICES_PATH)
            response = await client.get(
                DEVICES_PATH,
                headers=signed(b"", path=DEVICES_PATH),
            )

    assert unsigned.status_code == 401
    assert response.status_code == 200
    assert response.headers["cache-control"] == "no-store"
    rows = {row["id"]: row for row in response.json()["devices"]}
    assert rows == {
        disconnected["device_id"]: {
            "id": disconnected["device_id"],
            "name": "Mac de salón",
            "paired_at": 701.0,
            "connected": False,
            "assistant_turn_active": False,
            "last_seen": 850.0,
        },
        connected["device_id"]: {
            "id": connected["device_id"],
            "name": "fixture",
            "paired_at": 1001.0,
            "connected": True,
            "assistant_turn_active": False,
            "last_seen": 1001.0,
        },
    }
    assert revoked["device_id"] not in rows
    assert all("token" not in row for row in rows.values())


async def test_a_replayed_signature_is_refused_the_second_time(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body(device_id=owner["device_id"])
        headers = signed(payload)
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(
                answer_once(service, {"refused": None, "result": {"windows": 1}})
            )
            first = await client.post(PATH, content=payload, headers=headers)
            await mac
            assert first.status_code == 200
            second = await client.post(PATH, content=payload, headers=headers)
        assert second.status_code == 401
        assert second.json()["code"] == "consumer_replay"


async def test_an_unknown_device_is_answered_exactly_like_a_disconnected_one(bridged):
    # Holding the key does not entitle a caller to enumerate this hub's devices,
    # so the two cases must be indistinguishable from outside.
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        # Registered and connected, but with nothing listening: this is the case
        # the test exists for, and an earlier version of it left the device out
        # of `runtimes` entirely, so both requests took the "unknown" path and
        # the assertion could not have failed.
        runtime = DeviceRuntime(owner["device_id"], store, adapter, lambda: NOW_MS / 1000)
        await runtime.connect()
        app.state.device_bridge.runtimes[owner["device_id"]] = runtime
        assert not runtime.listeners
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            # Both under one op, so the bodies can be compared whole.
            op = {"X-Orbit-Op": "op-unknown-or-silent"}
            missing = body(device_id=str(uuid4()))
            absent = await client.post(PATH, content=missing, headers=signed(missing) | op)
            quiet = body(device_id=owner["device_id"])
            silent = await client.post(PATH, content=quiet, headers=signed(quiet) | op)
        assert absent.status_code == silent.status_code == 404
        assert absent.json() == silent.json()


async def test_an_unknown_capability_is_relayed_to_the_mac(bridged):
    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        await connect(app, store, adapter, owner)
        service = app.state.device_bridge
        payload = body("future_capability", owner["device_id"])
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            mac = asyncio.create_task(
                answer_once(
                    service,
                    {"refusal": {"code": "unknown_capability", "retry": {"kind": "after_fix"}}},
                )
            )
            response = await client.post(PATH, content=payload, headers=signed(payload))
            entry = await mac
        assert entry.capability == "future_capability"
        assert response.status_code == 403
        assert response.json()["refusal"]["code"] == "unknown_capability"


async def test_the_route_is_closed_until_it_is_configured(environment):
    # Half a configuration must not look like a working bridge.
    settings, store, adapter = environment
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: NOW_MS / 1000
    )
    async with app.router.lifespan_context(app):
        payload = body(device_id=str(uuid4()))
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:
            response = await client.post(PATH, content=payload, headers=signed(payload))
        assert response.status_code == 503


def test_a_bridge_without_credentials_is_refused_at_load():
    with pytest.raises(ValueError, match="key_id"):
        BridgeConfiguration()
    with pytest.raises(ValueError):
        BridgeConfiguration(key_id=KEY_ID, credential_file="relativa/no/vale")


@pytest.mark.parametrize(
    "capability,field",
    [
        ("screenshot", "application"),
        ("screenshot", "title"),
        ("selected_text", "application"),
        ("selected_text", "title"),
    ],
)
def test_capture_filter_length_refusal_carries_the_field_size_and_ceiling(capability, field):
    from orbit_mcp.hub import HubClientError, validate_parameters

    with pytest.raises(HubClientError) as error:
        validate_parameters(capability, {field: "a" * 1025})
    refusal = error.value.detail["refusal"]
    assert refusal["code"] == "invalid_parameters"
    assert refusal["field"] == field
    assert refusal["chars"] == 1025 and refusal["max_chars"] == 1024


@pytest.mark.parametrize(
    "read_id,byte_count", [("a" * 128, 128), ("a" * 129, 129), ("ñ" * 64, 128), ("ñ" * 65, 130)]
)
def test_calendar_read_id_matches_the_mac_byte_limit(read_id, byte_count):
    from orbit_mcp.hub import HubClientError, validate_parameters

    parameters = {"start_ms": 0, "end_ms": 1, "read_id": read_id, "after_ms": 0, "after_index": 0}
    if byte_count == 128:
        assert validate_parameters("calendar_read", parameters) == parameters
    else:
        with pytest.raises(HubClientError) as error:
            validate_parameters("calendar_read", parameters)
        refusal = error.value.detail["refusal"]
        assert refusal["field"] == "read_id"
        assert refusal["bytes"] == byte_count and refusal["max_bytes"] == 128


async def test_device_inventory_marks_only_a_connected_macs_active_assistant_turn(
    bridged, monkeypatch
):
    from support_runtime_completion_persistence import DeferredConnection

    from orbit_hub.protocol import SendParams

    app, store, adapter, owner = bridged
    async with app.router.lifespan_context(app):
        runtime = await connect(app, store, adapter, owner)
        monkeypatch.setattr(
            runtime.connection, "rpc", DeferredConnection.rpc.__get__(runtime.connection)
        )
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client:

            async def active():
                response = await client.get(DEVICES_PATH, headers=signed(b"", path=DEVICES_PATH))
                assert response.status_code == 200
                return next(
                    row for row in response.json()["devices"] if row["id"] == owner["device_id"]
                )["assistant_turn_active"]

            assert await active() is False
            _, receipt = await runtime.handle(
                "chat.send", SendParams(text="synthetic turn"), "turn-id"
            )
            assert await active() is True
            store.status(receipt["session_id"], "completed")
            assert await active() is False
            store.status(receipt["session_id"], "outcome_unknown")
            assert await active() is False
            store.status(receipt["session_id"], "streaming")
            runtime.listeners.clear()
            assert await active() is False
