"""Several dictionaries per meeting and per series, over the real routes and store.

Synthetic meetings, invented dictionaries and words only. The previous schema
below is the one hubs up to 0.10.1 created; the migration test builds a store
with it and opens it with this hub.
"""

import json
from pathlib import Path
from uuid import uuid4

from conftest import device
from support_app import client
from test_meeting_dictionaries import TEXTS, create, read
from test_meeting_views import meeting

from orbit_hub.meeting_final_pass import status
from orbit_hub.storage import Store

# The tables this changes, as hubs up to 0.10.1 created them:
# `recording_dictionaries` with one dictionary per meeting (its primary key the
# meeting), and `recording_final_passes` without the pass's progress.
PREVIOUS_SCHEMA = """
CREATE TABLE recording_dictionaries (
    device_id TEXT NOT NULL, recording TEXT NOT NULL, dictionary_id TEXT NOT NULL,
    PRIMARY KEY(device_id,recording));
CREATE TABLE recording_final_passes (
    device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
    state TEXT NOT NULL, error TEXT, PRIMARY KEY(device_id,recording));
"""

# The orders as the Mac writes them (`crates/orbit-core`, notes::dictionaries
# tests): the single-dictionary shapes of 0.10.1 and the lists.
ORDERS = (
    Path(__file__).resolve().parents[2]
    / "contracts"
    / "fixtures"
    / "hub"
    / "meeting-dictionary-orders.json"
)
ORDERS_RECORDING = "8c4f2a10-6b3d-4e5f-9a7b-1c2d3e4f5a6b"

SERIES = {"calendar_item_id": "serie-inventada", "title": "Revisión semanal"}


def post(http, headers, **payload):
    return http.post("/v1/meeting-dictionaries/orders", headers=headers, json=payload)


def ok(http, headers, **payload):
    response = post(http, headers, **payload)
    assert response.status_code == 200, response.text
    return response.json()


def document_dictionary(http, headers, recording):
    response = http.get(f"/v1/recordings/{recording}", headers=headers)
    assert response.status_code == 200, response.text
    return response.json()["dictionary"]


def listing(http, headers):
    return http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"]


def terms_of(http, headers, dictionary):
    found = next(d for d in listing(http, headers) if d["id"] == dictionary)
    return {(e["correct"], tuple(e["wrong"])) for e in found["terms"]}


