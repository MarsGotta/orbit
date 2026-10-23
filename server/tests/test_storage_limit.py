"""How much the hub may store, what it says about it, and to whom.

The limit is a setting of the hub (`storage_quota_bytes`); without it the
disk sets it (`STORAGE_FREE_FLOOR_BYTES`). The hub says what it uses and what
is left in every `ready`, tells the connected Macs when it crosses into
nearly full, and refuses with 507 and those numbers. Nothing is deleted.
The disk is a synthetic measurement: no test depends on this machine's disk.
"""

import hashlib
import time
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_transcription import wav_fixture

from orbit_hub import background
from orbit_hub.config import Settings
from orbit_hub.storage import (
    STORAGE_FREE_FLOOR_BYTES,
    STORAGE_WARNING_PERCENT,
    Store,
)

GIB = 1024**3


def opened(environment, free, quota=None):
    """A second Store on the environment's data, measuring `free()` as the disk."""
    settings, _, _ = environment
    return Store(settings.data_dir, settings.key_file, quota, free_space=free)


def test_without_a_quota_the_disk_sets_the_limit_and_leaves_room_for_a_copy(environment):
    store = opened(environment, lambda: 100 * GIB)
    try:
        status = store.space.status()
    finally:
        store.close()
    used = status["used_bytes"]
    assert used > 0
    # Half of what the database and the free space add up to, once the floor
    # is left free: the other half is room for the activation's copy.
    expected = (used + 100 * GIB - STORAGE_FREE_FLOOR_BYTES) // 2
    assert status == {
        "used_bytes": used,
        "max_bytes": expected,
        "remaining_bytes": expected - used,
        "limit": "disk",
        "nearly_full": False,
    }
    # At the limit the disk still holds one whole copy of the database and the floor.
    assert 100 * GIB - (expected - used) >= expected + STORAGE_FREE_FLOOR_BYTES - 1


def test_an_explicit_quota_caps_the_limit_but_never_past_what_the_disk_holds(environment):
    store = opened(environment, lambda: 100 * GIB, quota=20 * GIB)
    try:
        assert store.space.status()["max_bytes"] == 20 * GIB
        assert store.space.status()["limit"] == "quota"
        store.space.free_space = lambda: 10 * GIB  # the disk fills with something else
        status = store.space.status()
        assert status["limit"] == "disk"
        assert status["max_bytes"] < 20 * GIB
    finally:
        store.close()


