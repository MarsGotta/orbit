"""A meeting's series as the Mac reads and sets it.

`GET /v1/series/previous-meeting`: the earlier meeting of a calendar series and
the tasks of its note, done or not, for «Pendiente de la última vez» in Hoy.
`PUT /v1/recordings/{id}/series`: the template and the dictionary of the
meeting's series saved together, in one transaction, for «Usar lo mismo en toda
la serie». Real routes over the real store; notes written by the real worker
with a scripted model (`support_series.Bench`). Invented meetings and people.
"""

import json
from pathlib import Path
from uuid import uuid4

import pytest
from conftest import device
from support_app import client
from support_notes import note_answer
from support_series import SERIES, Bench, recording

from orbit_hub.meetings import series
from orbit_hub.notes.series_key import MAX_SERIES_KEY_BYTES

NOW = 2000.0
DAY_MS = 86_400_000
# The Monday of the next meeting of the series, at 10:00 UTC.
NEXT_MS = 1_792_000_800_000
OTHER_SERIES = "9A8B7C6D-0002-4E3F-8A1B-0FEDCBA98765"


def placed(store, owner: str, recording_id: str, started_at_ms: int, event: dict | None):
    """The closing manifest as a Mac sends it for an event at that time."""
    context = f"manifest:{owner}:{recording_id}"
    with store.transaction() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner, recording_id),
        ).fetchone()
        manifest = json.loads(store.decrypt(row["body"], context))
        manifest.update(
            started_at_ms=started_at_ms,
            finalized_at_ms=started_at_ms + 45 * 60_000,
            title=event["title"] if event else "Grabación suelta",
        )
        if event is None:
            manifest.pop("event", None)
        else:
            manifest["event"] = event
        db.execute(
            "UPDATE recording_manifests SET body=? WHERE device_id=? AND recording=?",
            (store.encrypt(json.dumps(manifest).encode(), context), owner, recording_id),
        )


def event(starts_at_ms: int, series: str = SERIES, title: str = "Revisión de diseño"):
    return {"title": title, "starts_at_ms": starts_at_ms, "calendar_item_id": series}


TASKS = [
    {"owner": "Lucía", "text": "Enviar el acta", "due": "viernes"},
    {"owner": "Bruno", "text": "Revisar el presupuesto", "due": ""},
]


@pytest.fixture
def bench(environment):
    _, store, _ = environment
    owner = device(store)
    bench = Bench(store, owner["device_id"], NOW)
    bench.token, bench.environment = owner["token"], environment
    yield bench
    bench.close()


def meeting(bench, started_at_ms: int, meeting_event: dict | None, *, tasks=None) -> str:
    """A finished meeting at that start; with `tasks`, its note is written."""
    identity = recording(bench.store, bench.owner)
    placed(bench.store, bench.owner, identity, started_at_ms, meeting_event)
    if tasks is not None:
        bench.model.script("note", {**note_answer(), "action_items": tasks})
        bench.worker.repository.request(bench.owner, identity, NOW, True)
    return identity


def previous(http, headers, series=SERIES, before_ms=NEXT_MS, title_digest=None):
    params = {"calendar_item_id": series, "before_ms": before_ms}
    if title_digest is not None:
        params["title_digest"] = title_digest
    return http.get("/v1/series/previous-meeting", params=params, headers=headers)


# Proposal 5: «Pendiente de la última vez».