def test_a_single_assignment_from_the_previous_schema_becomes_a_list_of_one(environment):
    settings, store, adapter = environment
    owners = [device(store, "uno"), device(store, "dos")]
    assigned = {}
    with client(environment) as http:
        for owner in owners:
            headers = {"Authorization": "Bearer " + owner["token"]}
            first, second = create(http, headers, "Ferretería Albar"), create(http, headers, "Lumo")
            for dictionary in (first, second, first):
                recording = meeting(store, owner["device_id"], texts=TEXTS)
                assigned[recording] = (owner, dictionary)
            # A series assignment lives in the dictionary's own body.
            in_series = meeting(store, owner["device_id"], texts=TEXTS, event=SERIES)
            ok(
                http,
                headers,
                action="select",
                recording_id=in_series,
                dictionary_id=second,
                use_for_series=True,
            )
            assigned[in_series] = (owner, None, second)
    # The store as the previous hub left it: one row per meeting.
    with store.transaction() as db:
        # A final pass for every meeting, queued as a 0.10.1 hub left it.
        passes = [
            (owner["device_id"], recording, "queued", None)
            for recording, (owner, *_) in assigned.items()
        ]
        db.execute("DROP TABLE recording_dictionaries")
        db.execute("DROP TABLE recording_final_passes")
        for statement in PREVIOUS_SCHEMA.split(";"):
            if statement.strip():
                db.execute(statement)
        db.executemany("INSERT INTO recording_final_passes VALUES (?,?,?,?)", passes)
        failed = (owners[0]["device_id"], next(iter(assigned)))
        db.execute(
            "UPDATE recording_final_passes SET state='failed',error=? "
            "WHERE device_id=? AND recording=?",
            ('{"code": "final_pass_timeout", "phase": "whisper_mic_local"}', *failed),
        )
        for recording, (owner, dictionary, *_) in assigned.items():
            if dictionary is not None:
                db.execute(
                    "INSERT INTO recording_dictionaries VALUES (?,?,?)",
                    (owner["device_id"], recording, dictionary),
                )
    before = sorted(tuple(row) for row in store._db.execute("SELECT * FROM recording_dictionaries"))
    assert len(before) == 6
    store.close()

    for opening in range(2):  # Opening it a second time migrates nothing again.
        reopened = Store(settings.data_dir, settings.key_file)
        try:
            after = sorted(
                tuple(row) for row in reopened._db.execute("SELECT * FROM recording_dictionaries")
            )
            assert after == before
            keys = [
                row["name"]
                for row in sorted(
                    reopened._db.execute("PRAGMA table_info(recording_dictionaries)"),
                    key=lambda row: row["pk"],
                )
                if row["pk"]
            ]
            assert keys == ["device_id", "recording", "dictionary_id"]
            # Every final pass is kept, with its state and error (the hub run
            # after the first opening works the queued ones, so only then).
            with reopened.read_snapshot() as db:
                states = {
                    (row["device_id"], row["recording"]): status(
                        db, row["device_id"], row["recording"]
                    )
                    for row in db.execute("SELECT device_id,recording FROM recording_final_passes")
                }
            assert len(states) == len(passes)
            if opening == 0:
                assert states.pop(failed) == {
                    "state": "failed",
                    "error": {"code": "final_pass_timeout", "phase": "whisper_mic_local"},
                }
                assert all(state == {"state": "queued", "error": None} for state in states.values())
            with client((settings, reopened, adapter)) as http:
                for recording, (owner, dictionary, *series) in assigned.items():
                    headers = {"Authorization": "Bearer " + owner["token"]}
                    chosen = document_dictionary(http, headers, recording)
                    if dictionary is not None:
                        assert chosen["dictionary_ids"] == [dictionary]
                        assert chosen["dictionary_id"] == dictionary
                        assert chosen["source"] == "requested"
                    else:
                        # The last assignment applies to this shared series on every Mac.
                        assert chosen["dictionary_ids"] == [second]
                        assert chosen["source"] == "series"
                # A list of one can now grow.
                recording, (owner, dictionary, *_) = next(iter(assigned.items()))
                headers = {"Authorization": "Bearer " + owner["token"]}
                other = create(http, headers, "Añadido")
                ok(
                    http,
                    headers,
                    action="select",
                    recording_id=recording,
                    dictionary_ids=[dictionary, other],
                    use_for_series=False,
                )
                assert document_dictionary(http, headers, recording)["dictionary_ids"] == [
                    dictionary,
                    other,
                ]
                ok(
                    http,
                    headers,
                    action="select",
                    recording_id=recording,
                    dictionary_ids=[dictionary],
                    use_for_series=False,
                )
        finally:
            reopened.close()


def test_a_meeting_uses_all_its_dictionaries_and_the_first_in_the_list_wins(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        first, second = create(http, headers, "Primero"), create(http, headers, "Segundo")
        # Each correction goes to one dictionary, as an older Mac sends it.
        for dictionary, wrong, right in [
            (first, "fondos", "LedgerBox"),
            (second, "fondos", "Fondant"),
            (second, "DLT", "NAV"),
        ]:
            other = meeting(store, owner["device_id"], texts=TEXTS)
            ok(
                http,
                headers,
                action="correct",
                recording_id=other,
                dictionary_id=dictionary,
                wrong=wrong,
                correct=right,
            )
        # The order sent does not matter: the list is the device's order.
        chosen = ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_ids=[second, first, second],
            use_for_series=False,
        )
        assert [d["id"] for d in chosen["dictionaries"]] == [first, second]
        meeting_dictionary = document_dictionary(http, headers, recording)
        assert meeting_dictionary["dictionary_ids"] == [first, second]
        # An older Mac reads one dictionary: the first, the one that wins.
        assert meeting_dictionary["dictionary_id"] == first
        assert meeting_dictionary["source"] == "requested"
        assert read(http, headers, recording) == [
            "LedgerBox, LedgerBox y prefondos; LedgerBox NAV.",
            "LedgerBox",
        ]
        # An empty list, like null, gives the meeting back to its series (none).
        ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_ids=[],
            use_for_series=False,
        )
        assert document_dictionary(http, headers, recording)["dictionary_ids"] == []
        assert read(http, headers, recording)[0] == TEXTS["mic_local"][0]


