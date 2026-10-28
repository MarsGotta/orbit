"""The user's own note templates and the series each one is assigned to.

Shared across the personal hub; existing device storage keys are preserved.
A custom template is encrypted at rest and has the same bounds as the built-in ones (`templates.py`). The built-ins are never changed:
editing one means saving a custom copy that names where it came from
(`forked_from`). Ids are chosen by the client, so creating one is a `PUT` that
a lost receipt can repeat without making two.

An assignment (a link) says which template writes the notes of a series, keyed
by the calendar's `calendar_item_id`. Deleting a template releases its links,
which sends those series back to General; notes already written keep their
template's name and sections, because the note stores them.
"""

import json

from pydantic import Field, field_validator

from ..protocol import HubError
from .models import NoteInputs, SeriesKey, Strict, TemplateId, TemplateSource
from .templates import (
    BUILTIN_TEMPLATES,
    DEFAULT_TEMPLATE_ID,
    MAX_CONTEXT_CHARS,
    MAX_NAME_CHARS,
    MAX_SECTIONS,
    Template,
    TemplateSection,
    builtin,
)

# Custom templates per personal hub. What it protects: the Settings list the user
# scrolls and the listing the daemon reads on every connection. Why this size:
# six built-ins plus 32 of their own is more kinds of meeting than anyone keeps a
# template for. What it is not: a bound on one template, which has its own.
MAX_CUSTOM_TEMPLATES = 32
# Bytes of the whole `GET /v1/note-templates` answer. What it protects: the
# daemon reads hub HTTP bodies up to 1 MiB (`MAX_HUB_HTTP_BODY_BYTES` in
# `contracts/constants.json`), so a listing past that could never be read.
# Why this size: 768 KiB leaves a quarter of that body for the built-ins and
# the JSON around them. A real template is a few KiB, so 32 of them use a
# fraction; only templates written to every bound at once (about 116 KiB each)
# reach it, and then the refusal says how many bytes and how many are allowed.
# What it is not: the per-template bounds, which are the built-ins'.
MAX_TEMPLATE_LISTING_BYTES = 768 * 1024
# Assignments per personal hub. What it protects: `GET /v1/note-template-links`,
# which must also fit the daemon's 1 MiB body. Why this size: 256 links with
# the longest series key (1 024 bytes) are about 300 KiB; a calendar has a few
# dozen recurring series. What it is not: a limit on how many series have
# notes, assigned or not.
MAX_TEMPLATE_LINKS = 256

SCHEMA = """
CREATE TABLE IF NOT EXISTS note_templates (
    device_id TEXT NOT NULL, id TEXT NOT NULL, body BLOB NOT NULL,
    created REAL NOT NULL, updated REAL NOT NULL,
    PRIMARY KEY(device_id, id));
CREATE TABLE IF NOT EXISTS note_template_links (
    device_id TEXT NOT NULL, series TEXT NOT NULL, template_id TEXT NOT NULL,
    updated REAL NOT NULL,
    PRIMARY KEY(device_id, series));
CREATE INDEX IF NOT EXISTS note_template_links_template
    ON note_template_links(device_id, template_id);
"""


