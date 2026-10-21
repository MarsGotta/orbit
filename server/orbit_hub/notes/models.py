"""The note of a recording: what the Mac sends, what the model writes, what Mars edits."""

import hashlib
from typing import Annotated, Literal

from pydantic import AfterValidator, BaseModel, ConfigDict, Field

from ..meetings.bounds import MAX_PERSON_NAME_CHARS
from .series_key import valid_series_key
from .templates import MAX_NAME_CHARS, MAX_SECTIONS

# Private notes are typed by Mars during a meeting. 65 536 characters is about
# ten thousand words, far beyond what anyone types live; the bound keeps one
# request (`PUT …/private-notes`) and the prompt built from it small.
MAX_PRIVATE_NOTES_CHARS = 64 * 1024
# Attendees and voices name people in the prompt. Calendars cap invitations
# far above this, but a meeting with more than 200 invitees or 64 distinct
# voices is a broadcast, and its note would not name them all anyway. Since
# SIM-40 the voices' names are one request of their own (`PUT …/speakers`).
MAX_ATTENDEES = 200
MAX_SPEAKERS = 64
# The calendar event of a recording (P9-11): its title is context for the
# prompt, and an address longer than RFC 5321's 254 characters is not one.
# The Mac keeps within both (`recording_notes::event_context`).
MAX_EVENT_TITLE_CHARS = 512
MAX_EMAIL_CHARS = 254
# What the model may write, per field. They bound the stored note and the
# frame that carries it to the Mac; a section longer than 16 KiB or more than
# sixty tasks is a transcript, not a note.
MAX_SECTION_CHARS = 16 * 1024
MAX_ACTION_ITEMS = 60
MAX_LIST_ITEMS = 40
MAX_ITEM_CHARS = 1000
MAX_TITLE_CHARS = 200

NoteState = Literal[
    "not_configured",
    "waiting_transcript",
    "queued",
    "generating",
    "ready",
    "failed",
]


TEMPLATE_ID_PATTERN = r"^[a-z0-9][a-z0-9_-]*$"
MAX_TEMPLATE_ID_CHARS = 64

SeriesKey = Annotated[str, AfterValidator(valid_series_key)]
TemplateId = Annotated[
    str, Field(min_length=1, max_length=MAX_TEMPLATE_ID_CHARS, pattern=TEMPLATE_ID_PATTERN)
]
# How the template of a note was chosen: named in the request, assigned to the
# event's series (P9-05), or the default because neither applied.
TemplateSource = Literal["requested", "series", "default"]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, hide_input_in_errors=True)


class Attendee(Strict):
    name: str = Field(default="", max_length=MAX_NAME_CHARS * 2)
    email: str = Field(default="", max_length=MAX_EMAIL_CHARS)
    is_self: bool = False


class EventContext(Strict):
    title: str = Field(default="", max_length=MAX_EVENT_TITLE_CHARS)
    starts_at_ms: int | None = Field(default=None, ge=0)
    ends_at_ms: int | None = Field(default=None, ge=0)
    attendees: tuple[Attendee, ...] = Field(default=(), max_length=MAX_ATTENDEES)
    # P9-05: the calendar's stable identifier of the event, shared by every
    # occurrence of a recurring one. It picks the series' template when the
    # request names none. A Mac that does not send it gets the default.
    calendar_item_id: SeriesKey | None = None


class NoteInputs(BaseModel):
    """What a note is written from besides its transcript.

    Stored with the note when it is asked for: the template, and the meeting's
    title and event from its manifest. The owner's name, the private notes and
    the language are the hub's own and are read when the note is written
    (`NotesWorker.generate`), so a later change reaches the next note. Rows
    stored by earlier hubs carry the Mac's fields (`speakers`…), ignored.
    """

    model_config = ConfigDict(extra="ignore", frozen=True, hide_input_in_errors=True)
    # None lets the hub choose: the template assigned to the event's series,
    # else General. A template Mars picked by hand is always named here.
    dictionary_terms: tuple[str, ...] = ()
    template_id: TemplateId | None = None
    title: str = Field(default="", max_length=512)
    event: EventContext | None = None
    # The local microphone is Mars; this is how the note names her.
    owner_name: str = Field(default="Yo", min_length=1, max_length=MAX_PERSON_NAME_CHARS)
    private_notes: str = Field(default="", max_length=MAX_PRIVATE_NOTES_CHARS)
    language: str = Field(default="es", pattern=r"^[a-z]{2}(-[A-Z]{2})?$")


class Section(Strict):
    name: str = Field(min_length=1, max_length=MAX_NAME_CHARS)
    markdown: str = Field(max_length=MAX_SECTION_CHARS)


class ActionItem(Strict):
    id: str = Field(pattern=r"^a[0-9a-f]{12}$")
    owner: str | None = Field(default=None, max_length=MAX_NAME_CHARS * 2)
    text: str = Field(min_length=1, max_length=MAX_ITEM_CHARS)
    due: str | None = Field(default=None, max_length=MAX_NAME_CHARS)


def action_id(owner: str | None, text: str) -> str:
    """Stable across regenerations that keep the same task: done marks survive."""
    canonical = (owner or "").strip().casefold() + "\x1f" + " ".join(text.split()).casefold()
    return "a" + hashlib.sha256(canonical.encode()).hexdigest()[:12]


class GeneratedNote(Strict):
    final_pass_error: dict | None = None
    title: str = Field(max_length=MAX_TITLE_CHARS)
    short_summary: str = Field(max_length=MAX_ITEM_CHARS)
    sections: tuple[Section, ...] = Field(max_length=MAX_SECTIONS)
    action_items: tuple[ActionItem, ...] = Field(max_length=MAX_ACTION_ITEMS)
    decisions: tuple[str, ...] = Field(max_length=MAX_LIST_ITEMS)
    open_topics: tuple[str, ...] = Field(max_length=MAX_LIST_ITEMS)
    template_id: str
    template_name: str
    # Absent in notes written before P9-05, which always named their template.
    template_source: TemplateSource | None = None
    model: str = Field(max_length=128)
    generated_at_ms: int = Field(ge=0)
    # What the note was written from, so a reader can tell a partial source.
    segments: int = Field(ge=0)
    duration_ms: int = Field(ge=0)
    parts: int = Field(ge=1)


class NoteEdits(Strict):
    """Mars's own changes, kept apart from what the model wrote."""

    title: str | None = Field(default=None, max_length=MAX_TITLE_CHARS)
    sections: tuple[Section, ...] | None = Field(default=None, max_length=MAX_SECTIONS)
    done: dict[str, bool] = Field(default_factory=dict, max_length=MAX_ACTION_ITEMS * 4)


class EditRequest(Strict):
    base_revision: int = Field(ge=0)
    title: str | None = Field(default=None, max_length=MAX_TITLE_CHARS)
    sections: tuple[Section, ...] | None = Field(default=None, max_length=MAX_SECTIONS)
    done: dict[str, bool] = Field(default_factory=dict, max_length=MAX_ACTION_ITEMS)


class GenerateRequest(Strict):
    """`POST …/note`: write the note again, or for the first time.

    The hub asks for a meeting's note itself when it is finalized (SIM-40);
    this is for another template, "write it again", or a recording older than
    notes. Mars's edits are kept aside, never lost: the document returns them
    as `superseded_edits`. None lets the hub choose the series' template, else
    General.
    """

    template_id: TemplateId | None = None
