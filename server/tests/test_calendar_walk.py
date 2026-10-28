from support_orbit_mcp import serving

"""Deimos asks about any range once, and gets all of it.

A month used to take Deimos four questions: the Mac read at most eight
days and two hundred events per read, and the answer said «no está completo,
sigue desde aquí». The user decided the macOS permission is the only thing
between Deimos and their agenda, and that no ceiling of Orbit's is ever seen. So the Mac
reads in pieces and the translator joins them: ``orbit_calendar`` is still one
call, however long the range and however full the day.

These cases cover the translator's half:

* The walk over the Mac's pages, with the exact continuation it sends,
  and nothing in the answer that says «incompleto» or «sigue desde aquí».
* One event split across two pages because its own invitees did not fit
  one frame comes back as one event.
* ``orbit_calendar_ask`` does not exist; ``orbit_calendar`` is the only
  agenda tool.
* The retired ``[calendar]`` section in ``mcp.toml`` is rejected at load;
  that configuration boundary is covered in ``test_settings.py``.
* Each missing condition arrives as itself, and an unknown permission
  is neither granted nor denied.
"""

import json
import re

import pytest
from fastapi.responses import Response
from support_orbit_mcp import (
    BRIDGE_RESULT,
    FakeHub,
    answered,
    hub_rejected,
    text_of,
)

OCTOBER_MS = 1_790_805_600_000  # 1 October 2026, 00:00 in Madrid (22:00 UTC the day before)
DAY_MS = 86_400_000
READ_ID = "7a1b0000-0000-4000-8000-00000000aaaa"


def event(index: int, start_ms: int, **extra) -> dict:
    value = {
        "observation_id": f"55555555-5555-4555-8555-{index:012d}",
        "calendar": {"calendar_id": f"calendar-{index % 3}", "source_id": "account"},
        "calendar_item_id": f"item-{index}",
        "start_unix_ms": start_ms,
        "end_unix_ms": start_ms + 3_600_000,
        "interpreted_timezone": "Europe/Madrid",
        "event_timezone": "Europe/Madrid",
        "is_floating": False,
        "is_all_day": False,
        "is_detached": False,
        "title": f"Evento {index}",
        "status": "confirmed",
        "invitees_status": "not_provided",
        "event_url_status": "not_provided",
    }
    value.update(extra)
    return value


def page(events, *, after=None, read_id=READ_ID, start=OCTOBER_MS, end=OCTOBER_MS + 31 * DAY_MS):
    result = {
        "read_id": read_id,
        "requested": {"start_unix_ms": start, "end_unix_ms": end},
        "read_at_unix_ms": start - 60_000,
        "system_timezone": "Europe/Madrid",
        "provider_sync_status": "unknown",
        "enumeration": "bounded_local_read",
        "events": events,
        "events_returned": len(events),
        "omitted": [],
    }
    if after is not None:
        result["next_after_ms"], result["next_after_index"] = after[:2]
        if len(after) == 3:
            result["next_after_invitee"] = after[2]
    return answered(result)