async def test_the_previous_meeting_of_the_series_brings_its_tasks_done_or_not(bench):
    older = meeting(bench, NEXT_MS - 14 * DAY_MS, event(NEXT_MS - 14 * DAY_MS), tasks=TASKS[:1])
    last = meeting(bench, NEXT_MS - 7 * DAY_MS, event(NEXT_MS - 7 * DAY_MS), tasks=TASKS)
    # Later than the last one, but of another series or of none.
    meeting(bench, NEXT_MS - DAY_MS, event(NEXT_MS - DAY_MS, OTHER_SERIES), tasks=TASKS)
    meeting(bench, NEXT_MS - DAY_MS, None, tasks=TASKS)
    # The meeting being asked about, and one after it, are not «before».
    meeting(bench, NEXT_MS, event(NEXT_MS), tasks=TASKS)
    assert await bench.drain() == 5
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        answer = previous(http, headers)
        assert answer.status_code == 200, answer.text
        body = answer.json()
        assert body["calendar_item_id"] == SERIES and body["before_ms"] == NEXT_MS
        found = body["previous"]
        assert found["recording_id"] == last
        assert found["title"] == "Revisión de diseño"
        assert found["started_at_ms"] == NEXT_MS - 7 * DAY_MS
        note = found["note"]
        assert note["state"] == "ready"
        assert note["short_summary"] == "Resumen corto."
        items = note["action_items"]
        assert [(i["owner"], i["text"], i["due"], i["done"]) for i in items] == [
            ("Lucía", "Enviar el acta", "viernes", False),
            ("Bruno", "Revisar el presupuesto", None, False),
        ]
        # Marking one done is the note's own edit, as in its Actions tab.
        edited = http.put(
            f"/v1/recordings/{last}/note/edits",
            json={"base_revision": note["revision"], "done": {items[0]["id"]: True}},
            headers=headers,
        )
        assert edited.status_code == 200, edited.text
        again = previous(http, headers).json()["previous"]["note"]
        assert [i["done"] for i in again["action_items"]] == [True, False]
        assert again["revision"] == note["revision"] + 1
        # Before the last one, the one before it.
        earlier = previous(http, headers, before_ms=NEXT_MS - 7 * DAY_MS).json()
        assert earlier["previous"]["recording_id"] == older


async def test_a_series_without_an_earlier_meeting_says_so(bench):
    meeting(bench, NEXT_MS - 7 * DAY_MS, event(NEXT_MS - 7 * DAY_MS, OTHER_SERIES), tasks=TASKS)
    meeting(bench, NEXT_MS + DAY_MS, event(NEXT_MS + DAY_MS), tasks=TASKS)
    await bench.drain()
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        answer = previous(http, headers)
        assert answer.status_code == 200, answer.text
        # Empty, not unknown: the key is present and null.
        assert answer.json() == {
            "calendar_item_id": SERIES,
            "before_ms": NEXT_MS,
            "previous": None,
        }
        # Another device's meetings of the same series are not this one's.
        stranger = device(bench.store, "otro")
        other = previous(http, {"Authorization": "Bearer " + stranger["token"]}, OTHER_SERIES)
        assert other.json()["previous"] is None


async def test_a_previous_note_not_written_has_no_tasks_yet_rather_than_none(bench):
    waiting = meeting(bench, NEXT_MS - 7 * DAY_MS, event(NEXT_MS - 7 * DAY_MS))
    bench.worker.repository.request(bench.owner, waiting, NOW, True)
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        note = previous(http, headers).json()["previous"]["note"]
        # Asked for and not written: this app has no model, so it is waiting,
        # being tried or failed, whichever its worker reached.
        assert note["state"] in {"waiting_transcript", "queued", "generating", "failed"}
        # Unknown, not «no tasks».
        assert note["action_items"] is None and note["short_summary"] is None
        unrequested = meeting(bench, NEXT_MS - DAY_MS, event(NEXT_MS - DAY_MS))
        found = previous(http, headers).json()["previous"]
        assert found["recording_id"] == unrequested and found["note"] is None


