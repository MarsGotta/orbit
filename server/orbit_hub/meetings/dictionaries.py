"""Meeting vocabulary and its assignments, owned by the hub and encrypted at rest."""

import json
import re
from typing import Annotated, Literal
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, Field, RootModel

from ..identifiers import canonical_uuid
from ..protocol import HubError
from . import ownership, repository

# Settings loads the whole list. 32 named vocabularies cover separate projects;
# the independent byte ceiling below keeps that list readable by the Mac.
MAX_DICTIONARIES = 32
# 128 terms with eight observed spellings each bound matching work and the note
# prompt. These are project names and jargon, not a replacement transcript.
MAX_TERMS = 128
MAX_FORMS = 8
# A short phrase or a full name fits in 128 characters; this bounds each regex
# alternative and prevents a selected paragraph from becoming a dictionary term.
MAX_TERM_CHARS = 128
MAX_NAME_CHARS = 80  # A name in the Settings list, rather than meeting content.
# Below the daemon's 1 MiB HTTP body bound, leaving room for the JSON envelope.
MAX_LIST_BYTES = 768 * 1024
# The library read pages whole dictionaries as single items. Its 256 KiB page
# must hold one, so 192 KiB leaves room for the item tag and pending/refusals.
# This is separate from the whole HTTP listing's bound above.
MAX_DICTIONARY_BYTES = 192 * 1024
# Like template assignments: 256 maximum-length calendar ids take ~300 KiB.
MAX_SERIES = 256
# Whisper uses only the last 223 tokens. 160 characters (including the glossary
# prefix, separators and final period) leave margin even at one token per
# character for Spanish names/acronyms;
# this is a conservative character heuristic, not a tokenizer guarantee for
# arbitrary Unicode. Whole most-recently-corrected terms win; no term is cut.
MAX_PROMPT_CHARS = 160

# A meeting's own dictionaries: one row each. Until 0.10.1 the primary key
# was the meeting, one dictionary per meeting; `migrate` turns those rows
# into lists of one. A series' dictionaries live in each dictionary's body.
SCHEMA = """
CREATE TABLE IF NOT EXISTS meeting_dictionaries (
    device_id TEXT NOT NULL, id TEXT NOT NULL, body BLOB NOT NULL,
    PRIMARY KEY(device_id,id));
CREATE TABLE IF NOT EXISTS recording_dictionaries (
    device_id TEXT NOT NULL, recording TEXT NOT NULL, dictionary_id TEXT NOT NULL,
    PRIMARY KEY(device_id,recording,dictionary_id));
"""


def migrate(db) -> None:
    """One dictionary per meeting becomes a list of one, in one transaction.

    Every row is copied as it is; nothing is read or decrypted. A store
    already migrated (its key holds `dictionary_id`) is left untouched, so
    opening it again is harmless.
    """
    key = [
        row["name"]
        for row in sorted(db.execute("PRAGMA table_info(recording_dictionaries)"), key=pk)
        if row["pk"]
    ]
    if "dictionary_id" in key:
        return
    db.execute("BEGIN IMMEDIATE")
    try:
        db.execute(
            "CREATE TABLE recording_dictionary_lists (device_id TEXT NOT NULL, "
            "recording TEXT NOT NULL, dictionary_id TEXT NOT NULL, "
            "PRIMARY KEY(device_id,recording,dictionary_id))"
        )
        db.execute(
            "INSERT INTO recording_dictionary_lists(device_id,recording,dictionary_id) "
            "SELECT device_id,recording,dictionary_id FROM recording_dictionaries"
        )
        db.execute("DROP TABLE recording_dictionaries")
        db.execute("ALTER TABLE recording_dictionary_lists RENAME TO recording_dictionaries")
    except BaseException:
        db.execute("ROLLBACK")
        raise
    db.execute("COMMIT")


def pk(row) -> int:
    return row["pk"]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, hide_input_in_errors=True)


class Save(Strict):
    action: Literal["save"]
    dictionary_id: str
    name: str


