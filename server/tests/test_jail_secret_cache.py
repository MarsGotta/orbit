"""Background discovery and fail-closed admission through the public jail API."""

# ruff: noqa: F811

import asyncio
import os
import threading
from contextlib import asynccontextmanager

import pytest
from fastapi.testclient import TestClient
from test_assistant_jail import fixture_paths, kernel_root  # noqa: F401
from test_jail_secret_roots import configured, masks

from orbit_hub.app import create_app
from orbit_hub.assistant import jail_secrets
from orbit_hub.assistant.jail import arguments, launch
from orbit_hub.errors import HubError


@asynccontextmanager
async def running_cache(settings):
    cache = settings.assistant_jail_cache
    cache.start()
    try:
        yield cache
    finally:
        await cache.close()


def fixtures(environment, kernel_root, monkeypatch):
    settings, agent, work, *_ = fixture_paths(environment, kernel_root, monkeypatch)
    root = kernel_root / "stacks"
    root.mkdir()
    return configured(settings, root), agent, work, root


async def test_cache_refreshes_in_background_and_warm_launches_do_not_scan(
    environment, kernel_root, monkeypatch
):
    from orbit_hub.assistant import jail_secret_cache

    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    first, second = root / ".env", root / "config.toml"
    first.touch()
    tick, sleeping, refreshed = asyncio.Event(), asyncio.Event(), asyncio.Event()
    visits = []
    original = jail_secrets.scan_root

    def scan(*args):
        visits.append(args[0])
        return original(*args)

    async def delay():
        if len(visits) >= 2:
            refreshed.set()
        sleeping.set()
        await tick.wait()
        tick.clear()

    monkeypatch.setattr(jail_secrets, "scan_root", scan)
    monkeypatch.setattr(jail_secret_cache, "refresh_delay", delay)
    async with running_cache(settings) as cache:
        await cache.wait_ready()
        await sleeping.wait()
        second.touch()
        assert str(first) in masks(await launch(settings, agent, work))
        assert str(second) not in masks(await launch(settings, agent, work))
        assert visits == [root]
        tick.set()
        await refreshed.wait()
        assert str(second) in masks(await launch(settings, agent, work))
        assert visits == [root, root]


async def test_first_launch_accepts_a_scan_slower_than_thirty_seconds(
    environment, kernel_root, monkeypatch
):
    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    secret = root / ".env"
    secret.touch()
    entered, release = threading.Event(), threading.Event()
    original = jail_secrets.scan_root

    def held_scan(*args):
        entered.set()
        assert release.wait(10), "test did not release the scan"
        return original(*args)

    monkeypatch.setattr(jail_secrets, "scan_root", held_scan)
    loop = asyncio.get_running_loop()
    clock = loop.time
    elapsed = 0
    monkeypatch.setattr(loop, "time", lambda: clock() + elapsed)
    async with running_cache(settings):
        pending = asyncio.create_task(launch(settings, agent, work))
        try:
            assert await asyncio.to_thread(entered.wait, 10)
            elapsed = 31
            for _ in range(10):
                await asyncio.sleep(0)
            assert not pending.done()
        finally:
            release.set()
        assert str(secret) in masks(await pending)


async def test_missing_cached_file_is_skipped(environment, kernel_root, monkeypatch):
    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    secret = root / ".env"
    secret.touch()
    async with running_cache(settings) as cache:
        await cache.wait_ready()
        assert str(secret) in masks(arguments(settings, agent, work))
        secret.unlink()
        assert str(secret) not in masks(arguments(settings, agent, work))


async def test_unreadable_files_are_not_masked(environment, kernel_root, monkeypatch):
    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    readable, unreadable = root / ".env", root / "config.toml"
    readable.touch()
    unreadable.touch()
    original = os.access

    def access(path, mode, **kwargs):
        if path == unreadable:
            assert mode == os.R_OK
            return False
        return original(path, mode, **kwargs)

    monkeypatch.setattr(os, "access", access)
    async with running_cache(settings) as cache:
        await cache.wait_ready()
        found = masks(arguments(settings, agent, work))
        assert str(readable) in found and str(unreadable) not in found


