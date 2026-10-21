"""P9-21: the fields the Mac reads of each event reach Deimos.

El hub conserva la respuesta tal cual. Los modelos del consumidor orbit-mcp
validan estos campos y el renderizador de agenda debe decirlos a Deimos.

"""

from pydantic import TypeAdapter

from orbit_mcp import calendar as agenda
from orbit_mcp.bridge import RESULT_MODELS

START = 1_792_058_400_000  # Wednesday 14 October 2026, 10:00 UTC


def _event(**extra):
    event = {
        "observation_id": "00000000-0000-4000-8000-000000000912",
        "calendar": {"calendar_id": "work", "source_id": "icloud"},
        "calendar_item_id": "item-911",
        "start_unix_ms": START,
        "end_unix_ms": START + 3_600_000,
        "event_timezone": "Europe/Madrid",
        "interpreted_timezone": "Europe/Madrid",
        "is_floating": False,
        "is_all_day": False,
        "is_detached": False,
        "title": "Revisión semanal",
        "status": "confirmed",
        "invitees_status": "not_provided",
        "event_url_status": "not_provided",
    }
    event.update(extra)
    return event


NEW_FIELDS = {
    "location": "Sala 3",
    "notes": "Orden del día:\nrevisar el plan",
    "organizer": {
        "name": "Ada",
        "participant_url": "mailto:ada@example.test",
        "email": "ada@example.test",
        "role": "chair",
        "response_status": "accepted",
        "participant_type": "person",
        "is_current_user": False,
    },
    "availability": "tentative",
    "recurrence": {
        "frequency": "weekly",
        "interval": 2,
        "days_of_week": [{"day": "wednesday"}, {"day": "friday", "week": -1}],
        "end": {"count": 10},
        "rules_count": 2,
    },
    "can_edit": False,
}


def _result(*events):
    return {
        "read_id": "00000000-0000-4000-8000-000000000911",
        "requested": {"start_unix_ms": START, "end_unix_ms": START + 86_400_000},
        "read_at_unix_ms": START,
        "system_timezone": "Europe/Madrid",
        "provider_sync_status": "unknown",
        "enumeration": "bounded_local_read",
        "events_returned": len(events),
        "events": list(events),
        "omitted": [],
    }


def _validated_by_mcp(raw):
    """What `ws.py` forwards: the validated model, dumped without nulls."""
    model = TypeAdapter(RESULT_MODELS["calendar_read"]).validate_python(raw)
    return model.model_dump(exclude_none=True)


def test_the_mcp_model_keeps_every_new_event_field():
    forwarded = _validated_by_mcp(_result(_event(**NEW_FIELDS)))
    event = forwarded["events"][0]
    for name, value in NEW_FIELDS.items():
        assert event[name] == value, name
    left_out = _validated_by_mcp(
        _result(_event(notes_left_out={"bytes": 40_000, "limit": 32_768}))
    )["events"][0]
    assert left_out["notes_left_out"] == {"bytes": 40_000, "limit": 32_768}
    # A Mac from before sends none of them: absent stays absent.
    assert "can_edit" not in _validated_by_mcp(_result(_event()))["events"][0]


def _rendered(**extra):
    reading = agenda.Reading(START, START + 86_400_000)
    reading.take(
        {
            "events": [_event(**extra)],
            "omitted": [],
            "read_at_unix_ms": START,
            "system_timezone": "Europe/Madrid",
            "provider_sync_status": "unknown",
            "enumeration": "bounded_local_read",
        }
    )
    return agenda.render(reading.answer())


def test_deimos_reads_the_new_fields_in_the_event_line():
    text = _rendered(**NEW_FIELDS)
    line = next(line for line in text.splitlines() if "Revisión semanal" in line)
    assert "en Sala 3" in line
    assert "provisional" in line
    assert "organiza Ada" in line
    assert "se repite cada 2 semanas" in line
    assert "miércoles" in line and "último viernes" in line
    assert "10 veces" in line
    assert "2 reglas" in line
    assert "no se puede cambiar desde Orbit" in line
    assert "notas: Orden del día: / revisar el plan" in line
    left_out = _rendered(notes_left_out={"bytes": 40_000, "limit": 32_768})
    assert "notas de 40000 bytes, más de las 32768 que viajan" in left_out


def test_deimos_reads_the_reminders_of_an_event():
    """P9-22: the Mac now reads each event's reminders; Deimos says them."""
    forwarded = _validated_by_mcp(
        _result(_event(alarms_minutes=[10, 0, 1440], alarms_not_editable=1))
    )
    event = forwarded["events"][0]
    assert event["alarms_minutes"] == [10, 0, 1440]
    assert event["alarms_not_editable"] == 1
    line = next(
        line
        for line in _rendered(alarms_minutes=[10, 0, 60, 1440], alarms_not_editable=2).splitlines()
        if "avisos" in line
    )
    assert "avisos: al empezar, 10 min antes, 1 h antes, 1 día antes" in line
    assert "2 avisos más que Orbit no edita" in line
    assert "avisos" not in _rendered()
