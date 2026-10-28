"""A note written by a fake model, for tests that read notes.

`FakeModel` answers like the processing client with a fixed, valid note; the
note worker that uses it is the real one, against the real store. No provider
and no network.
"""

import asyncio
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

from orbit_hub.meetings import repository as meetings
from orbit_hub.notes import NoteRepository, NotesWorker
from orbit_hub.processing import Completion, ProcessingConfiguration

MODEL = "qwen3.8-flash-next"


def configuration(**changes) -> ProcessingConfiguration:
    return ProcessingConfiguration.model_validate(
        {
            "endpoint": "https://llm.example.test/v1/chat/completions",
            "model": MODEL,
            "credential_file": "/srv/orbit/secrets/processing.key",
            **changes,
        }
    )


def note_answer(sections: int = 4, *, open_topics=(), decisions=("Aprobado",)) -> dict:
    """What the model writes for the four sections of the General template."""
    return {
        "title": "Comité",
        "short_summary": "Resumen corto.",
        "sections": {f"s{index + 1}": f"- punto {index + 1}" for index in range(sections)},
        "action_items": [{"owner": "Lucía", "text": "Enviar el acta", "due": ""}],
        "decisions": list(decisions),
        "open_topics": list(open_topics),
    }


@dataclass
class FakeModel:
    configuration: ProcessingConfiguration = field(default_factory=configuration)
    # Each purpose takes its next scripted answer, or a valid default note.
    answers: dict = field(default_factory=dict)
    # What each call asked, for a test that reads the prompt.
    calls: list = field(default_factory=list)

    def script(self, purpose: str, *answers) -> None:
        self.answers.setdefault(purpose, []).extend(answers)

    def purposes(self) -> list[str]:
        return [call["purpose"] for call in self.calls]

    def last(self, purpose: str) -> dict:
        return [call for call in self.calls if call["purpose"] == purpose][-1]

    async def complete_json(self, *, purpose, messages, schema_name, schema):
        self.calls.append(
            {"purpose": purpose, "messages": messages, "schema_name": schema_name, "schema": schema}
        )
        queue = self.answers.get(purpose) or []
        answer = queue.pop(0) if queue else note_answer()
        if isinstance(answer, Exception):
            raise answer
        if purpose == "note_part":
            answer = {
                key: value for key, value in answer.items() if key not in ("title", "short_summary")
            }
        return Completion(answer, MODEL, 100, 50, 10)

    async def close(self):
        pass


async def write_note(
    store,
    owner,
    recording,
    *,
    answers=(),
    now=1000.0,
    owner_name=None,
    private_notes=None,
    speakers=None,
    model=None,
) -> NotesWorker:
    """Ask for the note and run the real worker once on it, as the hub would.

    What the user adds is the hub's own data: the device's owner, the
    recording's private notes and the names of its voices.
    """
    if owner_name is not None:
        meetings.set_owner_name(store, owner, owner_name, now)
    if private_notes is not None:
        meetings.save_private_notes(store, owner, recording, private_notes, 0, now)
    if speakers is not None:
        meetings.rename(store, owner, recording, speakers, set(speakers), now)
    executor = ThreadPoolExecutor(max_workers=1)
    model = model or FakeModel()
    model.script("note", *answers)
    worker = NotesWorker(
        NoteRepository(store),
        lambda: now,
        signal=asyncio.Event(),
        store_executor=executor,
        client=model,
    )
    try:
        worker.repository.initialize()
        worker.repository.request(owner, recording, now, True)
        job = await worker._run(worker.repository.claim, now)
        assert job is not None, "the note was not due"
        await worker.generate(job)
    finally:
        executor.shutdown()
    return worker