def refused(refusal: dict, notice: str) -> Response:
    return Response(
        content=json.dumps(
            {
                "protocol": BRIDGE_RESULT,
                "state": "refused",
                "refusal": {**refusal, "detail": notice, "retry": {"kind": "after_fix"}},
            }
        ).encode(),
        status_code=403,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


EVENT_LINE = re.compile(r"^\S.*? \[C\d+\] (?P<title>.*?)(?: · |$)")


def event_lines(body: str) -> list[str]:
    """The lines of the answer that are events, in the order Deimos reads them."""
    return [line for line in body.splitlines() if EVENT_LINE.match(line)]


def titles(body: str) -> list[str]:
    return [EVENT_LINE.match(line)["title"] for line in event_lines(body)]


def calendar_body(result):
    assert len(result.content) == 2
    assert all(block.type == "text" for block in result.content)
    assert result.content[1].text == "Selected «portatil»: it is the only connected Mac."
    return result.content[0].text


async def ask(server, start=OCTOBER_MS, end=OCTOBER_MS + 31 * DAY_MS):
    return await server.call_tool("orbit_calendar", {"start_ms": start, "end_ms": end})


# ------------------------------------------------------- the walk over pages


async def test_a_month_in_three_pages_is_one_call_and_one_answer(tmp_path):
    first = [event(i, OCTOBER_MS + i * 3_600_000) for i in range(3)]
    second = [event(i, OCTOBER_MS + 9 * DAY_MS + i * 3_600_000) for i in range(3, 6)]
    third = [event(i, OCTOBER_MS + 20 * DAY_MS + i * 3_600_000) for i in range(6, 8)]
    hub = FakeHub(
        device_replies=[
            page(first, after=(OCTOBER_MS, 3)),
            page(second, after=(OCTOBER_MS + 9 * DAY_MS, 3)),
            page(third),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = calendar_body(result)
    assert not result.is_error, body

    # The continuation the Mac asked for, exactly, and nothing Deimos had to send.
    parameters = [request["parameters"] for request in hub.device_requests]
    assert parameters == [
        {"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + 31 * DAY_MS},
        {
            "start_ms": OCTOBER_MS,
            "end_ms": OCTOBER_MS + 31 * DAY_MS,
            "read_id": READ_ID,
            "after_ms": OCTOBER_MS,
            "after_index": 3,
        },
        {
            "start_ms": OCTOBER_MS,
            "end_ms": OCTOBER_MS + 31 * DAY_MS,
            "read_id": READ_ID,
            "after_ms": OCTOBER_MS + 9 * DAY_MS,
            "after_index": 3,
        },
    ]
    assert titles(body) == [f"Evento {i}" for i in range(8)]
    # The first line is the whole summary: count, range in the Mac's zone and
    # calendars, so that a preview of the answer already says what came.
    assert body.splitlines()[0] == (
        "Agenda: 8 events from Thu 1 Oct 2026 00:00 to Sat 31 Oct 2026 23:00 (time in Europe/Madrid, the Mac’s zone), in 3 calendars, read on the Mac at Wed 30 Sep 2026 23:59."
    )
    # Nothing tells Deimos the answer is short because of Orbit.
    for words in ("range_fully_read", "unread_from", "incompleto", "sigue desde"):
        assert words not in body, (words, body)
    # And a reader can tell whether it received all of it: the count opens and
    # closes the answer, so a copy cut short downstream does not pass for whole.
    assert body.rstrip().endswith("End of read: 8 events.")


async def test_a_single_page_answer_costs_one_request(tmp_path):
    hub = FakeHub(device_replies=[page([event(0, OCTOBER_MS)])])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    assert not result.is_error, text_of(result)
    assert len(hub.device_requests) == 1


async def test_an_empty_range_is_an_answer_not_an_error(tmp_path):
    hub = FakeHub(device_replies=[page([])])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = calendar_body(result)
    assert not result.is_error, body
    assert event_lines(body) == []
    assert body.startswith("Agenda: 0 events from ")
    assert body.rstrip().endswith("End of read: 0 events.")


async def test_a_cursor_that_does_not_advance_is_not_followed_forever(tmp_path):
    """A Mac that keeps asking for the same page is a fault, named as one."""
    stuck = page([event(0, OCTOBER_MS)], after=(OCTOBER_MS, 0))
    hub = FakeHub(device_replies=[stuck, stuck, stuck])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith("invalid_response:"), body
    # The refusal names its cause and carries the positions, not only a code.
    assert f"[{OCTOBER_MS}, 0, 0]" in body and "without advancing" in body, body
    assert len(hub.device_requests) == 1


# ----------------------------------------------- an event split across pages


async def test_an_event_split_across_pages_comes_back_whole(tmp_path):
    invitees = [
        {
            "participant_url": f"mailto:persona{i}@example.invalid",
            "email": f"persona{i}@example.invalid",
            "role": "required",
            "response_status": "accepted",
            "participant_type": "person",
            "is_current_user": False,
        }
        for i in range(6)
    ]
    head = event(0, OCTOBER_MS, invitees_status="provided", invitees=invitees[:4])
    head["invitees_continued"] = True
    tail = event(0, OCTOBER_MS, invitees_status="provided", invitees=invitees[4:])
    # The Mac stopped inside event 0: the index stays on it and the invitee
    # says where its list goes on, as `calendar_page` in orbit-core sets it.
    hub = FakeHub(
        device_replies=[
            page([head], after=(OCTOBER_MS, 0, 4)),
            page([tail, event(1, OCTOBER_MS)]),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = calendar_body(result)
    assert not result.is_error, body
    following = hub.device_requests[1]["parameters"]
    assert (following["after_index"], following["after_invitee"]) == (0, 4)
    lines = event_lines(body)
    assert len(lines) == 2
    # One event, every guest of both fragments, in order; never a fragment.
    guests = [f"persona{i} (accepted)" for i in range(6)]
    assert "with @example.invalid: " + ", ".join(guests) in lines[0], lines[0]
    assert "invitees_continued" not in body
    assert body.rstrip().endswith("End of read: 2 events.")


async def test_d4_an_event_that_could_not_travel_is_named_by_itself(tmp_path):
    """Decision 4: the read arrives whole without it, and the answer says which
    event it was and why — dates, identifier, field, bytes against bytes."""
    result = page([event(i, OCTOBER_MS + i * 3_600_000) for i in range(3)])
    left_out = {
        "calendar": {"calendar_id": "calendar-1", "source_id": "account"},
        "calendar_item_id": "item-500",
        "start_unix_ms": OCTOBER_MS + 3 * DAY_MS,
        "end_unix_ms": OCTOBER_MS + 3 * DAY_MS + 3_600_000,
        "field": "title",
        "bytes": 120_000,
        "limit": 98_304,
    }
    payload = json.loads(result.body)
    payload["result"]["oversized"] = [left_out]
    hub = FakeHub(device_replies=[answered(payload["result"])])
    async with serving(tmp_path, hub) as (_client, server):
        answer = await ask(server)
    body = calendar_body(answer)
    assert not answer.is_error, body
    assert titles(body) == ["Evento 0", "Evento 1", "Evento 2"]
    assert "1 more events could not be delivered because of their size" in body.splitlines()[0]
    assert (
        "Sun 4 Oct 2026 00:00–Sun 4 Oct 2026 01:00 · id item-500 · calendar calendar-1: the title takes 120000 bytes and one page allows 98304."
    ) in body
    assert body.rstrip().endswith("End of read: 3 events.")


async def test_a_cursor_inside_one_event_must_still_move_forward(tmp_path):
    """Inside one event the invitee position is the progress; standing still is a fault."""
    head = event(0, OCTOBER_MS, invitees_status="provided", invitees=[])
    head["invitees_continued"] = True
    stuck = page([head], after=(OCTOBER_MS, 0, 4))
    hub = FakeHub(device_replies=[stuck, stuck, stuck])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    assert result.is_error, text_of(result)
    assert "invalid_response" in text_of(result)
    assert len(hub.device_requests) == 2


# ------------------------------------------- each missing condition as itself


@pytest.mark.parametrize(
    ("code", "must_say", "must_not_say"),
    [
        ("calendar_backend_unavailable", "EventKit", "permission"),
    ],
)
async def test_each_missing_condition_arrives_as_itself(tmp_path, code, must_say, must_not_say):
    hub = FakeHub(device_replies=[refused({"code": code}, "lo que diga el Mac")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith(f"{code}:"), body
    assert must_say in body, body
    assert must_not_say not in body.split("lo que diga el Mac")[0], body
    assert "End of read" not in body, body


@pytest.mark.parametrize(
    "refusal,must_say",
    [
        (
            {"code": "calendar_read_limit", "cause": {"kind": "page", "max_bytes": 786432}},
            "limits",
        ),
        (
            {
                "code": "calendar_read_limit",
                "cause": {
                    "kind": "fields",
                    "max_title_bytes": 98304,
                    "max_label_bytes": 1024,
                    "max_timezone_bytes": 128,
                },
            },
            "limits",
        ),
        (
            {
                "code": "read_interrupted",
                "cause": {"kind": "calendar_expired", "deadline_ms": 30000},
            },
            "interrupted",
        ),
        ({"code": "helper_refused", "helper_code": "calendar_read_failed"}, "EventKit"),
        ({"code": "helper_refused", "helper_code": "calendar_invalidated"}, "changed"),
        (
            {"code": "helper_reply_invalid", "field": "calendar", "expected": "page"},
            "violates the contract",
        ),
        (
            {"code": "read_interrupted", "cause": {"kind": "calendar_generation_changed"}},
            "generation",
        ),
        (
            {
                "code": "helper_reply_invalid",
                "field": "response",
                "expected": "reply_before_deadline",
            },
            "violates the contract",
        ),
        (
            {"code": "read_interrupted", "cause": {"kind": "calendar_helper_stopped"}},
            "Orbit helper is stopped",
        ),
        (
            {"code": "read_interrupted", "cause": {"kind": "calendar_reply_replaced"}},
            "another agenda read",
        ),
        ({"code": "helper_refused", "helper_code": "calendar_clock_unavailable"}, "clock"),
    ],
)
async def test_calendar_failure_is_named_and_never_an_empty_agenda(tmp_path, refusal, must_say):
    code = refusal["code"]
    notice = "Frase del Mac con sus números: 786432 bytes."
    hub = FakeHub(device_replies=[refused(refusal, notice)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith(f"{code}:"), body
    assert must_say in body, body
    assert "invalid response" not in body and "Orbit refused the request" not in body, body
    assert notice in body, body
    assert "End of read" not in body, body


async def test_a_mac_that_is_not_connected_is_named_as_that(tmp_path):
    hub = FakeHub(device_replies=[hub_rejected(404, "device_unavailable")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith("device_unavailable: The Mac is not connected to Orbit."), body


async def test_a_mac_that_is_not_paired_is_named_as_that(tmp_path):
    hub = FakeHub(devices=[])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar",
            {"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS, "device": "desconocido"},
        )
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith("unknown_device:"), body
    assert hub.device_requests == []


@pytest.mark.parametrize(
    "cause", ["calendar_read_restarted", "capture_stopped", "helper_restarted"]
)
async def test_typed_interruption_restarts_the_walk_without_losing_events(tmp_path, cause):
    interruption = Response(
        content=json.dumps(
            {
                "state": "refused",
                "code": "read_interrupted",
                "refusal": {
                    "code": "read_interrupted",
                    "retry": {"kind": "check_outcome"},
                    "cause": {"kind": cause},
                },
            }
        ).encode(),
        status_code=403,
        media_type="application/json",
    )
    hub = FakeHub(
        device_replies=[
            page([event(0, OCTOBER_MS)], after=(OCTOBER_MS, 1)),
            interruption,
            page([event(0, OCTOBER_MS)], after=(OCTOBER_MS, 1), read_id=READ_ID[:-4] + "dddd"),
            page([event(1, OCTOBER_MS + DAY_MS)], read_id=READ_ID[:-4] + "dddd"),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await ask(server)
    assert not result.is_error, text_of(result)
    assert titles(text_of(result)) == ["Evento 0", "Evento 1"]
    assert "read_id" not in hub.device_requests[2]["parameters"]
    assert len(hub.device_requests) == 4
