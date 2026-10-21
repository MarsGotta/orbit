"""Meeting vocabulary and its assignments, owned by the hub and encrypted at rest."""

import json
import re
from typing import Annotated, Literal
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, Field, RootModel

from ..identifiers import canonical_uuid
from ..protocol import HubError
from . import repository

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

SCHEMA = """
CREATE TABLE IF NOT EXISTS meeting_dictionaries (
    device_id TEXT NOT NULL, id TEXT NOT NULL, body BLOB NOT NULL,
    PRIMARY KEY(device_id,id));
CREATE TABLE IF NOT EXISTS recording_dictionaries (
    device_id TEXT NOT NULL, recording TEXT NOT NULL, dictionary_id TEXT NOT NULL,
    PRIMARY KEY(device_id,recording));
"""


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, hide_input_in_errors=True)


class Save(Strict):
    action: Literal["save"]
    dictionary_id: str
    name: str


class Select(Strict):
    action: Literal["select"]
    recording_id: str
    dictionary_id: str | None
    use_for_series: bool = False


class Correct(Strict):
    action: Literal["correct"]
    recording_id: str
    dictionary_id: str
    wrong: str
    correct: str
    # Creates the named dictionary atomically with the first correction.
    name: str | None = None


class Remove(Strict):
    action: Literal["remove"]
    dictionary_id: str
    entry_id: str


class Order(RootModel[Annotated[Save | Select | Correct | Remove, Field(discriminator="action")]]):
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


def listing(store, db, device: str) -> list[dict]:
    return [
        json.loads(store.decrypt(row["body"], f"dictionary:{device}:{row['id']}"))
        for row in db.execute(
            "SELECT id,body FROM meeting_dictionaries WHERE device_id=? ORDER BY rowid", (device,)
        )
    ]


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


