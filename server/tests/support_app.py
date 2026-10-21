"""Shared fixtures and helpers for test_app."""

import sys

from fastapi.testclient import TestClient

from orbit_hub.app import create_app

__all__ = ("client", "crash_the_revocation_pass")


def client(environment):
    settings, store, adapter = environment
    return TestClient(
        create_app(settings, store=store, adapter=adapter, now=lambda: 1001),
        base_url="https://localhost",
    )


def crash_the_revocation_pass(store, monkeypatch, crash):
    """Make one pass of the lifespan's `enforce_revocations` loop fail.

    The loop asks `store.is_active` about every device with a runtime, so a
    device needs one (an open socket) for the pass to get there. Sockets ask
    `is_active` too; only the loop's own call raises `crash()`. Returns the
    count of crashes still to happen.
    """
    original = store.is_active
    pending = [1]

    def is_active(device_id):
        # Python 3.11 runs the loop's list comprehension in a frame of its own.
        caller = sys._getframe(1)
        callers = {caller.f_code.co_name, caller.f_back.f_code.co_name if caller.f_back else ""}
        if pending[0] and "enforce_revocations" in callers:
            pending[0] -= 1
            raise crash()
        return original(device_id)

    monkeypatch.setattr(store, "is_active", is_active)
    return pending
