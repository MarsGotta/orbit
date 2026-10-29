"""Restart admission through the jail API, with only synthetic scan roots."""

# ruff: noqa: F811

import asyncio
import json
import stat
import threading
from dataclasses import replace
from types import SimpleNamespace

import pytest
from test_assistant_jail import kernel_root  # noqa: F401
from test_jail_secret_cache import fixtures, running_cache
from test_jail_secret_roots import masks

from orbit_hub.assistant import jail_secret_cache, jail_secrets
from orbit_hub.assistant.jail import arguments, launch
from orbit_hub.errors import HubError


@pytest.mark.parametrize("age", [599, 600, 601, -1])
async def test_restart_uses_only_fresh_snapshot_and_replaces_it_after_scan(
    environment, kernel_root, monkeypatch, age
):
    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    secret = root / ".env.fixture"
    secret.write_text("invented-private-payload")
    now = 10_000
    monkeypatch.setattr(jail_secret_cache, "time", SimpleNamespace(time=lambda: now), raising=False)
    async with running_cache(settings):
        assert str(secret) in masks(await launch(settings, agent, work))
    snapshot = settings.data_dir / "secret-scan.json"
    assert snapshot.is_file(), "successful discovery was not saved for restart"
    assert stat.S_IMODE(snapshot.stat().st_mode) == 0o600
    saved = snapshot.read_text()
    json.loads(saved)
    assert str(secret) in saved
    assert "invented-private-payload" not in saved

    second = root / "credentials.fixture"
    second.touch()
    now += age
    entered, release = threading.Event(), threading.Event()
    refreshed = asyncio.Event()
    original = jail_secrets.scan_root

    def held_scan(*args):
        entered.set()
        assert release.wait(10), "test did not release discovery"
        return original(*args)

    async def delay():
        refreshed.set()
        await asyncio.Event().wait()

    monkeypatch.setattr(jail_secrets, "scan_root", held_scan)
    monkeypatch.setattr(jail_secret_cache, "refresh_delay", delay)
    restarted = replace(settings)
    # An open reader must keep seeing the complete old snapshot during replacement.
    with snapshot.open() as previous:
        async with running_cache(restarted):
            pending = asyncio.create_task(launch(restarted, agent, work))
            try:
                assert await asyncio.to_thread(entered.wait, 10)
                if age == 599:
                    found = masks(await asyncio.wait_for(asyncio.shield(pending), 2))
                    assert str(secret) in found and str(second) not in found
                    assert not release.is_set()
                else:
                    assert not pending.done()
                    with pytest.raises(HubError) as refused:
                        arguments(restarted, agent, work)
                    assert refused.value.numbers["cause"] == "secret_scan_not_ready"
            finally:
                release.set()
            assert str(secret) in masks(await pending)
            await asyncio.wait_for(refreshed.wait(), 2)
            assert str(second) in masks(await launch(restarted, agent, work))
            assert previous.read() == saved
            assert str(second) in snapshot.read_text()
            assert stat.S_IMODE(snapshot.stat().st_mode) == 0o600


@pytest.mark.parametrize("damage", ["truncated", "public", "symlink"])
async def test_unusable_saved_snapshot_waits_for_discovery(
    environment, kernel_root, monkeypatch, damage
):
    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    secret = root / ".env.fixture"
    secret.touch()
    async with running_cache(settings):
        await launch(settings, agent, work)
    snapshot = settings.data_dir / "secret-scan.json"
    assert snapshot.is_file(), "successful discovery was not saved for restart"
    if damage == "truncated":
        snapshot.write_text("{")
    elif damage == "public":
        snapshot.chmod(0o644)
    else:
        target = snapshot.with_suffix(".previous")
        snapshot.rename(target)
        snapshot.symlink_to(target)
    entered, release = threading.Event(), threading.Event()
    original = jail_secrets.scan_root

    def held_scan(*args):
        entered.set()
        assert release.wait(10), "test did not release discovery"
        return original(*args)

    monkeypatch.setattr(jail_secrets, "scan_root", held_scan)
    restarted = replace(settings)
    async with running_cache(restarted):
        pending = asyncio.create_task(launch(restarted, agent, work))
        try:
            assert await asyncio.to_thread(entered.wait, 10)
            assert not pending.done()
            with pytest.raises(HubError) as refused:
                arguments(restarted, agent, work)
            assert refused.value.numbers["cause"] == "secret_scan_not_ready"
        finally:
            release.set()
        assert str(secret) in masks(await pending)
    assert stat.S_IMODE(snapshot.stat().st_mode) == 0o600