class TemplateDraft(Strict):
    """What the client sends to create or replace a custom template."""

    name: str = Field(min_length=1, max_length=MAX_NAME_CHARS)
    context: str = Field(default="", max_length=MAX_CONTEXT_CHARS)
    sections: tuple[TemplateSection, ...] = Field(min_length=1, max_length=MAX_SECTIONS)
    # The template this one was copied from, when the user edited it; kept from
    # the first save, so replacing a copy never rewrites its origin.
    forked_from: TemplateId | None = None

    @field_validator("name")
    @classmethod
    def visible_name(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("the name is blank")
        return value


class LinkRequest(Strict):
    calendar_item_id: SeriesKey
    template_id: TemplateId


class UnlinkRequest(Strict):
    calendar_item_id: SeriesKey


def not_found(template_id: str) -> HubError:
    return HubError(
        "note_template_not_found",
        f"Template not found: «{template_id}»",
        numbers={"template_id": template_id},
    )


def builtin_refusal(template_id: str) -> HubError:
    return HubError(
        "note_template_builtin",
        f"Template «{template_id}» belongs to Orbit and cannot be changed or deleted: save the changes as your own template with forked_from",
        http_status=409,
        numbers={"template_id": template_id},
    )


def _encoded(value: dict) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def builtin_view(template: Template) -> dict:
    return {**template.model_dump(mode="json"), "forked_from": None, "updated_at_ms": None}


class TemplateRepository:
    """Shares Store's writer and encryption; never owns another connection."""

    def __init__(self, store):
        self.store = store

    def _seal(self, device: str, template_id: str, value: dict) -> bytes:
        return self.store.encrypt(_encoded(value), f"note-template:{device}:{template_id}")

    def _open(self, device: str, row) -> dict:
        body = self.store.decrypt(row["body"], f"note-template:{row['device_id']}:{row['id']}")
        return json.loads(body)

    def _customs(self, db, device: str) -> list[dict]:
        rows = db.execute(
            "SELECT device_id,id,body,updated FROM note_templates ORDER BY created,id,device_id",
        ).fetchall()
        # Older Macs could create the same client-chosen id independently.
        # One public id resolves to the most recently edited stored template.
        by_id, versions = {}, {}
        for row in rows:
            value = self._open(device, row)
            version = (row["updated"], row["device_id"])
            if value["id"] not in versions or version > versions[value["id"]]:
                by_id[value["id"]] = value
                versions[value["id"]] = version
        return list(by_id.values())

    def _custom(self, db, device: str, template_id: str) -> dict | None:
        row = db.execute(
            "SELECT device_id,id,body FROM note_templates WHERE id=? "
            "ORDER BY updated DESC,device_id DESC LIMIT 1",
            (template_id,),
        ).fetchone()
        return None if row is None else self._open(device, row)

    def listing(self, device: str) -> dict:
        """Built-ins first, in their fixed order, then the user's own in creation order."""
        with self.store.read_snapshot() as db:
            customs = self._customs(db, device)
        return {"templates": [builtin_view(t) for t in BUILTIN_TEMPLATES] + customs}

    def put(self, device: str, template_id: str, draft: TemplateDraft, now: float):
        """Create or replace `template_id`; returns the template and whether it is new.

        Every refusal is decided on a snapshot before the write: the store's
        single writer thread runs this whole call, so nothing changes between.
        """
        if builtin(template_id) is not None:
            raise builtin_refusal(template_id)
        with self.store.read_snapshot() as db:
            existing = self._custom(db, device, template_id)
            customs = self._customs(db, device)
            if existing is not None:
                origin = existing["forked_from"]
            elif draft.forked_from is None:
                origin = None
            else:
                source = builtin(draft.forked_from)
                if source is not None:
                    origin = {"id": source.id, "name": source.name}
                else:
                    copied = self._custom(db, device, draft.forked_from)
                    if copied is None:
                        raise not_found(draft.forked_from)
                    origin = {"id": copied["id"], "name": copied["name"]}
        template = {
            "id": template_id,
            "name": draft.name,
            "context": draft.context,
            "sections": [section.model_dump(mode="json") for section in draft.sections],
            "builtin": False,
            "forked_from": origin,
            "updated_at_ms": int(now * 1000),
        }
        others = [t for t in customs if t["id"] != template_id]
        if existing is None and len(others) >= MAX_CUSTOM_TEMPLATES:
            raise HubError(
                "note_templates_full",
                f"There are already {len(others)} custom templates and the maximum is {MAX_CUSTOM_TEMPLATES}: delete one before creating another",
                http_status=409,
                numbers={"templates": len(others), "max_templates": MAX_CUSTOM_TEMPLATES},
            )
        listing = {"templates": [builtin_view(t) for t in BUILTIN_TEMPLATES] + others + [template]}
        size = len(_encoded(listing))
        if size > MAX_TEMPLATE_LISTING_BYTES:
            raise HubError(
                "note_templates_too_large",
                f"With this template the list would be {size} bytes and the maximum is {MAX_TEMPLATE_LISTING_BYTES}: shorten its instructions or delete another",
                http_status=413,
                numbers={"bytes": size, "max_bytes": MAX_TEMPLATE_LISTING_BYTES},
            )
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT device_id FROM note_templates WHERE id=? ORDER BY updated DESC,device_id DESC LIMIT 1",
                (template_id,),
            ).fetchone()
            device = row["device_id"] if row else device
        sealed = self._seal(device, template_id, template)
        self.store.ensure_capacity(len(sealed))
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO note_templates(device_id,id,body,created,updated) VALUES (?,?,?,?,?) "
                "ON CONFLICT(device_id,id) DO UPDATE SET body=excluded.body, "
                "updated=excluded.updated",
                (device, template_id, sealed, now, now),
            )
        return template, existing is None

    def delete(self, device: str, template_id: str) -> dict:
        """Idempotent: a delete whose receipt was lost answers `deleted: false` again."""
        if builtin(template_id) is not None:
            raise builtin_refusal(template_id)
        with self.store.transaction() as db:
            deleted = db.execute("DELETE FROM note_templates WHERE id=?", (template_id,)).rowcount
            released = db.execute(
                "DELETE FROM note_template_links WHERE template_id=?",
                (template_id,),
            ).rowcount
        return {"template_id": template_id, "deleted": deleted > 0, "links_released": released}

    def links(self, device: str) -> dict:
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT series, template_id, updated FROM note_template_links "
                "ORDER BY updated,device_id",
            ).fetchall()
        rows = sorted({row["series"]: row for row in rows}.values(), key=lambda row: row["series"])
        return {
            "links": [
                {
                    "calendar_item_id": row["series"],
                    "template_id": row["template_id"],
                    "updated_at_ms": int(row["updated"] * 1000),
                }
                for row in rows
            ]
        }

    def link(self, device: str, request: LinkRequest, now: float) -> dict:
        with self.store.transaction() as db:
            return self.link_in(db, device, request.calendar_item_id, request.template_id, now)

    def link_in(self, db, device: str, series: str, template_id: str, now: float) -> dict:
        """Assign `template_id` to `series` inside the caller's write transaction.

        A refusal of anything written with it in that transaction (the
        series' dictionary, `meetings/series.py`) rolls this back too.
        """
        if builtin(template_id) is None and self._custom(db, device, template_id) is None:
            raise not_found(template_id)
        count = db.execute(
            "SELECT count(DISTINCT series) FROM note_template_links WHERE series<>?",
            (series,),
        ).fetchone()[0]
        if count >= MAX_TEMPLATE_LINKS:
            raise HubError(
                "note_template_links_full",
                f"There are already {count} series with assigned templates and the maximum is {MAX_TEMPLATE_LINKS}: remove one before assigning another",
                http_status=409,
                numbers={"links": count, "max_links": MAX_TEMPLATE_LINKS},
            )
        owner = db.execute(
            "SELECT device_id FROM note_template_links WHERE series=? ORDER BY updated DESC,device_id DESC LIMIT 1",
            (series,),
        ).fetchone()
        device = owner["device_id"] if owner else device
        self.store.ensure_capacity(len(series.encode()) + 128)
        db.execute(
            "INSERT INTO note_template_links(device_id,series,template_id,updated) "
            "VALUES (?,?,?,?) ON CONFLICT(device_id,series) DO UPDATE SET "
            "template_id=excluded.template_id, updated=excluded.updated",
            (device, series, template_id, now),
        )
        return {
            "calendar_item_id": series,
            "template_id": template_id,
            "updated_at_ms": int(now * 1000),
        }

    def unlink(self, device: str, series: str) -> dict:
        with self.store.transaction() as db:
            deleted = db.execute(
                "DELETE FROM note_template_links WHERE series=?", (series,)
            ).rowcount
        return {"calendar_item_id": series, "deleted": deleted > 0}

    def find(self, device: str, template_id: str) -> Template | None:
        fixed = builtin(template_id)
        if fixed is not None:
            return fixed
        with self.store.read_snapshot() as db:
            found = self._custom(db, device, template_id)
        if found is None:
            return None
        return Template(
            id=found["id"],
            name=found["name"],
            context=found["context"],
            sections=tuple(TemplateSection(**section) for section in found["sections"]),
        )

    def resolve(self, device: str, inputs: NoteInputs) -> tuple[Template | None, TemplateSource]:
        """The template a note is written with, and how it was chosen.

        A template named in the request always wins, and one that does not
        exist is reported, never replaced. Otherwise the event's series picks
        it; with no series or no assignment, General.
        """
        if inputs.template_id is not None:
            return self.find(device, inputs.template_id), "requested"
        series = inputs.event.calendar_item_id if inputs.event is not None else None
        if series is not None:
            with self.store.read_snapshot() as db:
                row = db.execute(
                    "SELECT template_id FROM note_template_links WHERE series=? "
                    "ORDER BY updated DESC,device_id DESC LIMIT 1",
                    (series,),
                ).fetchone()
            linked = self.find(device, row["template_id"]) if row is not None else None
            if linked is not None:
                return linked, "series"
        return builtin(DEFAULT_TEMPLATE_ID), "default"