def test_a_disk_with_less_than_the_floor_free_admits_nothing_and_says_so(environment):
    store = opened(environment, lambda: STORAGE_FREE_FLOOR_BYTES // 2)
    try:
        status = store.space.status()
        assert status["max_bytes"] == 0 and status["remaining_bytes"] == 0
        assert status["nearly_full"] is True
        with pytest.raises(Exception) as refused:
            store.devices.create_code(1000)
        assert refused.value.code == "quota_exceeded"
        assert refused.value.numbers["limit"] == "disk"
        assert refused.value.numbers["remaining_bytes"] == 0
    finally:
        store.close()


def test_nearly_full_starts_at_the_warning_share_of_the_limit(environment):
    store = opened(environment, lambda: 10**15)
    try:
        used = store.space.physical_bytes()
        store.space.quota_bytes = (
            -(-used * 100 // STORAGE_WARNING_PERCENT) + 1
        )  # just under the share
        assert store.space.status()["nearly_full"] is False
        store.space.quota_bytes = used * 100 // STORAGE_WARNING_PERCENT  # at it
        assert store.space.status()["nearly_full"] is True
    finally:
        store.close()


def test_the_quota_is_a_setting_of_the_hub(tmp_path):
    base = (
        f'data_dir = "{tmp_path / "data"}"\n'
        f'key_file = "{tmp_path / "key"}"\n'
        'hermes_url = "https://hermes.invalid"\n'
        f'hermes_credential_file = "{tmp_path / "credential"}"\n'
        '[meeting_transcription]\nendpoint = "http://127.0.0.1:9/"\nmodel = "synthetic"\n'
    )
    path = tmp_path / "hub.toml"
    path.write_text(base, encoding="utf-8")
    assert Settings.load(path).storage_quota_bytes is None
    path.write_text("storage_quota_bytes = 21474836480\n" + base, encoding="utf-8")
    assert Settings.load(path).storage_quota_bytes == 20 * GIB
    path.write_text("storage_quota_bytes = 0\n" + base, encoding="utf-8")
    with pytest.raises(ValueError, match="storage_quota_bytes"):
        Settings.load(path)


def test_a_chunk_over_the_limit_is_refused_with_its_numbers_and_what_sets_the_limit(environment):
    _, store, _ = environment
    owner = device(store)
    body, recording = wav_fixture(), str(uuid4())
    store.space.quota_bytes = store.space.physical_bytes() + 1024
    with client(environment) as http:
        response = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers={
                "Authorization": "Bearer " + owner["token"],
                "X-Orbit-Recording": recording,
                "X-Orbit-Track": "mic_local",
                "X-Orbit-Sequence": "0",
                "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
            },
        )
    assert response.status_code == 507
    refusal = response.json()["refusal"]
    assert refusal["code"] == "quota_exceeded"
    assert refusal["limit"] == "quota"
    assert refusal["max_bytes"] == store.space.quota_bytes
    assert refusal["remaining_bytes"] == max(0, refusal["max_bytes"] - refusal["used_bytes"])
    assert refusal["retry"] == {"kind": "after_fix"}
    assert "storage_quota_bytes" in response.json()["message"]


def test_a_chunk_the_full_disk_refused_is_storage_full_with_the_free_space(environment):
    _, store, _ = environment
    owner = device(store)
    body, recording = wav_fixture(), str(uuid4())
    # Admission lets it in (its numbers are what the hub measures, not the
    # disk SQLite writes to): the disk itself refuses.
    store.space.free_space = lambda: 100 * GIB

    def disk_full(*_args, **_kwargs):
        raise OSError(28, "No space left on device")

    store.encrypt = disk_full
    with client(environment) as http:
        response = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers={
                "Authorization": "Bearer " + owner["token"],
                "X-Orbit-Recording": recording,
                "X-Orbit-Track": "mic_local",
                "X-Orbit-Sequence": "0",
                "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
            },
        )
    assert response.status_code == 507
    payload = response.json()
    assert payload["code"] == "storage_full"
    assert "libera espacio" in payload["message"]
    refusal = payload["refusal"]
    assert refusal["free_bytes"] == 100 * GIB
    assert refusal["used_bytes"] == store.space.physical_bytes()
    assert refusal["retry"] == {"kind": "after_fix"}


def test_ready_says_what_the_hub_uses_and_what_is_left(environment):
    _, store, _ = environment
    owner = device(store)
    store.space.free_space = lambda: 100 * GIB
    with (
        client(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        ready = ws.receive_json()
    assert ready["event"] == "ready"
    storage = ready["data"]["storage"]
    assert storage == store.space.status()
    assert storage["limit"] == "disk" and storage["nearly_full"] is False


def test_the_connected_macs_are_told_when_the_hub_becomes_nearly_full(environment, monkeypatch):
    monkeypatch.setattr(background, "STORAGE_CHECK_SECONDS", 0.05)
    _, store, _ = environment
    owner = device(store)
    store.space.free_space = lambda: 100 * GIB
    with (
        client(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        assert ws.receive_json()["data"]["storage"]["nearly_full"] is False
        store.space.quota_bytes = store.space.physical_bytes() + 1  # past 80 % of the limit
        while True:
            frame = ws.receive_json()
            if frame["event"] == "storage.status":
                break
        told = frame["data"]
        assert told["nearly_full"] is True and told["limit"] == "quota"
        assert told["remaining_bytes"] == told["max_bytes"] - told["used_bytes"]
        # Back below once someone acts, and said again.
        store.space.quota_bytes = None
        while True:
            frame = ws.receive_json()
            if frame["event"] == "storage.status":
                break
        assert frame["data"]["nearly_full"] is False


def test_the_storage_watch_says_nothing_while_the_level_holds(environment, monkeypatch):
    monkeypatch.setattr(background, "STORAGE_CHECK_SECONDS", 0.01)
    _, store, _ = environment
    owner = device(store)
    store.space.free_space = lambda: 100 * GIB
    with (
        client(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + owner["token"]}
        ) as ws,
    ):
        ws.receive_json()
        checked = []
        status = store.space.status

        def observed():
            checked.append(1)
            return status()

        store.space.status = observed
        deadline = time.monotonic() + 10
        while len(checked) < 5 and time.monotonic() < deadline:
            time.sleep(0.01)
        assert len(checked) >= 5, "the hub stopped looking at its storage"
        ws.send_json({"v": 1, "id": str(uuid4()), "method": "ping", "params": {}})
        events = []
        while not events or events[-1] != "pong":
            events.append(ws.receive_json()["event"])
        assert "storage.status" not in events
