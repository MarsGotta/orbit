"""«Recordatorio personal» in Hoy, kept by the hub so every Mac of its owner
shows it, and never put into a note. A meeting's reminder is named by the
digest of its title's words (`meetings.series.title_digest`), never by its
text. The latest edit wins, whichever Mac sends it first. Real routes over the
real store; invented meetings.
"""

from conftest import device
from support_app import client

from orbit_hub.meetings.series import title_digest
from orbit_hub.storage.reminders import MAX_REMINDER_CHARS

KEY = title_digest("Revisión semanal")


def path(key: str = KEY) -> str:
    return f"/v1/reminders/{key}"


def bearer(paired: dict) -> dict:
    return {"Authorization": "Bearer " + paired["token"]}


def test_a_reminder_written_on_one_mac_is_read_on_the_other(environment):
    _, store, _ = environment
    first, second = device(store, "Mac del despacho"), device(store, "Mac de casa")
    with client(environment) as http:
        empty = http.get(path(), headers=bearer(second))
        assert empty.status_code == 200, empty.text
        # None, a fact: never an unknown.
        assert empty.json() == {"key": KEY, "reminder": None}
        saved = http.put(
            path(),
            json={"text": "Llevar las cifras del trimestre", "edited_at_ms": 1_000},
            headers=bearer(first),
        )
        assert saved.status_code == 200, saved.text
        expected = {"text": "Llevar las cifras del trimestre", "edited_at_ms": 1_000}
        assert saved.json() == {"key": KEY, "reminder": expected}
        assert http.get(path(), headers=bearer(second)).json()["reminder"] == expected


def test_the_latest_edit_wins_whichever_arrives_first(environment):
    _, store, _ = environment
    first, second = device(store, "Mac del despacho"), device(store, "Mac de casa")
    with client(environment) as http:
        http.put(path(), json={"text": "Nuevo", "edited_at_ms": 2_000}, headers=bearer(first))
        # Written earlier on a Mac that was offline, sent when its hub came back.
        late = http.put(
            path(), json={"text": "Viejo", "edited_at_ms": 1_500}, headers=bearer(second)
        )
        assert late.json()["reminder"] == {"text": "Nuevo", "edited_at_ms": 2_000}
        # Emptied later: an empty reminder is an edit too, and wins.
        http.put(path(), json={"text": "", "edited_at_ms": 3_000}, headers=bearer(second))
        assert http.get(path(), headers=bearer(first)).json()["reminder"] == {
            "text": "",
            "edited_at_ms": 3_000,
        }


def test_the_database_keeps_neither_the_key_nor_the_text_in_clear(environment):
    _, store, _ = environment
    paired = device(store)
    with client(environment) as http:
        http.put(
            path(),
            json={"text": "Preguntar por el presupuesto", "edited_at_ms": 1},
            headers=bearer(paired),
        )
    with store.reading() as db:
        rows = db.execute("SELECT key, body FROM personal_reminders").fetchall()
    assert len(rows) == 1
    assert rows[0]["key"] != KEY
    assert b"presupuesto" not in rows[0]["body"]


def test_refusals_name_their_field_and_numbers(environment):
    _, store, _ = environment
    headers = bearer(device(store))
    with client(environment) as http:
        bad_key = http.get(path("Revisión semanal"), headers=headers)
        assert bad_key.status_code == 400
        assert bad_key.json()["refusal"]["field"] == "key"
        long = http.put(
            path(),
            json={"text": "a" * (MAX_REMINDER_CHARS + 1), "edited_at_ms": 1},
            headers=headers,
        )
        assert long.status_code == 400
        refusal = long.json()["refusal"]
        assert (refusal["field"], refusal["chars"], refusal["max_chars"]) == (
            "text",
            MAX_REMINDER_CHARS + 1,
            MAX_REMINDER_CHARS,
        )
        negative = http.put(path(), json={"text": "x", "edited_at_ms": -1}, headers=headers)
        assert negative.status_code == 400
        assert http.get(path(), headers={}).status_code == 401