def test_a_previous_meeting_query_is_refused_with_the_field_and_its_numbers(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        missing = http.get(
            "/v1/series/previous-meeting", params={"before_ms": NEXT_MS}, headers=headers
        )
        assert missing.status_code == 400
        assert missing.json()["code"] == "invalid_parameters"
        assert missing.json()["refusal"]["field"] == "calendar_item_id"
        long = "x" * (MAX_SERIES_KEY_BYTES + 1)
        too_long = previous(http, headers, series=long)
        assert too_long.status_code == 400
        refusal = too_long.json()["refusal"]
        assert refusal["field"] == "calendar_item_id"
        assert (refusal["bytes"], refusal["max_bytes"]) == (
            MAX_SERIES_KEY_BYTES + 1,
            MAX_SERIES_KEY_BYTES,
        )
        no_time = http.get(
            "/v1/series/previous-meeting", params={"calendar_item_id": SERIES}, headers=headers
        )
        assert no_time.status_code == 400
        assert no_time.json()["refusal"]["field"] == "before_ms"
        assert previous(http, {}).status_code == 401


# «Pendiente de la última vez» by the title, when the series has no earlier
# meeting: one recorded by hand with the event's title carries no series. The
# title travels as the digest of its words (`title_digest`), never as text.

TITLES = json.loads(
    (
        Path(__file__).resolve().parents[2] / "contracts/fixtures/series-title-keys-v1.json"
    ).read_text()
)["cases"]


def test_title_keys_are_the_macs(bench):
    """The same words and digest as `orbit_core::notes::title_key`, from one fixture."""
    for case in TITLES:
        assert series.title_key(case["title"]) == case["key"], case
        assert series.title_digest(case["title"]) == case["digest"], case


async def test_without_an_earlier_meeting_in_the_series_the_same_title_brings_its_tasks(bench):
    by_hand = meeting(bench, NEXT_MS - 7 * DAY_MS, None, tasks=TASKS)
    retitled(bench.store, bench.owner, by_hand, "Revisión de diseño 41")
    # Another title, and a later one that is not before the next meeting.
    other = meeting(bench, NEXT_MS - DAY_MS, None, tasks=TASKS)
    retitled(bench.store, bench.owner, other, "Comida de equipo")
    assert await bench.drain() == 2
    digest = series.title_digest("Revisión de diseño")
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        found = previous(http, headers, title_digest=digest).json()["previous"]
        assert found["recording_id"] == by_hand
        assert [i["text"] for i in found["note"]["action_items"]] == [
            "Enviar el acta",
            "Revisar el presupuesto",
        ]
        # A Mac that sends no title gets what it always got.
        assert previous(http, headers).json()["previous"] is None


async def test_the_series_wins_over_a_later_meeting_of_the_same_title(bench):
    linked = meeting(bench, NEXT_MS - 14 * DAY_MS, event(NEXT_MS - 14 * DAY_MS), tasks=TASKS[:1])
    by_hand = meeting(bench, NEXT_MS - 7 * DAY_MS, None, tasks=TASKS)
    retitled(bench.store, bench.owner, by_hand, "Revisión de diseño")
    await bench.drain()
    headers = {"Authorization": "Bearer " + bench.token}
    digest = series.title_digest("Revisión de diseño")
    with client(bench.environment) as http:
        found = previous(http, headers, title_digest=digest).json()["previous"]
        assert found["recording_id"] == linked


def test_a_title_key_that_is_not_a_digest_is_refused_with_its_field(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        refused = previous(http, headers, title_digest="Revisión de diseño")
        assert refused.status_code == 400
        assert refused.json()["refusal"]["field"] == "title_digest"


def retitled(store, owner: str, recording_id: str, title: str) -> None:
    """The manifest's own title, as a Mac sends it for a recording started by hand."""
    context = f"manifest:{owner}:{recording_id}"
    with store.transaction() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner, recording_id),
        ).fetchone()
        manifest = json.loads(store.decrypt(row["body"], context))
        manifest["title"] = title
        db.execute(
            "UPDATE recording_manifests SET body=? WHERE device_id=? AND recording=?",
            (store.encrypt(json.dumps(manifest).encode(), context), owner, recording_id),
        )


# Proposal 9: «Usar lo mismo en toda la serie».


def dictionary(http, headers, name="Proyecto Albatros") -> str:
    identity = str(uuid4())
    saved = http.post(
        "/v1/meeting-dictionaries/orders",
        json={"action": "save", "dictionary_id": identity, "name": name},
        headers=headers,
    )
    assert saved.status_code == 200, saved.text
    return identity


def state(http, headers, recording_id):
    links = http.get("/v1/note-template-links", headers=headers).json()["links"]
    dictionaries = http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"]
    document = http.get(f"/v1/recordings/{recording_id}", headers=headers).json()
    return (
        [(link["calendar_item_id"], link["template_id"]) for link in links],
        {d["id"]: [s["calendar_item_id"] for s in d["series"]] for d in dictionaries},
        document["dictionary"],
    )


def use_for_series(http, headers, recording_id, template_id, dictionary_id):
    return http.put(
        f"/v1/recordings/{recording_id}/series",
        json={"template_id": template_id, "dictionary_id": dictionary_id},
        headers=headers,
    )