class Select(Strict):
    """The meeting's dictionaries, or with `use_for_series` its series'.

    `dictionary_id` is one, as Macs up to 0.10.1 send it (null: none);
    `dictionary_ids` is the whole list. One of the two, never both.
    """

    action: Literal["select"]
    recording_id: str
    dictionary_id: str | None = None
    dictionary_ids: list[str] | None = None
    use_for_series: bool = False


class Correct(Strict):
    """One correction saved in one dictionary, or in several at once."""

    action: Literal["correct"]
    recording_id: str
    dictionary_id: str | None = None
    dictionary_ids: list[str] | None = None
    wrong: str
    correct: str
    # Creates the dictionaries named that do not exist yet, atomically with
    # the correction.
    name: str | None = None


class Remove(Strict):
    action: Literal["remove"]
    dictionary_id: str
    entry_id: str


class RemoveCorrection(Strict):
    """«Deshacer» a correction: the heard form leaves that term in each dictionary."""

    action: Literal["remove_correction"]
    dictionary_ids: list[str]
    wrong: str
    correct: str


class Order(
    RootModel[
        Annotated[
            Save | Select | Correct | Remove | RemoveCorrection, Field(discriminator="action")
        ]
    ]
):
    pass


def text(value: str, field: str, maximum: int) -> str:
    size = len(value)
    if size > maximum:
        raise HubError(
            "dictionary_text_too_long",
            f"El campo {field} tiene {size} caracteres; el máximo es {maximum}",
            http_status=413,
            numbers={"characters": size, "max_characters": maximum, "field": field},
        )
    if not value.strip() or any(not c.isprintable() for c in value):
        raise HubError(
            "dictionary_text_invalid",
            f"El campo {field} está vacío o contiene controles",
            numbers={"characters": size, "field": field},
        )
    return value.strip()


def identifier(value: str) -> str:
    try:
        return canonical_uuid(value)
    except ValueError:
        raise HubError("invalid_dictionary_id", "Identificador de diccionario no válido") from None


def named_ids(order, *, required: bool, code: str = "invalid_dictionary_order") -> list[str]:
    """The dictionaries an order names, as a list without repetitions.

    `dictionary_ids` as sent, or `dictionary_id` as a list of one (null:
    none). An order must carry one of the two: an older Mac always sends
    `dictionary_id`, so its absence is named.
    """
    single, several = getattr(order, "dictionary_id", None), order.dictionary_ids
    if several is not None and single is not None:
        raise HubError(
            code,
            "La orden trae dictionary_id y dictionary_ids; se envía uno de los dos",
            numbers={"field": "dictionary_ids"},
        )
    if several is None and "dictionary_id" not in order.model_fields_set:
        raise HubError(
            code, "Falta el diccionario: dictionary_id", numbers={"field": "dictionary_id"}
        )
    ids = several if several is not None else [] if single is None else [single]
    # A device has at most MAX_DICTIONARIES, so a longer list names one twice
    # or one that does not exist.
    if len(ids) > MAX_DICTIONARIES:
        raise HubError(
            code,
            f"La orden nombra {len(ids)} diccionarios; el máximo es {MAX_DICTIONARIES}",
            numbers={"field": "dictionary_ids", "ids": len(ids), "max_ids": MAX_DICTIONARIES},
        )
    if required and not ids:
        raise HubError(
            code, "La orden no nombra ningún diccionario", numbers={"field": "dictionary_ids"}
        )
    return list(dict.fromkeys(identifier(value) for value in ids))


def listing(store, db, device: str) -> list[dict]:
    """Every dictionary of the person, oldest first: the order that decides conflicts."""
    return [
        json.loads(store.decrypt(row["body"], f"dictionary:{row['device_id']}:{row['id']}"))
        for row in db.execute("SELECT device_id,id,body FROM meeting_dictionaries ORDER BY rowid")
    ]