def resolve(store, db, device: str, recording: str) -> tuple[dict | None, dict]:
    series = series_of(store, db, device, recording)
    row = db.execute(
        "SELECT dictionary_id FROM recording_dictionaries WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    choices = listing(store, db, device)
    chosen = next((d for d in choices if d["id"] == row["dictionary_id"]), None) if row else None
    source = "requested" if chosen else None
    if chosen is None and series:
        chosen = next(
            (
                d
                for d in choices
                if any(s["calendar_item_id"] == series["calendar_item_id"] for s in d["series"])
            ),
            None,
        )
        source = "series" if chosen else None
    return chosen, {
        "dictionary_id": chosen["id"] if chosen else None,
        "source": source,
        "series": series,
    }


def terms_for(store, device: str, recording: str) -> list[dict]:
    with store.read_snapshot() as db:
        chosen, _ = resolve(store, db, device, recording)
    return chosen["terms"] if chosen else []


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


def execute(store, device: str, order: Save | Select | Correct | Remove) -> dict:
    from ..notes.quota import reserve

    chosen_id = identifier(order.dictionary_id) if order.dictionary_id is not None else None
    with store.transaction() as db:
        dictionaries = listing(store, db, device)
        chosen = next((d for d in dictionaries if d["id"] == chosen_id), None)
        series = None
        assign = isinstance(order, Select)
        if isinstance(order, (Select, Correct)):
            repository.require(db, device, order.recording_id)
            series = series_of(store, db, device, order.recording_id)
        if isinstance(order, Correct):
            current, _ = resolve(store, db, device, order.recording_id)
            # A vocabulary correction does not freeze an inherited choice.
            assign = current is None or current["id"] != chosen_id
        if (
            isinstance(order, Save)
            or isinstance(order, Correct)
            and order.name is not None
            and chosen is None
        ):
            name = text(order.name, "name", MAX_NAME_CHARS)
            if chosen is None:
                if len(dictionaries) >= MAX_DICTIONARIES:
                    raise HubError(
                        "dictionaries_full",
                        f"Hay {len(dictionaries)} diccionarios; el máximo es {MAX_DICTIONARIES}",
                        http_status=409,
                        numbers={
                            "dictionaries": len(dictionaries),
                            "max_dictionaries": MAX_DICTIONARIES,
                        },
                    )
                chosen = {"id": chosen_id, "name": name, "terms": [], "series": []}
                dictionaries.append(chosen)
            else:
                chosen["name"] = name
        if chosen_id is not None and chosen is None:
            raise HubError("dictionary_not_found", "El diccionario no existe", http_status=404)
        if isinstance(order, Correct):
            wrong, right = (
                text(order.wrong, "wrong", MAX_TERM_CHARS),
                text(order.correct, "correct", MAX_TERM_CHARS),
            )
            entry = next(
                (e for e in chosen["terms"] if e["correct"].casefold() == right.casefold()), None
            )
            if entry is None:
                if len(chosen["terms"]) >= MAX_TERMS:
                    raise HubError(
                        "dictionary_full",
                        f"Hay {len(chosen['terms'])} términos; el máximo es {MAX_TERMS}",
                        http_status=409,
                        numbers={"terms": len(chosen["terms"]), "max_terms": MAX_TERMS},
                    )
                entry = {"id": str(uuid4()), "correct": right, "wrong": []}
            forms = [f for f in entry["wrong"] if f.casefold() != wrong.casefold()]
            if len(forms) >= MAX_FORMS:
                raise HubError(
                    "dictionary_forms_full",
                    f"Hay {len(forms)} formas equivocadas; el máximo es {MAX_FORMS}",
                    http_status=409,
                    numbers={"forms": len(forms), "max_forms": MAX_FORMS},
                )
            for old in chosen["terms"]:
                old["wrong"] = [f for f in old["wrong"] if f.casefold() != wrong.casefold()]
            entry.update(correct=right, wrong=[*forms, wrong])
            chosen["terms"] = [
                e for e in chosen["terms"] if e["id"] != entry["id"] and e["wrong"]
            ] + [entry]
        if isinstance(order, Remove):
            chosen["terms"] = [e for e in chosen["terms"] if e["id"] != order.entry_id]
        if isinstance(order, Select) and order.use_for_series:
            if series is None:
                raise HubError(
                    "dictionary_series_missing",
                    "La reunión no tiene serie de calendario",
                    http_status=409,
                )
            for dictionary in dictionaries:
                dictionary["series"] = [
                    s
                    for s in dictionary["series"]
                    if s["calendar_item_id"] != series["calendar_item_id"]
                ]
            if chosen:
                chosen["series"].append(series)
            count = sum(len(d["series"]) for d in dictionaries)
            if count > MAX_SERIES:
                raise HubError(
                    "dictionary_series_full",
                    f"Hay {count} series; el máximo es {MAX_SERIES}",
                    http_status=409,
                    numbers={"series": count, "max_series": MAX_SERIES},
                )
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
                    numbers={"bytes": size, "max_bytes": MAX_DICTIONARY_BYTES},
                )
        sealed = [
            (
                d["id"],
                store.encrypt(
                    json.dumps(d, ensure_ascii=False).encode(), f"dictionary:{device}:{d['id']}"
                ),
            )
            for d in dictionaries
        ]
        reserve(store, sum(len(body) for _, body in sealed))
        for identity, body in sealed:
            db.execute(
                "INSERT INTO meeting_dictionaries VALUES (?,?,?) ON CONFLICT(device_id,id) DO UPDATE SET body=excluded.body",
                (device, identity, body),
            )
        if assign:
            db.execute(
                "DELETE FROM recording_dictionaries WHERE device_id=? AND recording=?",
                (device, order.recording_id),
            )
            if chosen_id is not None and not (isinstance(order, Select) and order.use_for_series):
                db.execute(
                    "INSERT INTO recording_dictionaries VALUES (?,?,?)",
                    (device, order.recording_id, chosen_id),
                )
    return chosen or {"dictionary_id": None}


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
        # One order carries two phrases and a name; 16 KiB fits even JSON
        # escaping every character and rejects paragraphs before parsing.
        order = parse(
            await bounded_body(request, 16 * 1024), Order, "invalid_dictionary_order", "Diccionario"
        ).root
        result = await run(lambda: execute(store, device, order))

        return JSONResponse(result)
