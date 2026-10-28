"""A meeting's calendar series, as the Mac reads and sets it.

The series is the calendar's `calendar_item_id` that a closing manifest carries
in its event (`notes/series_key.py`); a recording started without an event has
none. Two things go by it here:

- the earlier meeting of a series and the tasks of its note, done or not, which
  Hoy shows as «Pendiente de la última vez» for the next meeting of the series;
  failing that, the earlier meeting of the same title (`title_key`): one
  recorded by hand with the event's title carries no series;
- the template and the dictionaries of the series, saved together in one
  transaction («Usar lo mismo en toda la serie»).

The meeting's language is not a property of its series: it lives in one place,
the recording (`PUT /v1/recordings/{id}/language`).
"""

import hashlib
import json

from ..protocol import HubError
from . import dictionaries, repository, views


def title_key(title: str) -> str:
    """The words of a title, lowercase, without those made only of digits.

    The Mac's own (`orbit_core::notes::title_key`), checked against it with
    `contracts/fixtures/series-title-keys-v1.json`: «Revisión de diseño 12»
    and «revisión de diseño» are the same meeting.
    """
    words, word = [], []
    for character in title + " ":
        if character.isalnum():
            word.append(character)
            continue
        if word and not all("0" <= digit <= "9" for digit in word):
            words.append("".join(word).lower())
        word = []
    return " ".join(words)


def title_digest(title: str) -> str | None:
    """What the Mac sends instead of the title: the SHA-256 of its key, or
    None for a title without words, which matches nothing."""
    key = title_key(title)
    return hashlib.sha256(key.encode()).hexdigest() if key else None


def previous(
    store, notes, db, device: str, series: str, before_ms: int, title: str | None = None
) -> dict | None:
    """The latest closed meeting of `series` that started before `before_ms`;
    failing that, with `title` (a `title_digest`), the latest of that title.

    None when there is none: a fact, not an unknown. A voice call or a
    recording still open is not a meeting of the series. Its note is null when
    nobody asked for it; a note not written yet has a state and no tasks
    (`action_items: null`), never an empty list.
    """
    by_series = by_title = None
    for row in db.execute("SELECT device_id, recording, body FROM recording_manifests").fetchall():
        device = row["device_id"]
        manifest = json.loads(store.decrypt(row["body"], f"manifest:{device}:{row['recording']}"))
        started = manifest.get("started_at_ms")
        if (
            type(started) is not int
            or type(manifest.get("finalized_at_ms")) is not int
            or manifest.get("kind", "meeting") != "meeting"
            or started >= before_ms
        ):
            continue
        key = (started, row["recording"])
        event = manifest.get("event")
        if isinstance(event, dict) and event.get("calendar_item_id") == series:
            if by_series is None or key > by_series[0]:
                by_series = (key, manifest, device)
        elif (
            title is not None
            and (by_title is None or key > by_title[0])
            and _titled(store, db, device, row["recording"], manifest, started, title)
        ):
            by_title = (key, manifest, device)
    found = by_series or by_title
    if found is None:
        return None
    (started, recording), manifest, device = found
    return {
        "recording_id": recording,
        "title": views.named(store, db, device, recording, views.title(manifest, started)),
        "started_at_ms": started,
        "note": _tasks(views.note_document(notes, db, device, recording)),
    }


def _titled(store, db, device: str, recording: str, manifest: dict, started: int, digest: str):
    """The meeting has that title: its own, or the one the user gave it."""
    own = views.title(manifest, started)
    return digest in {
        title_digest(own),
        title_digest(views.named(store, db, device, recording, own)),
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
    dictionary_ids: list[str],
    now: float,
) -> dict:
    """The series of `recording` takes this template and these dictionaries (or none).

    One transaction: if either half is refused, neither is saved, and the
    refusal names its cause and numbers. The meeting's own dictionary choice is
    cleared so that it follows its series, as `select` with `use_for_series`
    does; the notes already written keep their template until written again.
    """
    with store.transaction() as db:
        repository.require(db, device, recording)
        series = dictionaries.series_of(store, db, device, recording)
        if series is None:
            raise HubError(
                "meeting_series_missing",
                "The meeting has no calendar series: neither template nor dictionary was saved",
                http_status=409,
            )
        templates.link_in(db, device, series["calendar_item_id"], template_id, now)
        listed = dictionaries.listing(store, db, device)
        chosen = dictionaries.existing(listed, dictionary_ids)
        dictionaries.assign_series(listed, chosen, series)
        dictionaries.save_all(store, db, device, listed)
        dictionaries.set_own(db, device, recording, [])
    ids = [d["id"] for d in chosen]
    return {
        "recording_id": recording,
        "calendar_item_id": series["calendar_item_id"],
        "template_id": template_id,
        # The first, which wins a conflict: what a Mac up to 0.10.1 reads.
        "dictionary_id": ids[0] if ids else None,
        "dictionary_ids": ids,
    }