def existing(dictionaries: list[dict], ids: list[str]) -> list[dict]:
    """The dictionaries with these ids, in the device's order; all of them, or a refusal."""
    known = {d["id"] for d in dictionaries}
    if any(identity not in known for identity in ids):
        raise HubError("dictionary_not_found", "El diccionario no existe", http_status=404)
    return [d for d in dictionaries if d["id"] in ids]


def series_of(store, db, device: str, recording: str) -> dict | None:
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is not None:
        event = json.loads(store.decrypt(row["body"], f"manifest:{device}:{recording}")).get(
            "event"
        )
    else:
        event = None
    if event and event.get("calendar_item_id"):
        return {"calendar_item_id": event["calendar_item_id"], "title": event.get("title", "")}
    return None


def own_ids(db, device: str, recording: str) -> set[str]:
    return {
        row["dictionary_id"]
        for row in db.execute(
            "SELECT dictionary_id FROM recording_dictionaries WHERE device_id=? AND recording=?",
            (device, recording),
        )
    }


def set_own(db, device: str, recording: str, ids: list[str]) -> None:
    """The meeting's own list; empty gives the meeting back to its series."""
    db.execute(
        "DELETE FROM recording_dictionaries WHERE device_id=? AND recording=?",
        (device, recording),
    )
    db.executemany(
        "INSERT INTO recording_dictionaries VALUES (?,?,?)",
        [(device, recording, identity) for identity in ids],
    )


def chosen_in(
    dictionaries: list[dict], own: set[str], series: dict | None
) -> tuple[list[dict], str | None]:
    """The meeting's own dictionaries, else its series', in the device's order."""
    chosen = [d for d in dictionaries if d["id"] in own]
    if chosen:
        return chosen, "requested"
    if series:
        chosen = [
            d
            for d in dictionaries
            if any(s["calendar_item_id"] == series["calendar_item_id"] for s in d["series"])
        ]
    return chosen, "series" if chosen else None


def resolve(store, db, device: str, recording: str) -> tuple[list[dict], dict]:
    series = series_of(store, db, device, recording)
    chosen, source = chosen_in(listing(store, db, device), own_ids(db, device, recording), series)
    ids = [d["id"] for d in chosen]
    return chosen, {
        # The first, which wins a conflict: what a Mac up to 0.10.1 reads.
        "dictionary_id": ids[0] if ids else None,
        "dictionary_ids": ids,
        "source": source,
        "series": series,
    }


def combined(chosen: list[dict]) -> list[dict]:
    """The terms of a meeting's dictionaries as one list, the first dictionary winning.

    A heard form belongs to the first dictionary that corrects it: a later
    dictionary's entry loses that form, and drops out when it has none left.
    The same correct form in two dictionaries is one term with the forms of
    both. The first dictionary's terms come last, as the most recent, so they
    also win the Whisper prompt's budget (`prompt`). The inputs are not changed.
    """
    claimed: set[str] = set()
    terms: dict[str, dict] = {}
    layers = []
    for dictionary in chosen:
        layer = []
        for entry in dictionary["terms"]:
            forms = [form for form in entry["wrong"] if form.casefold() not in claimed]
            claimed.update(form.casefold() for form in forms)
            key = entry["correct"].casefold()
            if key in terms:
                terms[key]["wrong"].extend(forms)
            elif forms:
                terms[key] = {**entry, "wrong": forms}
                layer.append(terms[key])
        layers.append(layer)
    return [entry for layer in reversed(layers) for entry in layer]


def terms_for(store, device: str, recording: str) -> list[dict]:
    with store.read_snapshot() as db:
        chosen, _ = resolve(store, db, device, recording)
    return combined(chosen)


def prompt(terms: list[dict]) -> str:
    kept = []
    result = ""
    for term in reversed(terms):
        candidate = "Glosario: " + ", ".join([*kept, term["correct"]]) + "."
        if len(candidate) <= MAX_PROMPT_CHARS:
            kept.append(term["correct"])
            result = candidate
    return result


