"""The device's routes for its meetings, authenticated as every device route.

Every refusal is a `HubError` with its code and numbers; the app's handler
renders it in the step-2 format, with the request's `op`.
"""

from fastapi import Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict, Field

from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..notes.http import parse
from ..notes.models import MAX_PRIVATE_NOTES_CHARS, MAX_SPEAKERS, TemplateId
from ..notes.series_key import SeriesKeyError, valid_series_key
from . import dictionaries, repository, series, turns, views

# Bodies of the small requests: one name (≤ 128 characters), a few voices'
# names, a meeting's title (≤ 256) or a language code. Each character
# escaped as `\uXXXX` takes six bytes, so 16 KiB holds the largest valid one
# many times over and refuses anything else with its size before it is parsed.
MAX_SMALL_BODY_BYTES = 16 * 1024
# Private notes: their character ceiling with every character escaped (six
# bytes), plus 1 KiB for the field names and the revision.
MAX_PRIVATE_NOTES_BODY_BYTES = 6 * MAX_PRIVATE_NOTES_CHARS + 1024


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, hide_input_in_errors=True)


class ProfileRequest(Strict):
    owner_name: str


class SelectedTurn(Strict):
    at_ms: int = Field(ge=0)
    speaker_key: str = Field(min_length=1, max_length=256)


class SpeakersRequest(Strict):
    names: dict[str, str | None] = Field(max_length=MAX_SPEAKERS)
    turn: SelectedTurn | None = None


class PrivateNotesRequest(Strict):
    text: str = Field(max_length=MAX_PRIVATE_NOTES_CHARS)
    base_revision: int = Field(ge=0)


class TitleRequest(Strict):
    # Bounded by `repository.meeting_title`, which refuses with the numbers.
    title: str


class SeriesRequest(Strict):
    """«Usar lo mismo en toda la serie»: no dictionary leaves the series without one.

    `dictionary_id` is one, as Macs up to 0.10.1 send it; `dictionary_ids`
    is the whole list. One of the two (`dictionaries.named_ids`).
    """

    template_id: TemplateId
    dictionary_id: str | None = None
    dictionary_ids: list[str] | None = None


class LanguageRequest(Strict):
    # ISO 639-1, the shape `[meeting_transcription] language` takes.
    language: str = Field(pattern=r"^[a-z]{2}$")


def recording_of(value: str, field: str = "recording_id") -> str:
    try:
        return canonical_uuid(value)
    except ValueError:
        raise HubError(
            "invalid_parameters",
            f"Identificador de grabación no válido en {field}",
            numbers={"field": field},
        ) from None


def series_of(value: str | None) -> str:
    """A calendar series key from a query, or a refusal naming the field and its numbers."""
    if value is None:
        raise HubError(
            "invalid_parameters",
            "Falta calendar_item_id, la serie del calendario",
            numbers={"field": "calendar_item_id"},
        )
    try:
        return valid_series_key(value)
    except SeriesKeyError as error:
        raise HubError(
            "invalid_parameters",
            f"calendar_item_id no válido: {error.reason}",
            numbers={"field": "calendar_item_id", **error.numbers},
        ) from None


def digest_of(value: str | None) -> str | None:
    """A title's digest from a query (`series.title_digest`), or a refusal naming the field."""
    if value is None:
        return None
    if len(value) != 64 or any(character not in "0123456789abcdef" for character in value):
        raise HubError(
            "invalid_parameters",
            "title_digest no válido: es el SHA-256 en hexadecimal del título",
            numbers={"field": "title_digest", "chars": len(value), "expected_chars": 64},
        )
    return value


def integer(value: str | None, field: str, *, default: int, minimum: int, maximum: int) -> int:
    if value is None:
        return default
    try:
        number = int(value)
    except ValueError:
        number = None
    if number is None or not minimum <= number <= maximum:
        raise HubError(
            "invalid_parameters",
            f"{field} fuera de rango: entre {minimum} y {maximum}",
            numbers={"field": field, "min": minimum, "max": maximum},
        )
    return number


