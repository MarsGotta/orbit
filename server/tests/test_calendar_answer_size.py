"""How many characters Deimos receives for a month of agenda.

Hermes, not Orbit, bounds what reaches Deimos (measured by the reviewer in the
installed Hermes): a tool result stays whole in Deimos's context up to 50,000
characters (`tools/budget_config.py:20`, `DEFAULT_MCP_RESULT_SIZE_CHARS`); a
longer one is saved to a file and Deimos sees a 1,500-character preview and the
path (`tools/tool_result_storage.py:247`); past 2,000,000 it is cut in the
middle (`tools/mcp_tool_content.py:24`).

So the answer is written to say everything the Mac's JSON said in far fewer
characters, and its first lines are the summary a 1,500-character preview can
show. These cases pin the sizes for a typical and a busy month in the
measured proportions (`support_agenda_month.py`), check that nothing of
any event was dropped to get there, and that the preview carries the summary.
"""

import json

import pytest
from support_agenda_month import DAY_MS, HEAVY, TYPICAL, WEEK_START_MS, month

from orbit_mcp import calendar as agenda

HERMES_WHOLE_RESULT_CHARS = 50_000
HERMES_PREVIEW_CHARS = 1_500
HERMES_HARD_CUT_CHARS = 2_000_000
SEED = 2026_0925


def reading_of(events):
    reading = agenda.Reading(WEEK_START_MS, WEEK_START_MS + 35 * DAY_MS)
    reading.take(
        {
            "events": events,
            "omitted": [],
            "read_at_unix_ms": WEEK_START_MS,
            "system_timezone": "Europe/Madrid",
            "provider_sync_status": "unknown",
            "enumeration": "bounded_local_read",
        }
    )
    return reading.answer()


# (name, events a working day, guest mix, events, guests, JSON chars, text chars)
MONTHS = [
    ("típico", 8, TYPICAL, 210, 1_171, 402_366, 100_446),
    ("cargado", 17, HEAVY, 435, 7_328, 1_865_589, 367_160),
]


@pytest.mark.parametrize(
    ("name", "per_day", "mix", "count", "guests", "as_json", "as_text"), MONTHS
)
def test_a_month_of_agenda_in_characters(name, per_day, mix, count, guests, as_json, as_text):
    events = month(SEED, per_day, mix)
    assert len(events) == count
    assert sum(len(event["invitees"]) for event in events) == guests
    reading = reading_of(events)
    # What the answer weighed when it was the Mac's JSON (before this change).
    legacy = len(json.dumps(reading, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    text = agenda.render(reading)
    assert (legacy, len(text)) == (as_json, as_text), name
    # Past what Hermes keeps whole either way: the preview has to carry it.
    assert len(text) > HERMES_WHOLE_RESULT_CHARS
    assert len(text) < HERMES_HARD_CUT_CHARS
    preview = text[:HERMES_PREVIEW_CHARS]
    first, per_day_line = preview.splitlines()[:2]
    assert first.startswith(f"Agenda: {count} eventos del ")
    assert "en 28 calendarios" in first
    assert per_day_line.startswith("Por día: ") and per_day_line.endswith(".")
    assert text.rstrip().endswith(f"Fin de la lectura: {count} eventos.")


@pytest.mark.parametrize(("name", "per_day", "mix"), [(m[0], m[1], m[2]) for m in MONTHS])
def test_nothing_of_any_event_was_left_out_to_get_there(name, per_day, mix):
    events = month(SEED, per_day, mix)
    text = agenda.render(reading_of(events))
    lines = [line for line in text.splitlines() if " [C" in line and " · id " in line]
    assert len(lines) == len(events), name
    by_identifier = {line.rsplit(" · id ", 1)[1].split(" / ")[0]: line for line in lines}
    for event in events:
        line = by_identifier[event["calendar_item_id"]]
        assert event["title"] in line
        assert event["event_id"] in line
        if "event_url" in event:
            assert event["event_url"] in line
        for guest in event["invitees"]:
            local, domain = guest["email"].split("@")
            assert f"@{domain}: " in line and local in line, (guest, line)