def substitute(value: str, terms: list[dict]) -> str:
    replacements = {form.casefold(): entry["correct"] for entry in terms for form in entry["wrong"]}
    if not replacements:
        return value
    # One pass: a replacement never feeds another, and phrases beat words.
    forms = sorted(replacements, key=len, reverse=True)
    pattern = r"(?<!\w)(?:" + "|".join(re.escape(form) for form in forms) + r")(?!\w)"
    return re.sub(
        pattern,
        lambda match: replacements.get(match[0].casefold(), match[0]),
        value,
        flags=re.IGNORECASE,
    )


def created(dictionaries: list[dict], identity: str, name: str) -> dict:
    if len(dictionaries) >= MAX_DICTIONARIES:
        raise HubError(
            "dictionaries_full",
            f"Hay {len(dictionaries)} diccionarios; el máximo es {MAX_DICTIONARIES}",
            http_status=409,
            numbers={"dictionaries": len(dictionaries), "max_dictionaries": MAX_DICTIONARIES},
        )
    dictionary = {"id": identity, "name": name, "terms": [], "series": []}
    dictionaries.append(dictionary)
    return dictionary


def add_correction(dictionary: dict, wrong: str, right: str) -> None:
    """`wrong` is now heard as `right` in this dictionary, and only there."""
    entry = next(
        (e for e in dictionary["terms"] if e["correct"].casefold() == right.casefold()), None
    )
    if entry is None:
        if len(dictionary["terms"]) >= MAX_TERMS:
            raise HubError(
                "dictionary_full",
                f"Hay {len(dictionary['terms'])} términos; el máximo es {MAX_TERMS}",
                http_status=409,
                numbers={
                    "terms": len(dictionary["terms"]),
                    "max_terms": MAX_TERMS,
                    "dictionary_id": dictionary["id"],
                },
            )
        entry = {"id": str(uuid4()), "correct": right, "wrong": []}
    forms = [f for f in entry["wrong"] if f.casefold() != wrong.casefold()]
    if len(forms) >= MAX_FORMS:
        raise HubError(
            "dictionary_forms_full",
            f"Hay {len(forms)} formas equivocadas; el máximo es {MAX_FORMS}",
            http_status=409,
            numbers={
                "forms": len(forms),
                "max_forms": MAX_FORMS,
                "dictionary_id": dictionary["id"],
            },
        )
    for old in dictionary["terms"]:
        old["wrong"] = [f for f in old["wrong"] if f.casefold() != wrong.casefold()]
    entry.update(correct=right, wrong=[*forms, wrong])
    dictionary["terms"] = [
        e for e in dictionary["terms"] if e["id"] != entry["id"] and e["wrong"]
    ] + [entry]


def conflicts(chosen: list[dict], targets: list[dict], wrong: str, right: str) -> list[dict]:
    """Where another dictionary of the meeting hears `wrong` as something else.

    `chosen` is the meeting's list after the correction; the first
    dictionary in it that corrects `wrong` is the one applied.
    """

    def correction(dictionary: dict) -> str | None:
        return next(
            (
                entry["correct"]
                for entry in dictionary["terms"]
                if any(form.casefold() == wrong.casefold() for form in entry["wrong"])
            ),
            None,
        )

    applied = next((d["id"] for d in chosen if correction(d) is not None), None)
    target_ids = {d["id"] for d in targets}
    found = []
    for target in targets:
        for other in chosen:
            other_correct = correction(other) if other["id"] not in target_ids else None
            if other_correct is not None and other_correct.casefold() != right.casefold():
                found.append(
                    {
                        "wrong": wrong,
                        "correct": right,
                        "dictionary_id": target["id"],
                        "other_dictionary_id": other["id"],
                        "other_name": other["name"],
                        "other_correct": other_correct,
                        "applied_dictionary_id": applied,
                    }
                )
    return found