def mount(
    app,
    store,
    identity,
    bounded_body,
    store_executor,
    now,
    notes,
    default_language,
    recording_changed,
    transcription_signal,
    diarization_configured,
):
    repository_of_notes = notes.repository

    async def run(function):
        return await in_store(store_executor, function)

    async def body_of(request: Request, model, maximum: int):
        return parse(await bounded_body(request, maximum), model, "invalid_parameters", "Petición")

    def read(function):
        """`function(db)` in one snapshot of the store."""

        def reading():
            with store.read_snapshot() as db:
                return function(db)

        return run(reading)

    def transcript(db, device, recording):
        repository.require(db, device, recording)
        return turns.read(store, db, device, recording, default_language=default_language)

    dictionaries.mount(app, store, identity, bounded_body, run)

    @app.put("/v1/device/profile")
    async def put_profile(request: Request):
        device = identity(request)
        payload = await body_of(request, ProfileRequest, MAX_SMALL_BODY_BYTES)
        name = await run(
            lambda: repository.set_owner_name(store, device, payload.owner_name, now())
        )
        return JSONResponse({"owner_name": name})

    @app.get("/v1/recordings")
    async def list_recordings(
        request: Request,
        before_ms: str | None = None,
        before_id: str | None = None,
        limit: str | None = None,
    ):
        device = identity(request)
        before = integer(before_ms, "before_ms", default=None, minimum=1, maximum=2**63 - 1)
        after = None if before_id is None else recording_of(before_id, "before_id")
        page = integer(
            limit, "limit", default=views.MAX_LIST_PAGE, minimum=1, maximum=views.MAX_LIST_PAGE
        )
        return JSONResponse(
            await read(
                lambda db: views.device_listing(
                    store, repository_of_notes, db, device, before, page, after
                )
            )
        )

    @app.get("/v1/series/previous-meeting")
    async def previous_meeting(
        request: Request,
        calendar_item_id: str | None = None,
        before_ms: str | None = None,
        title_digest: str | None = None,
    ):
        """The earlier meeting of a series (or, failing that, of the same title)
        and its note's tasks, for Hoy."""
        device, key = identity(request), series_of(calendar_item_id)
        title = digest_of(title_digest)
        if before_ms is None:
            raise HubError(
                "invalid_parameters",
                "Falta before_ms, el inicio de la reunión siguiente",
                numbers={"field": "before_ms"},
            )
        before = integer(before_ms, "before_ms", default=None, minimum=1, maximum=2**63 - 1)
        found = await read(
            lambda db: series.previous(store, repository_of_notes, db, device, key, before, title)
        )
        return JSONResponse({"calendar_item_id": key, "before_ms": before, "previous": found})

    @app.put("/v1/recordings/{recording_id}/series")
    async def put_series(recording_id: str, request: Request):
        """The template and the dictionaries of the meeting's series, saved together."""
        device, recording = identity(request), recording_of(recording_id)
        payload = await body_of(request, SeriesRequest, MAX_SMALL_BODY_BYTES)
        chosen = dictionaries.named_ids(payload, required=False, code="invalid_parameters")
        saved = await run(
            lambda: series.use_for_series(
                store,
                notes.templates,
                device,
                recording,
                payload.template_id,
                chosen,
                now(),
            )
        )
        return JSONResponse(saved)

    @app.get("/v1/recordings/{recording_id}")
    async def read_recording(recording_id: str, request: Request):
        device, recording = identity(request), recording_of(recording_id)
        return JSONResponse(
            await read(
                lambda db: views.document(
                    store,
                    repository_of_notes,
                    db,
                    device,
                    recording,
                    default_language=default_language,
                )
            )
        )

    @app.get("/v1/recordings/{recording_id}/turns")
    async def read_turns(recording_id: str, request: Request, offset: str | None = None):
        device, recording = identity(request), recording_of(recording_id)
        start = integer(offset, "offset", default=0, minimum=0, maximum=2**31 - 1)
        return JSONResponse(
            await read(lambda db: views.turns_page(transcript(db, device, recording), start))
        )

    @app.put("/v1/recordings/{recording_id}/speakers")
    async def put_speakers(recording_id: str, request: Request):
        device, recording = identity(request), recording_of(recording_id)
        payload = await body_of(request, SpeakersRequest, MAX_SMALL_BODY_BYTES)
        before = await read(lambda db: transcript(db, device, recording))
        keys = {speaker.key for speaker in before.speakers}
        await run(
            lambda: repository.rename(
                store, device, recording, payload.names, keys, now(), turn=payload.turn
            )
        )
        after = await read(lambda db: transcript(db, device, recording))
        recording_changed(device, recording)
        return JSONResponse({"speakers": views.speakers(after)})

    @app.post("/v1/recordings/{recording_id}/final-pass")
    async def request_final_pass(recording_id: str, request: Request):
        from ..meeting_final_pass import FinalPassRepository

        device, recording = identity(request), recording_of(recording_id)
        await run(
            lambda: FinalPassRepository(store).request(
                device, recording, configured=diarization_configured
            )
        )
        # The note stays as it is: «Rehacer resumen» is its own order
        # (`POST …/note`), which keeps the version edited by hand.
        recording_changed(device, recording)
        return JSONResponse({"recording_id": recording, "requested": True})

    @app.put("/v1/recordings/{recording_id}/private-notes")
    async def put_private_notes(recording_id: str, request: Request):
        device, recording = identity(request), recording_of(recording_id)
        payload = await body_of(request, PrivateNotesRequest, MAX_PRIVATE_NOTES_BODY_BYTES)
        revision = await run(
            lambda: repository.save_private_notes(
                store, device, recording, payload.text, payload.base_revision, now()
            )
        )
        recording_changed(device, recording)
        return JSONResponse({"revision": revision})

    @app.put("/v1/recordings/{recording_id}/title")
    async def put_title(recording_id: str, request: Request):
        """The user's name for the meeting; an empty one gives it back its own."""
        device, recording = identity(request), recording_of(recording_id)
        payload = await body_of(request, TitleRequest, MAX_SMALL_BODY_BYTES)
        await run(lambda: repository.set_title(store, device, recording, payload.title, now()))
        head = await read(
            lambda db: views.heading(store, repository_of_notes, db, device, recording)
        )
        recording_changed(device, recording)
        return JSONResponse({"title": head["title"]})

    @app.put("/v1/recordings/{recording_id}/language")
    async def put_language(recording_id: str, request: Request):
        """Transcribe the meeting in this language, and again what it has.

        The note, if one was asked for, is written again once the transcript
        is complete, with its template; the user's edits are set aside as when the
        template changes.
        """
        device, recording = identity(request), recording_of(recording_id)
        payload = await body_of(request, LanguageRequest, MAX_SMALL_BODY_BYTES)
        repeated = await run(
            lambda: repository.change_language(
                store, device, recording, payload.language, default_language, now()
            )
        )
        if repeated:
            transcription_signal.set()
            asked = await read(lambda db: repository_of_notes.inputs(device, recording, db=db))
            if asked is not None:
                await run(
                    lambda: repository_of_notes.request(
                        device,
                        recording,
                        now(),
                        notes.configured,
                        template_id=asked.template_id,
                    )
                )
                notes.signal.set()
        recording_changed(device, recording)
        return JSONResponse({"language": payload.language, "requeued": repeated})
