"""A meeting's calendar series, as the Mac reads and sets it.

The series is the calendar's `calendar_item_id` that a closing manifest carries
in its event (`notes/series_key.py`); a recording started without an event has
none. Two things go by it here:

- the earlier meeting of a series and the tasks of its note, done or not, which
  Hoy shows as «Pendiente de la última vez» for the next meeting of the series;
- the template and the dictionary of the series, saved together in one
  transaction («Usar lo mismo en toda la serie»).

The meeting's language is not a property of its series: it lives in one place,
the recording (`PUT /v1/recordings/{id}/language`).
"""

import json

from ..protocol import HubError
from . import dictionaries, repository, views


def previous(store, notes, db, device: str, series: str, before_ms: int) -> dict | None:
    """The latest closed meeting of `series` that started before `before_ms`.

    None when the series has none: a fact, not an unknown. A voice call or a
    recording still open is not a meeting of the series. Its note is null when
    nobody asked for it; a note not written yet has a state and no tasks
    (`action_items: null`), never an empty list.
    """
    found = None
    for row in db.execute(
        "SELECT recording, body FROM recording_manifests WHERE device_id=?", (device,)
    ).fetchall():
        manifest = json.loads(store.decrypt(row["body"], f"manifest:{device}:{row['recording']}"))
        started = manifest.get("started_at_ms")
        event = manifest.get("event")
        if (
            type(started) is not int
            or type(manifest.get("finalized_at_ms")) is not int
            or manifest.get("kind", "meeting") != "meeting"
            or not isinstance(event, dict)
            or event.get("calendar_item_id") != series
            or started >= before_ms
        ):
            continue
        key = (started, row["recording"])
        if found is None or key > found[0]:
            found = (key, manifest)
    if found is None:
        return None
    (started, recording), manifest = found
    return {
        "recording_id": recording,
        "title": views.named(store, db, device, recording, views.title(manifest, started)),
        "started_at_ms": started,
        "note": _tasks(views.note_document(notes, db, device, recording)),
    }


def _tasks(document: dict | None) -> dict | None:
    if document is None:
        return None
    seen = document["note"]
    return {
        "state": document["state"],
        "revision": document["revision"],
        "short_summary": seen["short_summary"] if seen is not None else None,
        "action_items": None
        if seen is None
        else [
            {key: item.get(key) for key in ("id", "owner", "text", "due", "done")}
            for item in seen["action_items"]
        ],
    }


def use_for_series(
    store,
    templates,
    device: str,
    recording: str,
    template_id: str,
    dictionary_id: str | None,
    now: float,
) -> dict:
    """The series of `recording` takes this template and this dictionary, or none.

    One transaction: if either half is refused, neither is saved, and the
    refusal names its cause and numbers. The meeting's own dictionary choice is
    cleared so that it follows its series, as `select` with `use_for_series`
    does; the notes already written keep their template until written again.
    """
    chosen_id = dictionaries.identifier(dictionary_id) if dictionary_id is not None else None
    with store.transaction() as db:
        repository.require(db, device, recording)
        series = dictionaries.series_of(store, db, device, recording)
        if series is None:
            raise HubError(
                "meeting_series_missing",
                "La reunión no tiene serie de calendario: no se guardó ni la plantilla "
                "ni el diccionario",
                http_status=409,
            )
        templates.link_in(db, device, series["calendar_item_id"], template_id, now)
        listed = dictionaries.listing(store, db, device)
        chosen = next((d for d in listed if d["id"] == chosen_id), None)
        if chosen_id is not None and chosen is None:
            raise HubError("dictionary_not_found", "El diccionario no existe", http_status=404)
        dictionaries.assign_series(listed, chosen, series)
        dictionaries.save_all(store, db, device, listed)
        db.execute(
            "DELETE FROM recording_dictionaries WHERE device_id=? AND recording=?",
            (device, recording),
        )
    return {
        "recording_id": recording,
        "calendar_item_id": series["calendar_item_id"],
        "template_id": template_id,
        "dictionary_id": chosen_id,
    }