def test_one_correction_is_saved_in_several_dictionaries_at_once_and_undone_from_all(
    environment,
):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        first, second = create(http, headers, "Primero"), create(http, headers, "Segundo")
        ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_ids=[first],
            use_for_series=False,
        )
        # A form this term already had stays when the new one is undone.
        ok(
            http,
            headers,
            action="correct",
            dictionary_id=first,
            recording_id=recording,
            wrong="ledger box",
            correct="LedgerBox",
        )
        created = str(uuid4())
        saved = ok(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[first, second, created],
            wrong="fondos",
            correct="LedgerBox",
            name="Nuevo",
        )
        assert [d["id"] for d in saved["dictionaries"]] == [first, second, created]
        assert saved["conflicts"] == []
        assert terms_of(http, headers, first) == {("LedgerBox", ("ledger box", "fondos"))}
        assert terms_of(http, headers, second) == {("LedgerBox", ("fondos",))}
        assert terms_of(http, headers, created) == {("LedgerBox", ("fondos",))}
        assert next(d for d in listing(http, headers) if d["id"] == created)["name"] == "Nuevo"
        # The meeting now uses every dictionary the word went to.
        assert document_dictionary(http, headers, recording)["dictionary_ids"] == [
            first,
            second,
            created,
        ]
        assert read(http, headers, recording)[1] == "LedgerBox"

        for _ in range(2):  # «Deshacer» twice is harmless.
            undone = ok(
                http,
                headers,
                action="remove_correction",
                dictionary_ids=[first, second, created],
                wrong="FONDOS",
                correct="LedgerBox",
            )
            assert [d["id"] for d in undone["dictionaries"]] == [first, second, created]
        assert terms_of(http, headers, first) == {("LedgerBox", ("ledger box",))}
        assert terms_of(http, headers, second) == set()
        assert terms_of(http, headers, created) == set()
        assert read(http, headers, recording)[1] == "fondos"


def test_a_correction_into_several_dictionaries_is_all_or_nothing(environment, monkeypatch):
    from orbit_hub.meetings import dictionaries

    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        roomy, full = create(http, headers, "Con sitio"), create(http, headers, "Lleno")
        monkeypatch.setattr(dictionaries, "MAX_TERMS", 2)
        for i in range(2):
            ok(
                http,
                headers,
                action="correct",
                recording_id=recording,
                dictionary_id=full,
                wrong=f"forma{i}",
                correct=f"Término{i}",
            )
        before = (listing(http, headers), document_dictionary(http, headers, recording))
        refused = post(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[roomy, full],
            wrong="fondos",
            correct="LedgerBox",
        )
        assert refused.status_code == 409, refused.text
        assert refused.json()["code"] == "dictionary_full"
        numbers = refused.json()["refusal"]
        assert (numbers["terms"], numbers["max_terms"], numbers["dictionary_id"]) == (2, 2, full)
        assert (listing(http, headers), document_dictionary(http, headers, recording)) == before

        too_long = post(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[roomy],
            wrong="x",
            correct="x" * (dictionaries.MAX_TERM_CHARS + 1),
        )
        assert too_long.status_code == 413
        assert too_long.json()["refusal"]["characters"] == dictionaries.MAX_TERM_CHARS + 1
        assert too_long.json()["refusal"]["max_characters"] == dictionaries.MAX_TERM_CHARS

        unknown = post(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[roomy, str(uuid4())],
            wrong="fondos",
            correct="LedgerBox",
        )
        assert unknown.status_code == 404
        assert unknown.json()["code"] == "dictionary_not_found"
        assert (listing(http, headers), document_dictionary(http, headers, recording)) == before

        for payload, field in [
            ({"dictionary_ids": [], "wrong": "a", "correct": "b"}, "dictionary_ids"),
            (
                {"dictionary_id": roomy, "dictionary_ids": [roomy], "wrong": "a", "correct": "b"},
                "dictionary_ids",
            ),
            # An older Mac always sends `dictionary_id`: its absence is named.
            ({"wrong": "a", "correct": "b"}, "dictionary_id"),
        ]:
            invalid = post(http, headers, action="correct", recording_id=recording, **payload)
            assert invalid.status_code == 400, invalid.text
            assert invalid.json()["code"] == "invalid_dictionary_order"
            assert invalid.json()["refusal"]["field"] == field
        too_many = post(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_ids=[str(uuid4()) for _ in range(dictionaries.MAX_DICTIONARIES + 1)],
            use_for_series=False,
        )
        assert too_many.status_code == 400
        numbers = too_many.json()["refusal"]
        assert (numbers["field"], numbers["ids"], numbers["max_ids"]) == (
            "dictionary_ids",
            dictionaries.MAX_DICTIONARIES + 1,
            dictionaries.MAX_DICTIONARIES,
        )
        assert (listing(http, headers), document_dictionary(http, headers, recording)) == before


