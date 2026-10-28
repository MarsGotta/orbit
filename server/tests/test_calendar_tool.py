from support_orbit_mcp import serving

"""Deimos asks about a date, and Orbit reads it then.

The user asked, days running, for Deimos to see **all** of their calendar from
the moment they grant the macOS permission. A stored copy landed and stayed;
Deimos still answered them with this:

    «La que me acaba de devolver abarca del 20 al 27 de septiembre y contiene
     44 eventos. Dentro de esa copia puedo consultar tu agenda, pero no irme,
     por ejemplo, a diciembre.»

These cases cover the hub and MCP halves of the fix: the bridge learns a third
capability, and Deimos gets a tool that reads the Mac at the moment it asks.
The Mac half lives in ``crates/orbit-daemon/src/daemon/calendar/``.

A later change took the rest of the way: no stored copy, no second agenda tool, and any
range in one call (``test_calendar_walk.py``). What stays here is what
this fix established and still holds — the range travels with the question, a
refusal is never an empty agenda, and a malformed range never reaches the Mac.
"""

import json

from fastapi.responses import Response
from support_orbit_mcp import (
    BRIDGE_RESULT,
    FakeHub,
    answered,
    text_of,
)

# 1 December 2026, 00:00 UTC — the month Deimos could not reach.
DECEMBER_MS = 1_796_083_200_000
DAY_MS = 86_400_000

READING = {
    "read_id": "7a1b0000-0000-4000-8000-00000000dddd",
    "requested": {"start_unix_ms": DECEMBER_MS, "end_unix_ms": DECEMBER_MS + 31 * DAY_MS},
    "read_at_unix_ms": DECEMBER_MS - 1_000,
    "system_timezone": "Europe/Madrid",
    "provider_sync_status": "unknown",
    "enumeration": "bounded_local_read",
    "events": [
        {
            "observation_id": "55555555-5555-4555-8555-555555555555",
            "calendar": {"calendar_id": "local", "source_id": "account"},
            "calendar_item_id": "item-0",
            "start_unix_ms": DECEMBER_MS + 3_600_000,
            "end_unix_ms": DECEMBER_MS + 7_200_000,
            "interpreted_timezone": "Europe/Madrid",
            "is_floating": True,
            "is_all_day": False,
            "is_detached": False,
            "title": "Cena de diciembre",
            "status": "confirmed",
            "invitees_status": "not_provided",
            "event_url_status": "not_provided",
        }
    ],
    "events_returned": 1,
    "omitted": [],
}


async def test_a_december_question_reaches_the_mac_with_december_in_it(tmp_path):
    """The range travels with the question instead of being Orbit's to choose."""
    hub = FakeHub(device_replies=[answered(READING)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar",
            {"start_ms": DECEMBER_MS, "end_ms": DECEMBER_MS + 31 * DAY_MS},
        )
    assert not result.is_error, text_of(result)
    assert hub.device_requests[0]["capability"] == "calendar_read"
    # This line used to read `str(DECEMBER_MS)`, which is precisely the
    # shape the Mac refuses — the fake bridge never ran that parsing, so the
    # case stayed green while the tool failed every call. The pair is held
    # together in `test_bridge_contract.py` now.
    assert hub.device_requests[0]["parameters"]["start_ms"] == DECEMBER_MS
    body = text_of(result)
    assert "Cena de diciembre" in body
    # The answer says when the Mac actually read, so nobody has to guess
    # whether they are holding something six hours old.
    # In the Mac's zone, in the summary line (00:59:59 there on 1 Dec).
    assert "read on the Mac at Tue 1 Dec 2026 00:59." in body.splitlines()[0]


def refused_with(code: str, notice: str) -> Response:
    """The typed refusal envelope, carrying its named cause and detail."""
    return Response(
        content=json.dumps(
            {
                "protocol": BRIDGE_RESULT,
                "state": "refused",
                "refusal": {"code": code, "detail": notice, "retry": {"kind": "after_fix"}},
            },
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode(),
        status_code=403,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


async def test_a_refusal_never_arrives_looking_like_an_empty_agenda(tmp_path):
    """Whatever the cause, a refusal is an error result and carries no events."""
    hub = FakeHub(
        device_replies=[
            refused_with(
                "calendar_permission_required",
                "Orbit no tiene acceso de lectura completo a tus Calendarios en macOS.",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": DECEMBER_MS, "end_ms": DECEMBER_MS + DAY_MS}
        )
    body = text_of(result)
    assert result.is_error, body
    assert '"events"' not in body, body
    assert "NOT mean the agenda is empty" in body, body


async def test_invalid_ranges_are_refused_before_they_reach_the_mac(tmp_path):
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        for start, end in [
            (DECEMBER_MS, DECEMBER_MS),
            (DECEMBER_MS, DECEMBER_MS - DAY_MS),
        ]:
            result = await server.call_tool("orbit_calendar", {"start_ms": start, "end_ms": end})
            assert result.is_error, text_of(result)
    assert hub.device_requests == []


async def test_calendar_explains_cached_events_and_unknown_sync_even_with_events(tmp_path):
    hub = FakeHub(device_replies=[answered(READING)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": DECEMBER_MS, "end_ms": DECEMBER_MS + 31 * DAY_MS}
        )
    body = text_of(result)
    assert not result.is_error, body
    assert "Cena de diciembre" in body
    assert "what macOS has stored" in body
    assert "cannot know whether it is current" in body
    assert "account has stopped syncing" in body
