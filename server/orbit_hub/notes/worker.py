"""Writes the note of each requested recording once its transcript is complete.

One job at a time: the processing model has one slot.
"""

import asyncio
from collections.abc import Callable
from contextlib import nullcontext, suppress

from ..log import report, stop_if_asked, supervised
from ..meeting_final_pass import note_ready, status
from ..meetings import repository as meetings
from ..meetings import turns
from ..processing import ProcessingClient, ProcessingError
from ..protocol import HubError
from .custom_templates import TemplateRepository
from .generation import (
    answer_schema,
    build_note,
    note_messages,
    part_messages,
    partial_for_reduce,
    reduce_messages,
    split,
    transcript_budget,
)
from .quota import failure_of
from .repository import WAIT_TRANSCRIPT_SECONDS, NoteRepository

# The transcription worker signals each finished group; this tick only covers
# a lost signal.
IDLE_TICK_SECONDS = 5
# Failures the next attempt can fix: the provider was slow, down, busy or wrote
# something that does not parse. Three retries over about twelve minutes cover
# a model host restarting; anything else needs a person (a key, a bound) and is
# reported with its numbers instead of being repeated.
TRANSIENT = frozenset(
    {
        "provider_timeout",
        "provider_unreachable",
        "provider_http_error",
        "invalid_response",
        "invalid_json_content",
        "note_schema_mismatch",
    }
)
RETRY_DELAYS_SECONDS = (30, 120, 600)


def retry_at(code: str, attempt: int, now: float) -> float | None:
    """When to try again after a failure, or None when it needs a person."""
    if code in TRANSIENT and attempt < len(RETRY_DELAYS_SECONDS):
        return now + RETRY_DELAYS_SECONDS[attempt]
    return None