async def test_failed_initial_scan_refuses_without_running_bwrap(
    environment, kernel_root, monkeypatch
):
    settings, agent, work, _ = fixtures(environment, kernel_root, monkeypatch)

    def broken(*args):
        raise OSError("invented discovery failure")

    monkeypatch.setattr(jail_secrets, "scan_root", broken)
    async with running_cache(settings):
        with pytest.raises(HubError) as refused:
            await launch(settings, agent, work)
        assert refused.value.code == "agent_adapter_missing"
        assert refused.value.numbers["cause"] == "secret_scan_failed"


async def test_initial_wait_is_bounded_without_cancelling_the_scan(
    environment, kernel_root, monkeypatch
):
    from orbit_hub.assistant import jail_secret_cache

    settings, agent, work, _ = fixtures(environment, kernel_root, monkeypatch)
    entered, release = threading.Event(), threading.Event()
    original = jail_secrets.scan_root

    def held(*args):
        entered.set()
        assert release.wait(10), "test did not release the scan"
        return original(*args)

    monkeypatch.setattr(jail_secrets, "scan_root", held)
    wait_limit = jail_secret_cache.INITIAL_SCAN_WAIT_SECONDS
    monkeypatch.setattr(jail_secret_cache, "INITIAL_SCAN_WAIT_SECONDS", 0)
    async with running_cache(settings) as cache:
        try:
            assert await asyncio.to_thread(entered.wait, 10)
            with pytest.raises(HubError) as refused:
                await launch(settings, agent, work)
            assert refused.value.numbers["cause"] == "secret_scan_timeout"
        finally:
            release.set()
        monkeypatch.setattr(jail_secret_cache, "INITIAL_SCAN_WAIT_SECONDS", wait_limit)
        await cache.wait_ready()
        assert (await launch(settings, agent, work))[-1] == "--"


def test_hub_start_scans_without_an_assistant_and_shutdown_stops_refresh(
    environment, kernel_root, monkeypatch
):
    from orbit_hub.assistant import jail_secret_cache

    settings, store, adapter = environment
    root = kernel_root / "stacks"
    root.mkdir()
    secret = root / ".env"
    secret.touch()
    settings = configured(settings, root)
    sleeping, stopped = threading.Event(), threading.Event()

    async def delay():
        try:
            sleeping.set()
            await asyncio.Event().wait()
        finally:
            stopped.set()

    monkeypatch.setattr(jail_secret_cache, "refresh_delay", delay)
    app = create_app(settings, store=store, assistant=adapter)
    with TestClient(app) as client:
        assert sleeping.wait(10), "hub startup did not scan its configured root"
        client.portal.call(settings.assistant_jail_cache.wait_ready)
        assert settings.assistant_jail_cache.paths() == (secret,)
    assert stopped.is_set(), "hub shutdown left its scan worker running"


async def test_launches_use_previous_snapshot_during_a_slow_refresh(
    environment, kernel_root, monkeypatch
):
    from orbit_hub.assistant import jail_secret_cache

    settings, agent, work, root = fixtures(environment, kernel_root, monkeypatch)
    secret = root / ".env"
    secret.touch()
    tick, sleeping = asyncio.Event(), asyncio.Event()
    entered, release = threading.Event(), threading.Event()
    original = jail_secrets.scan_root
    visits = []

    def scan(*args):
        visits.append(args[0])
        if len(visits) == 2:
            entered.set()
            assert release.wait(10), "test did not release refresh"
        return original(*args)

    async def delay():
        sleeping.set()
        await tick.wait()
        tick.clear()

    monkeypatch.setattr(jail_secrets, "scan_root", scan)
    monkeypatch.setattr(jail_secret_cache, "refresh_delay", delay)
    async with running_cache(settings) as cache:
        await cache.wait_ready()
        await sleeping.wait()
        tick.set()
        try:
            assert await asyncio.to_thread(entered.wait, 10)
            assert str(secret) in masks(await launch(settings, agent, work))
            assert not release.is_set()
        finally:
            release.set()