def test_adding_a_word_another_dictionary_of_the_meeting_corrects_differently_warns(
    environment,
):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        first, second = create(http, headers, "Ferretería Albar"), create(http, headers, "Lumo")
        ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_ids=[first, second],
            use_for_series=False,
        )
        ok(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[first],
            wrong="fondos",
            correct="LedgerBox",
        )
        warned = ok(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[second],
            wrong="Fondos",
            correct="Fondant",
        )
        assert warned["conflicts"] == [
            {
                "wrong": "Fondos",
                "correct": "Fondant",
                "dictionary_id": second,
                "other_dictionary_id": first,
                "other_name": "Ferretería Albar",
                "other_correct": "LedgerBox",
                "applied_dictionary_id": first,
            }
        ]
        # The first in the list wins.
        assert read(http, headers, recording)[1] == "LedgerBox"
        # An older Mac's order is warned the same way, in a field it ignores.
        single = ok(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_id=second,
            wrong="fondos",
            correct="Fondant",
        )
        assert single["id"] == second
        assert [c["other_dictionary_id"] for c in single["conflicts"]] == [first]
        # The same correction in both is no conflict.
        agreed = ok(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_ids=[first, second],
            wrong="DLT",
            correct="NAV",
        )
        assert agreed["conflicts"] == []