class NotesWorker:
    def __init__(
        self,
        repository: NoteRepository,
        now: Callable[[], float],
        *,
        signal: asyncio.Event,
        store_executor,
        client: ProcessingClient | None = None,
        default_language: str | None = None,
        recording_changed=None,
    ):
        self.repository, self.now = repository, now
        self.signal, self.executor = signal, store_executor
        self.client = client
        # `[meeting_transcription] language`: what a group transcribed before
        # the hub kept a language reads as (`meetings.turns.read`).
        self.default_language = default_language
        # Tells the owning device's connections that a recording changed.
        self.recording_changed = recording_changed or (lambda device, recording: None)
        self.templates = TemplateRepository(repository.store)
        self._task: asyncio.Task | None = None

    @property
    def configured(self) -> bool:
        return self.client is not None and getattr(self.client, "configured", True)

    async def _run(self, function, *args):
        return await asyncio.get_running_loop().run_in_executor(self.executor, function, *args)

    def start(self) -> None:
        self.repository.initialize()
        self.repository.recover_interrupted(self.now())
        if self.configured:
            self.repository.requeue_unconfigured(self.now())
        self._task = asyncio.create_task(supervised("notes", self._loop))

    async def close(self) -> None:
        if self._task is not None:
            self._task.cancel()
            with suppress(asyncio.CancelledError):
                await self._task
        if self.client is not None:
            await self.client.close()

    async def _loop(self) -> None:
        while True:
            stop_if_asked()
            # Cleared before looking, never after: a request that arrives while
            # the claims run sets it again and the wait below returns at once.
            if self.signal.is_set():
                await self._run(self.repository.wake_waiting)
            self.signal.clear()
            if await self.run_once():
                continue
            with suppress(TimeoutError):
                await asyncio.wait_for(self.signal.wait(), IDLE_TICK_SECONDS)

    async def run_once(self) -> bool:
        """Do the next due note.

        Returns whether there was one. One bad job must not stop the others:
        an unexpected failure is recorded on that job with an internal cause.
        """
        job = await self._run(self.repository.claim, self.now())
        if job is not None:
            self.recording_changed(job["device_id"], job["recording"])
            try:
                await self.generate(job)
            except Exception:  # noqa: BLE001 — one bad note must not stop the others.
                report(event="note_generation_crashed", code="unexpected")
                await self._run(
                    self.repository.fail, job, {"code": "note_internal_error"}, self.now(), None
                )
            self.recording_changed(job["device_id"], job["recording"])
            return True
        return False

    def material(self, device: str, recording: str):
        """The transcript as every reader sees it, its readiness, the private
        notes and the user's name for the meeting (None when they gave none)."""
        store = self.repository.store
        with store.read_snapshot() as db:
            ready = note_ready(store, db, device, recording)
            transcript = turns.read(
                store, db, device, recording, default_language=self.default_language
            )
            private, _revision = meetings.private_notes(store, db, device, recording)
            title = meetings.title(store, db, device, recording)
        return ready, transcript, private, title

    async def generate(self, job: dict) -> None:
        device, recording, stored = job["device_id"], job["recording"], job["inputs"]
        ready, transcript, private, title = await self._run(self.material, device, recording)
        if not ready:
            now = self.now()
            await self._run(
                self.repository.wait_transcript, job, now + WAIT_TRANSCRIPT_SECONDS, now
            )
            return
        if not self.configured:
            await self._fail(job, ProcessingError("processing_not_configured"))
            return
        # Read now, not when the note was asked for: a voice or the meeting
        # renamed, or a private note typed since, reaches this note.
        inputs = stored.model_copy(
            update={
                "owner_name": transcript.local_name,
                "dictionary_terms": tuple(e["correct"] for e in transcript.dictionary_terms),
                "private_notes": private,
                "language": transcript.language or stored.language,
                "title": title or stored.title,
            }
        )
        template, source = await self._run(self.templates.resolve, device, inputs)
        if template is None:
            await self._fail(
                job, ProcessingError("note_template_not_found", template_id=inputs.template_id)
            )
            return
        lines = transcript.lines()
        if not lines:
            await self._fail(job, ProcessingError("transcript_empty", segments=transcript.groups))
            return
        try:
            note = await self._write(template, inputs, lines, transcript, source)
        except ProcessingError as failure:
            await self._fail(job, failure)
            return
        with self.repository.store.read_snapshot() as db:
            final = status(db, device, recording)
        if final is not None and final["error"] is not None:
            note = note.model_copy(update={"final_pass_error": final["error"]})
        await self._store(job, note)

    async def _store(self, job: dict, note) -> None:
        """Store a finished result; a full hub fails the job with its cause and numbers.

        Not retried: freeing the hub's disk or raising its quota needs a person.
        """
        try:
            await self._run(self.repository.complete, job, note, self.now())
        except HubError as failure:
            stored = failure_of(failure)
            if stored is None:
                raise
            error = {**stored, "attempt": job["attempts"] + 1}
            await self._run(self.repository.fail, job, error, self.now(), None)

    async def _write(self, template, inputs, lines, transcript, source):
        session = (
            self.client.session() if hasattr(self.client, "session") else nullcontext(self.client)
        )
        async with session as client:
            return await self._write_with(client, template, inputs, lines, transcript, source)

    async def _write_with(self, client, template, inputs, lines, transcript, source):
        settings = client.configuration
        budget = transcript_budget(settings.context_tokens, settings.max_completion_tokens, inputs)
        parts = split(lines, budget)
        duration = transcript.audio_ms
        if len(parts) == 1:
            completion = await client.complete_json(
                purpose="note",
                messages=note_messages(template, inputs, lines, duration),
                schema_name="orbit_note",
                schema=answer_schema(template, partial=False),
            )
        else:
            partials = []
            for index, part in enumerate(parts):
                answer = await client.complete_json(
                    purpose="note_part",
                    messages=part_messages(template, inputs, part, index, len(parts)),
                    schema_name="orbit_note_part",
                    schema=answer_schema(template, partial=True),
                )
                partials.append(partial_for_reduce(answer.content, template))
            completion = await client.complete_json(
                purpose="note_reduce",
                messages=reduce_messages(template, inputs, partials, duration),
                schema_name="orbit_note",
                schema=answer_schema(template, partial=False),
            )
        return build_note(
            completion.content,
            template,
            inputs,
            model=completion.model,
            generated_at_ms=int(self.now() * 1000),
            segments=transcript.groups,
            duration_ms=duration,
            parts=len(parts),
            template_source=source,
        )

    async def _fail(self, job: dict, failure: ProcessingError) -> None:
        attempt = job["attempts"]
        now = self.now()
        error = {**failure.as_dict(), "attempt": attempt + 1}
        await self._run(self.repository.fail, job, error, now, retry_at(failure.code, attempt, now))