def execute(store, device: str, order: Save | Select | Correct | Remove | RemoveCorrection) -> dict:
    with store.transaction() as db:
        if isinstance(order, (Select, Correct)):
            device = ownership.device_for(db, order.recording_id, device)
        dictionaries = listing(store, db, device)
        if isinstance(order, Save):
            answer = save(dictionaries, order)
        elif isinstance(order, Select):
            answer = select(store, db, device, dictionaries, order)
        elif isinstance(order, Correct):
            answer = correct(store, db, device, dictionaries, order)
        elif isinstance(order, Remove):
            [chosen] = existing(dictionaries, [identifier(order.dictionary_id)])
            chosen["terms"] = [e for e in chosen["terms"] if e["id"] != order.entry_id]
            answer = chosen
        else:
            wrong = text(order.wrong, "wrong", MAX_TERM_CHARS)
            right = text(order.correct, "correct", MAX_TERM_CHARS)
            targets = existing(dictionaries, named_ids(order, required=True))
            for target in targets:
                for entry in target["terms"]:
                    if entry["correct"].casefold() == right.casefold():
                        entry["wrong"] = [
                            f for f in entry["wrong"] if f.casefold() != wrong.casefold()
                        ]
                target["terms"] = [e for e in target["terms"] if e["wrong"]]
            answer = {"dictionaries": targets}
        save_all(store, db, device, dictionaries)
    return answer


def save(dictionaries: list[dict], order: Save) -> dict:
    identity = identifier(order.dictionary_id)
    name = text(order.name, "name", MAX_NAME_CHARS)
    found = next((d for d in dictionaries if d["id"] == identity), None)
    if found is None:
        return created(dictionaries, identity, name)
    found["name"] = name
    return found


def select(store, db, device: str, dictionaries: list[dict], order: Select) -> dict:
    ids = named_ids(order, required=False)
    repository.require(db, device, order.recording_id)
    chosen = existing(dictionaries, ids)
    if order.use_for_series:
        series = series_of(store, db, device, order.recording_id)
        if series is None:
            raise HubError(
                "dictionary_series_missing",
                "La reunión no tiene serie de calendario",
                http_status=409,
            )
        assign_series(dictionaries, chosen, series)
        set_own(db, device, order.recording_id, [])
    else:
        set_own(db, device, order.recording_id, [d["id"] for d in chosen])
    if order.dictionary_ids is not None:
        return {"dictionaries": chosen}
    return chosen[0] if chosen else {"dictionary_id": None}


def correct(store, db, device: str, dictionaries: list[dict], order: Correct) -> dict:
    ids = named_ids(order, required=True)
    repository.require(db, device, order.recording_id)
    wrong = text(order.wrong, "wrong", MAX_TERM_CHARS)
    right = text(order.correct, "correct", MAX_TERM_CHARS)
    known = {d["id"] for d in dictionaries}
    missing = [identity for identity in ids if identity not in known]
    if missing and order.name is None:
        raise HubError("dictionary_not_found", "El diccionario no existe", http_status=404)
    if missing:
        name = text(order.name, "name", MAX_NAME_CHARS)
        for identity in missing:
            created(dictionaries, identity, name)
    targets = existing(dictionaries, ids)
    for target in targets:
        add_correction(target, wrong, right)
    series = series_of(store, db, device, order.recording_id)
    current, _ = chosen_in(dictionaries, own_ids(db, device, order.recording_id), series)
    current_ids = [d["id"] for d in current]
    if order.dictionary_ids is None:
        # As before: a correction keeps a choice that already has its
        # dictionary, inherited or not; otherwise that dictionary is chosen.
        if ids[0] not in current_ids:
            set_own(db, device, order.recording_id, ids)
    elif any(identity not in current_ids for identity in ids):
        # The word goes to every dictionary named; the meeting uses them all.
        set_own(db, device, order.recording_id, list(dict.fromkeys([*current_ids, *ids])))
    after, _ = chosen_in(dictionaries, own_ids(db, device, order.recording_id), series)
    found = conflicts(after, targets, wrong, right)
    if order.dictionary_ids is None:
        return {**targets[0], "conflicts": found}
    return {"dictionaries": targets, "conflicts": found}