def test_a_series_uses_several_dictionaries_and_its_meetings_follow(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recordings = [meeting(store, owner["device_id"], texts=TEXTS, event=SERIES) for _ in range(2)]
    with client(environment) as http:
        first, second, third = (create(http, headers, name) for name in ("Uno", "Dos", "Tres"))
        ok(
            http,
            headers,
            action="correct",
            recording_id=recordings[0],
            dictionary_id=first,
            wrong="fondos",
            correct="LedgerBox",
        )
        ok(
            http,
            headers,
            action="correct",
            recording_id=recordings[0],
            dictionary_id=second,
            wrong="DLT",
            correct="NAV",
        )
        saved = http.put(
            f"/v1/recordings/{recordings[0]}/series",
            headers=headers,
            json={"template_id": "general", "dictionary_ids": [second, first]},
        )
        assert saved.status_code == 200, saved.text
        assert saved.json()["dictionary_ids"] == [first, second]
        assert saved.json()["dictionary_id"] == first
        for recording in recordings:
            chosen = document_dictionary(http, headers, recording)
            assert chosen["dictionary_ids"] == [first, second]
            assert chosen["source"] == "series"
            assert read(http, headers, recording)[0] == (
                "LedgerBox, LedgerBox y prefondos; LedgerBox NAV."
            )
        series = {
            d["id"]: [s["calendar_item_id"] for s in d["series"]] for d in listing(http, headers)
        }
        assert series == {
            first: [SERIES["calendar_item_id"]],
            second: [SERIES["calendar_item_id"]],
            third: [],
        }
        # The `select` order sets the whole list of the series too.
        ok(
            http,
            headers,
            action="select",
            recording_id=recordings[1],
            dictionary_ids=[third],
            use_for_series=True,
        )
        series = {
            d["id"]: [s["calendar_item_id"] for s in d["series"]] for d in listing(http, headers)
        }
        assert series == {first: [], second: [], third: [SERIES["calendar_item_id"]]}
        assert document_dictionary(http, headers, recordings[0])["dictionary_ids"] == [third]
        # An older Mac's single dictionary still sets the series to that one.
        older = http.put(
            f"/v1/recordings/{recordings[0]}/series",
            headers=headers,
            json={"template_id": "general", "dictionary_id": first},
        )
        assert older.status_code == 200, older.text
        assert older.json()["dictionary_id"] == first
        assert document_dictionary(http, headers, recordings[1])["dictionary_ids"] == [first]
        both = http.put(
            f"/v1/recordings/{recordings[0]}/series",
            headers=headers,
            json={"template_id": "general", "dictionary_id": first, "dictionary_ids": [first]},
        )
        assert both.status_code == 400
        assert both.json()["refusal"]["field"] == "dictionary_ids"


def test_the_whisper_prompt_and_the_note_get_every_dictionary_with_the_first_winning():
    from orbit_hub.meetings.dictionaries import combined, prompt, substitute

    first = {
        "id": "a",
        "name": "Uno",
        "terms": [
            {"id": "1", "correct": "LedgerBox", "wrong": ["fondos"]},
            {"id": "2", "correct": "Orbit", "wrong": ["orvit"]},
        ],
        "series": [],
    }
    second = {
        "id": "b",
        "name": "Dos",
        "terms": [
            {"id": "3", "correct": "Fondant", "wrong": ["FONDOS"]},
            {"id": "4", "correct": "orbit", "wrong": ["órbit"]},
            {"id": "5", "correct": "NAV", "wrong": ["DLT"]},
        ],
        "series": [],
    }
    terms = combined([first, second])
    assert substitute("fondos, órbit, orvit y DLT", terms) == "LedgerBox, Orbit, Orbit y NAV"
    # Fondant lost its only form; Orbit is one term with both forms.
    assert sorted(t["correct"] for t in terms) == ["LedgerBox", "NAV", "Orbit"]
    # The first dictionary's terms are the most recent: they win the prompt's budget.
    assert prompt(terms) == "Glosario: Orbit, LedgerBox, NAV."
    assert combined([]) == []
    # The inputs are not modified.
    assert first["terms"][0]["wrong"] == ["fondos"]
    assert second["terms"][0]["wrong"] == ["FONDOS"]


def test_an_older_mac_selecting_none_gives_the_meeting_to_its_series(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    with client(environment) as http:
        dictionary = create(http, headers)
        ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_id=dictionary,
            use_for_series=False,
        )
        answer = ok(
            http,
            headers,
            action="select",
            recording_id=recording,
            dictionary_id=None,
            use_for_series=False,
        )
        assert answer == {"dictionary_id": None}
        assert document_dictionary(http, headers, recording) == {
            "dictionary_id": None,
            "dictionary_ids": [],
            "source": None,
            "series": None,
        }
        # Neither field: the one an older Mac always sends is named.
        missing = post(http, headers, action="select", recording_id=recording, use_for_series=False)
        assert missing.status_code == 400
        assert missing.json()["refusal"]["field"] == "dictionary_id"


def test_every_order_the_mac_writes_is_accepted_by_the_real_route(environment):
    _, store, _ = environment
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    orders = json.loads(ORDERS.read_text().replace(ORDERS_RECORDING, recording))
    with client(environment) as http:
        for order in orders:
            ok(http, headers, **order)
        saved = {d["name"]: d["terms"] for d in listing(http, headers)}
        assert {name: [t["correct"] for t in terms] for name, terms in saved.items()} == {
            "Ferretería Albar": ["Orbit"],
            "Lumo": [],
            "Proyecto nuevo": [],
        }
        assert document_dictionary(http, headers, recording)["dictionary_ids"] == []
