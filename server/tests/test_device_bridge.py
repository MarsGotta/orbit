"""The hub half of the bridge: which answer belongs to which question."""

import pytest

from orbit_hub.device_bridge import (
    MAX_PENDING_REQUESTS,
    REQUEST_TIMEOUT_SECONDS,
    Bridge,
    BridgeRefused,
)


def test_an_answer_is_returned_to_the_question_that_asked_it():
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    claimed = bridge.resolve("r1", "mac-de-prueba")
    assert claimed is not None
    assert claimed.capability == "screenshot"
    # Claimed once: a second answer to the same question has no owner left.
    assert bridge.resolve("r1", "mac-de-prueba") is None


def test_a_device_cannot_answer_a_question_it_was_never_asked():
    # The payload is a picture of somebody's screen. Delivering it because the
    # id happened to match is exactly the failure worth being paranoid about.
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    assert bridge.resolve("r1", "otro-mac") is None
    # And the real question is still waiting, not consumed by the impostor.
    assert bridge.resolve("r1", "mac-de-prueba") is not None


def test_an_unknown_answer_is_dropped_rather_than_delivered_anywhere():
    assert Bridge().resolve("jamas-preguntado", "mac-de-prueba") is None


def test_an_id_is_never_reused_while_its_question_is_open():
    # Otherwise a late answer to the old question resolves the new one, and the
    # caller gets a screenshot of something it did not ask about.
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    with pytest.raises(BridgeRefused) as refused:
        bridge.open("r1", "mac-de-prueba", "selected_text", asked_at=1.0)
    assert refused.value.code == "duplicate_request"


def test_a_caller_cannot_fill_the_hub_with_questions_the_mac_never_sees():
    bridge = Bridge()
    for index in range(MAX_PENDING_REQUESTS):
        bridge.open(f"r{index}", "mac-de-prueba", "screenshot", asked_at=0.0)
    with pytest.raises(BridgeRefused) as refused:
        bridge.open("uno-mas", "mac-de-prueba", "screenshot", asked_at=0.0)
    assert refused.value.code == "too_many_requests"
    assert refused.value.numbers == {"count": MAX_PENDING_REQUESTS + 1, "max": MAX_PENDING_REQUESTS}


def test_an_abandoned_question_stops_being_answerable():
    # The caller already gave up. Handing it a screenshot afterwards delivers a
    # picture of a screen nobody is watching for it any more.
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    expired = bridge.expire(now=REQUEST_TIMEOUT_SECONDS)
    assert [entry.request_id for entry in expired] == ["r1"]
    assert bridge.resolve("r1", "mac-de-prueba") is None


def test_a_question_still_within_its_window_is_left_alone():
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    assert bridge.expire(now=REQUEST_TIMEOUT_SECONDS - 0.01) == []
    assert bridge.resolve("r1", "mac-de-prueba") is not None


def test_a_disconnected_mac_takes_its_questions_with_it():
    # It will never answer, and letting the question survive a reconnection
    # would deliver a reply about a screen from before the gap.
    bridge = Bridge()
    bridge.open("r1", "mac-de-prueba", "screenshot", asked_at=0.0)
    bridge.open("r2", "otro-mac", "screenshot", asked_at=0.0)
    gone = bridge.abandon("mac-de-prueba")
    assert [entry.request_id for entry in gone] == ["r1"]
    assert bridge.resolve("r1", "mac-de-prueba") is None
    # The other device is untouched.
    assert bridge.resolve("r2", "otro-mac") is not None