def test_template_and_dictionary_are_saved_together_for_the_series(bench):
    identity = meeting(bench, NEXT_MS - 7 * DAY_MS, event(NEXT_MS - 7 * DAY_MS))
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        chosen, other = dictionary(http, headers), dictionary(http, headers, "Otro")
        # The meeting had its own dictionary: the series' now applies to it.
        selected = http.post(
            "/v1/meeting-dictionaries/orders",
            json={
                "action": "select",
                "recording_id": identity,
                "dictionary_id": other,
                "use_for_series": False,
            },
            headers=headers,
        )
        assert selected.status_code == 200, selected.text
        saved = use_for_series(http, headers, identity, "project", chosen)
        assert saved.status_code == 200, saved.text
        assert saved.json() == {
            "recording_id": identity,
            "calendar_item_id": SERIES,
            "template_id": "project",
            "dictionary_id": chosen,
            "dictionary_ids": [chosen],
        }
        links, series, meeting_dictionary = state(http, headers, identity)
        assert links == [(SERIES, "project")]
        assert series == {chosen: [SERIES], other: []}
        assert meeting_dictionary == {
            "dictionary_id": chosen,
            "dictionary_ids": [chosen],
            "source": "series",
            "series": {"calendar_item_id": SERIES, "title": "Revisión de diseño"},
        }
        # No dictionary for the series: released, the template stays.
        released = use_for_series(http, headers, identity, "general", None)
        assert released.status_code == 200, released.text
        links, series, meeting_dictionary = state(http, headers, identity)
        assert links == [(SERIES, "general")]
        assert series == {chosen: [], other: []}
        assert meeting_dictionary["dictionary_id"] is None


def test_a_refused_half_leaves_the_other_unsaved_and_names_its_cause(bench, monkeypatch):
    identity = meeting(bench, NEXT_MS - 7 * DAY_MS, event(NEXT_MS - 7 * DAY_MS))
    loose = meeting(bench, NEXT_MS - DAY_MS, None)
    headers = {"Authorization": "Bearer " + bench.token}
    with client(bench.environment) as http:
        chosen = dictionary(http, headers)
        before = state(http, headers, identity)

        unknown_dictionary = use_for_series(http, headers, identity, "project", str(uuid4()))
        assert unknown_dictionary.status_code == 404
        assert unknown_dictionary.json()["code"] == "dictionary_not_found"
        assert state(http, headers, identity) == before

        unknown_template = use_for_series(http, headers, identity, "no-existe", chosen)
        assert unknown_template.status_code == 404
        assert unknown_template.json()["code"] == "note_template_not_found"
        assert unknown_template.json()["refusal"]["template_id"] == "no-existe"
        assert state(http, headers, identity) == before

        no_series = use_for_series(http, headers, loose, "project", chosen)
        assert no_series.status_code == 409
        assert no_series.json()["code"] == "meeting_series_missing"
        assert state(http, headers, identity) == before

        from orbit_hub.notes import custom_templates

        monkeypatch.setattr(custom_templates, "MAX_TEMPLATE_LINKS", 0)
        full = use_for_series(http, headers, identity, "project", chosen)
        assert full.status_code == 409
        assert full.json()["code"] == "note_template_links_full"
        assert full.json()["refusal"]["links"] == 0
        assert full.json()["refusal"]["max_links"] == 0
        assert state(http, headers, identity) == before
        monkeypatch.undo()

        from orbit_hub.meetings import dictionaries

        monkeypatch.setattr(dictionaries, "MAX_SERIES", 0)
        series_full = use_for_series(http, headers, identity, "project", chosen)
        assert series_full.status_code == 409
        assert series_full.json()["code"] == "dictionary_series_full"
        assert (
            series_full.json()["refusal"]["series"],
            series_full.json()["refusal"]["max_series"],
        ) == (1, 0)
        assert state(http, headers, identity) == before

        stranger = str(uuid4())
        assert use_for_series(http, headers, stranger, "general", None).status_code == 404
        invalid = http.put(
            f"/v1/recordings/{identity}/series", json={"template_id": "general"}, headers=headers
        )
        assert invalid.status_code == 400
        assert invalid.json()["refusal"]["field"] == "dictionary_id"