def assign_series(dictionaries: list[dict], chosen: list[dict], series: dict) -> None:
    """`chosen` (possibly none) become the series' dictionaries, replacing its list."""
    for dictionary in dictionaries:
        dictionary["series"] = [
            s for s in dictionary["series"] if s["calendar_item_id"] != series["calendar_item_id"]
        ]
    for dictionary in chosen:
        dictionary["series"].append(series)
    count = sum(len(d["series"]) for d in dictionaries)
    if count > MAX_SERIES:
        raise HubError(
            "dictionary_series_full",
            f"Hay {count} series; el máximo es {MAX_SERIES}",
            http_status=409,
            numbers={"series": count, "max_series": MAX_SERIES},
        )


def save_all(store, db, device: str, dictionaries: list[dict]) -> None:
    """Every dictionary of the device, within its bounds, in the caller's transaction."""
    encoded = json.dumps(
        {"dictionaries": dictionaries}, ensure_ascii=False, separators=(",", ":")
    ).encode()
    if len(encoded) > MAX_LIST_BYTES:
        raise HubError(
            "dictionaries_too_large",
            f"La lista ocupa {len(encoded)} bytes; el máximo es {MAX_LIST_BYTES}",
            http_status=413,
            numbers={"bytes": len(encoded), "max_bytes": MAX_LIST_BYTES},
        )
    for dictionary in dictionaries:
        size = len(json.dumps(dictionary, ensure_ascii=False, separators=(",", ":")).encode())
        if size > MAX_DICTIONARY_BYTES:
            raise HubError(
                "dictionary_too_large",
                f"El diccionario ocupa {size} bytes; el máximo es {MAX_DICTIONARY_BYTES}",
                http_status=413,
                numbers={
                    "bytes": size,
                    "max_bytes": MAX_DICTIONARY_BYTES,
                    "dictionary_id": dictionary["id"],
                },
            )
    sealed = []
    for dictionary in dictionaries:
        identity = dictionary["id"]
        owners = db.execute(
            "SELECT device_id FROM meeting_dictionaries WHERE id=? LIMIT 2", (identity,)
        ).fetchall()
        if len(owners) > 1:
            raise HubError(
                "dictionary_id_ambiguous",
                "Hay dos diccionarios con el mismo identificador",
                http_status=409,
                numbers={"matches_at_least": len(owners)},
            )
        owner = owners[0]["device_id"] if owners else device
        body = store.encrypt(
            json.dumps(dictionary, ensure_ascii=False).encode(), f"dictionary:{owner}:{identity}"
        )
        sealed.append((owner, identity, body))
    store.ensure_capacity(sum(len(body) for _, _, body in sealed))
    db.executemany(
        "INSERT INTO meeting_dictionaries VALUES (?,?,?) ON CONFLICT(device_id,id) DO UPDATE SET body=excluded.body",
        sealed,
    )


def mount(app, store, identity, bounded_body, run):
    from fastapi import Request
    from fastapi.responses import JSONResponse

    from ..notes.http import parse

    @app.get("/v1/meeting-dictionaries")
    async def get_dictionaries(request: Request):
        device = identity(request)

        def read():
            with store.read_snapshot() as db:
                return {"dictionaries": listing(store, db, device)}

        return JSONResponse(await run(read))

    @app.post("/v1/meeting-dictionaries/orders")
    async def dictionary_order(request: Request):
        device = identity(request)
        # One order carries two phrases, a name and at most 32 dictionary ids
        # (about 1.3 KiB); 16 KiB fits even JSON escaping every character of
        # the text and rejects paragraphs before parsing.
        order = parse(
            await bounded_body(request, 16 * 1024), Order, "invalid_dictionary_order", "Diccionario"
        ).root
        result = await run(lambda: execute(store, device, order))

        return JSONResponse(result)
