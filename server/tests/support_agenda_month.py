"""Synthetic months of agenda, in the proportions measured on a real agenda.

Names and domains retain the original character lengths, so the pinned size
checks measure the same distribution with invented data.

The measured proportions are:
28 calendars; eight events a working day (typical) or seventeen (busy), one a
day at weekends, five weeks; guests drawn from `TYPICAL` (mean 6) or `HEAVY`
(mean 17); six events in ten with a meeting URL; EventKit-shaped identifiers.
A typical month is 210 events and a busy one 435. Seeded, so the sizes the
tests pin are the same on every machine.
"""

import random
import uuid

WEEK_START_MS = 1_789_257_600_000
HOUR_MS = 3_600_000
DAY_MS = 24 * HOUR_MS
TYPICAL = (0, 2, 3, 4, 6, 8, 10, 15)
HEAVY = (4, 8, 10, 12, 15, 20, 30, 40)
WORDS = [
    "la",
    "de",
    "que",
    "el",
    "en",
    "y",
    "a",
    "los",
    "se",
    "del",
    "las",
    "un",
    "por",
    "con",
    "no",
    "una",
    "su",
    "para",
    "es",
    "al",
    "lo",
    "como",
    "más",
    "revisión",
    "equipo",
    "plan",
    "sprint",
    "cliente",
    "datos",
]


def _uuid(rng):
    return str(uuid.UUID(int=rng.getrandbits(128), version=4))


def calendars(rng, count=28):
    return [
        {"calendar_id": _uuid(rng).upper(), "source_id": _uuid(rng).upper()} for _ in range(count)
    ]


def invitee(rng, current):
    first = rng.choice(
        [
            "melinda",
            "benito",
            "darion",
            "nerea",
            "fabian",
            "luz",
            "dalia",
            "noemi",
            "celia",
            "lidia",
        ]
    )
    last = rng.choice(
        ["perez", "castillo", "soto", "flores", "molina", "rojas", "navarro", "campos"]
    )
    domain = rng.choice(["example.test", "staff.test", "x.example", "contact.example"])
    email = f"{first}.{last}@{domain}"
    return {
        "name": f"{first}.{last}",
        "participant_url": f"mailto:{email}",
        "email": email,
        "role": rng.choice(["required", "optional", "chair"]),
        "response_status": rng.choice(["accepted", "tentative", "pending", "declined"]),
        "participant_type": "person",
        "is_current_user": current,
    }


def text(rng, target):
    out = []
    while len(" ".join(out)) < target:
        out.append(rng.choice(WORDS))
    return " ".join(out)


def occurrence(rng, calendar, start, minutes, guests):
    with_url = rng.randrange(10) < 6
    value = {
        "observation_id": _uuid(rng),
        "calendar": calendar,
        "calendar_item_id": _uuid(rng).upper(),
        "event_id": f"{_uuid(rng).upper()}:{_uuid(rng).upper()}",
        "start_unix_ms": start,
        "end_unix_ms": start + minutes * 60_000,
        "event_timezone": "Europe/Madrid",
        "interpreted_timezone": "Europe/Madrid",
        "is_floating": False,
        "is_all_day": False,
        "is_detached": False,
        "title": f"[{rng.choice(['AcmeSoft - PlanningGroup', 'Explorer', 'Team Space', 'Review', '1:1'])}] {text(rng, 22)}",
        "status": "confirmed",
        "last_modified_unix_ms": start - 7 * DAY_MS,
        "invitees": [invitee(rng, i == 0) for i in range(guests)],
        "invitees_status": "provided",
        "event_url_status": "provided",
    }
    if rng.randrange(2) == 0:
        value["original_occurrence_unix_ms"] = start
    if with_url:
        value["event_url"] = (
            f"https://meet.google.com/{rng.getrandbits(12):03x}-{rng.getrandbits(16):04x}-{rng.getrandbits(12):03x}"
        )
    else:
        value["event_url_status"] = "not_provided"
    return value


def month(seed, per_day, mix):
    rng = random.Random(seed)
    cals = calendars(rng)
    events = []
    for week in range(5):
        for day in range(7):
            count = per_day if 1 <= day <= 5 else 1
            for slot in range(count):
                start = WEEK_START_MS + (week * 7 + day) * DAY_MS + 8 * HOUR_MS + slot * 40 * 60_000
                events.append(occurrence(rng, rng.choice(cals), start, 30, rng.choice(mix)))
    return events
