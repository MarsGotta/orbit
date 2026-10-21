"""PUT /v1/captures/{id}: the bytes of a remote capture, with a ceiling and an expiry.

A capture the Mac cannot send over the WebSocket (the frame does not fit) goes
up over HTTP like the audio chunks. These cases check the route's limits and
the in-memory store behind it: none of this touches a real device or screen.
"""

import hashlib
from uuid import uuid4

from conftest import device
from support_app import client

# The ceiling of a PNG capture: the constants registry, copied into the hub.
from orbit_hub.captures import MAX_CAPTURE_BYTES, MAX_SAVED_CAPTURES_PER_DEVICE

PNG = b"\x89PNG\r\n\x1a\n" + b"captura sintetica" * 64


def upload(owner, body=PNG, content_type="image/png", capture_id=None, request_id=None):
    return {
        "url": f"/v1/captures/{capture_id or uuid4()}",
        "content": body,
        "headers": {
            "Authorization": "Bearer " + owner["token"],
            "Content-Type": content_type,
            "X-Orbit-Request": request_id or str(uuid4()),
            "X-Orbit-Sha256": hashlib.sha256(body).hexdigest(),
        },
    }


def test_the_capture_route_is_only_for_paired_devices(environment):
    _, store, _ = environment
    owner = device(store)
    payload = upload(owner, body=PNG[:16])
    headers = {name: value for name, value in payload["headers"].items() if name != "Authorization"}
    with client(environment) as http:
        response = http.put(payload["url"], content=payload["content"], headers=headers)
    assert response.status_code == 401


def test_the_route_accepts_the_ceiling_image_and_refuses_more_or_another_type(environment):
    _, store, _ = environment
    owner = device(store)
    ceiling = b"\x89PNG\r\n\x1a\n" + bytes(MAX_CAPTURE_BYTES - 8)
    assert len(ceiling) == MAX_CAPTURE_BYTES
    with client(environment) as http:
        accepted = http.put(**upload(owner, body=ceiling))
        assert accepted.status_code == 200, accepted.text
        heavier = http.put(**upload(owner, body=ceiling + b"\x00"))
        assert heavier.status_code == 413, heavier.text
        wrong_type = http.put(**upload(owner, body=ceiling, content_type="image/jpeg"))
        assert wrong_type.status_code == 415, wrong_type.text


def test_each_device_gets_max_pending_captures_and_then_is_told_to_wait(environment):
    _, store, _ = environment
    owner, other = device(store), device(store)
    with client(environment) as http:
        first = None
        for _ in range(MAX_SAVED_CAPTURES_PER_DEVICE):
            payload = upload(owner)
            if first is None:
                first = payload
            response = http.put(**payload)
            assert response.status_code == 200, response.text
        duplicate = http.put(**first)
        assert duplicate.status_code == 409, duplicate.text
        assert duplicate.json()["code"] == "capture_duplicate"
        saturated = http.put(**upload(owner))
        assert saturated.status_code == 429, saturated.text
        assert saturated.json()["code"] == "too_many"
        assert saturated.json()["refusal"]["code"] == "too_many"
        assert saturated.json()["refusal"]["resource"] == "capture"
        assert saturated.json()["refusal"]["count"] == MAX_SAVED_CAPTURES_PER_DEVICE + 1
        assert saturated.json()["refusal"]["max"] == MAX_SAVED_CAPTURES_PER_DEVICE
        # The quota is per device: one Mac's memory does not take another's.
        neighbour = http.put(**upload(other))
        assert neighbour.status_code == 200, neighbour.text


def test_capture_store_hands_each_capture_over_exactly_once():
    from orbit_hub.captures import CaptureStore

    clock = [1_000.0]
    store = CaptureStore(now=lambda: clock[0])
    store.put("dispositivo", "captura-1", "puente-1", PNG)
    # It is handed over once, with the exact bytes; handing it over consumes it.
    assert store.take("dispositivo", "captura-1", "puente-1") == PNG
    assert store.take("dispositivo", "captura-1", "puente-1") is None
    # And a capture of another device or another request is not handed across.
    store.put("dispositivo", "captura-2", "puente-2", PNG)
    assert store.take("otro", "captura-2", "puente-2") is None
    assert store.take("dispositivo", "captura-2", "puente-otro") is None


def test_capture_store_expires_after_five_minutes_with_an_injected_clock():
    from orbit_hub.captures import CaptureStore

    clock = [1_000.0]
    store = CaptureStore(now=lambda: clock[0])
    store.put("dispositivo", "captura-1", "puente-1", PNG)
    clock[0] += 299
    assert store.take("dispositivo", "captura-1", "puente-1") == PNG
    store.put("dispositivo", "captura-2", "puente-2", PNG)
    clock[0] += 301
    assert store.take("dispositivo", "captura-2", "puente-2") is None
