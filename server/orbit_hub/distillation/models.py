"""Bounded review contracts; model output contains patches, never whole replacements."""

from typing import Literal

from pydantic import Field, JsonValue

from ..vault.models import MAX_NOTE_BYTES, Strict

# Twenty-four touched notes accommodates a normal meeting's people/projects,
# while bounding review size and write recovery. It does not limit vault size.
MAX_CHANGES = 24
# Proposal patches and their before images need a separate budget from read envelopes.
MAX_PROPOSAL_BYTES = 8 * 1024 * 1024
# Background catalogue/capture and commit I/O has no app reply deadline. Two
# minutes admits hundreds of round trips while bounding occupation of the job
# worker; this is not the seven-second budget of an ordinary app read.
CONNECTOR_JOB_SECONDS = 120


class Quote(Strict):
    id: str = Field(pattern=r"^[a-zA-Z0-9_-]{1,40}$")
    at_ms: int = Field(ge=0)
    speaker: str = Field(min_length=1, max_length=256)
    text: str = Field(min_length=1, max_length=4096)


class Property(Strict):
    key: str = Field(min_length=1, max_length=128)
    after: JsonValue
    quote_ids: list[str] = Field(min_length=1, max_length=20)


class Lines(Strict):
    section: str = Field(max_length=256)
    before: str = Field(max_length=MAX_NOTE_BYTES)
    after: str = Field(max_length=MAX_NOTE_BYTES)
    quote_ids: list[str] = Field(min_length=1, max_length=20)


class Change(Strict):
    path: str = Field(min_length=1, max_length=4096)
    title: str = Field(min_length=1, max_length=256)
    properties: list[Property] = Field(max_length=40)
    lines: list[Lines] = Field(max_length=80)


class Answer(Strict):
    summary: str = Field(min_length=1, max_length=4096)
    quotes: list[Quote] = Field(min_length=1, max_length=200)
    changes: list[Change] = Field(min_length=1, max_length=MAX_CHANGES)


class ChangeEdit(Strict):
    include: bool | None = None
    properties: dict[str, JsonValue] = Field(default_factory=dict, max_length=40)
    lines: dict[str, str] = Field(default_factory=dict, max_length=80)


class Review(Strict):
    revision: int = Field(ge=0)
    changes: dict[str, ChangeEdit] = Field(default_factory=dict, max_length=MAX_CHANGES)
    decisions: dict[str, Literal["incoming", "current", "both"]] = Field(
        default_factory=dict, max_length=200
    )
