"""What the assistant hears about each device: what it is and what it handles.

The hub's inventory says each device's `kind` and `capabilities`. A hub from
before says neither, and its devices are Macs that handle everything. Every
name here is invented.
"""

import time

from support_orbit_mcp import FakeHub, serving, text_of

from orbit_hub.bridge_outcomes import failure
from orbit_hub.errors import PrivateError

MAC_ID = "6a2e0000-0000-4000-8000-000000000001"
PHONE_ID = "6a2e0000-0000-4000-8000-000000000002"
EVERY_GROUP = (
    "system_audio",
    "screen",
    "selected_text",
    "folders",
    "calendar",
    "call_detection",
    "shortcuts",
    "control",
)


def listing(*, mac_says_nothing=False):
    now = time.time()
    mac = {
        "id": MAC_ID,
        "name": "Mac de pruebas",
        "paired_at": 1_000.0,
        "connected": True,
        "last_seen": now,
    }
    if not mac_says_nothing:
        mac |= {"kind": "mac", "capabilities": list(EVERY_GROUP)}
    phone = {
        "id": PHONE_ID,
        "name": "Teléfono Sirio",
        "paired_at": 1_100.0,
        "connected": True,
        "last_seen": now,
        "kind": "phone",
        "capabilities": ["folders", "calendar"],
    }
    return [mac, phone]


def line_of(text: str, name: str) -> str:
    return next(line for line in text.splitlines() if line.startswith(f"- {name} "))


async def test_orbit_devices_tells_what_each_device_is_and_handles(tmp_path):
    hub = FakeHub(devices=listing())
    async with serving(tmp_path, hub, default_device="Mac de pruebas", devices={}) as (_, server):
        result = await server.call_tool("orbit_devices", {})

    assert not result.is_error
    text = text_of(result)
    mac, phone = line_of(text, "Mac de pruebas"), line_of(text, "Teléfono Sirio")
    assert "Mac de pruebas — connected yes — last seen" in mac
    assert "mac" in mac.replace("Mac de pruebas", "")
    assert all(group in mac for group in EVERY_GROUP)
    assert "phone" in phone
    assert "folders" in phone and "calendar" in phone
    assert all(group not in phone for group in EVERY_GROUP if group not in {"folders", "calendar"})


async def test_a_device_from_an_older_hub_is_listed_as_a_mac_that_handles_everything(tmp_path):
    # Only the Mac: an older hub says neither field of any device.
    hub = FakeHub(devices=listing(mac_says_nothing=True)[:1])
    async with serving(tmp_path, hub, default_device="Mac de pruebas", devices={}) as (_, server):
        result = await server.call_tool("orbit_devices", {})

    assert not result.is_error
    mac = line_of(text_of(result), "Mac de pruebas")
    assert "mac" in mac.replace("Mac de pruebas", "")
    assert all(group in mac for group in EVERY_GROUP)


async def test_a_screenshot_of_a_phone_without_a_screen_is_refused_naming_the_device(tmp_path):
    refused = failure(
        PrivateError(
            403,
            "device_capability_missing",
            {
                "device_id": PHONE_ID,
                "kind": "phone",
                "capability": "screenshot",
                "group": "screen",
            },
        )
    )
    hub = FakeHub(devices=listing(), device_replies=[refused])
    async with serving(tmp_path, hub, default_device="Mac de pruebas", devices={}) as (_, server):
        result = await server.call_tool("orbit_screenshot", {"device": "Teléfono Sirio"})

    assert result.is_error
    text = text_of(result)
    assert "device_capability_missing" in text
    assert "Teléfono Sirio" in text
    assert "that device" in text.lower()
    assert "screen" in text
    assert "The Mac" not in text
    assert PHONE_ID not in text
