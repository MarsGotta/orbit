"""Note templates of a series against the real store, with a scripted model.

`FakeModel` answers like the processing client: each purpose takes its next
scripted answer, or a valid default note. The worker that uses it is the real one; `run_once` drives it one job
at a time, so nothing here waits on a clock or a deadline. No provider, no
network and no fixed port.
"""

import asyncio
import json
from concurrent.futures import ThreadPoolExecutor
from contextlib import asynccontextmanager
from uuid import uuid4

import httpx
from support_grouping import finalize, put
from support_notes import FakeModel

from orbit_hub.app import create_app
from orbit_hub.notes import NoteRepository, NotesWorker

SERIES = "5E6F7A8B-0001-4C2D-9E0F-123456789ABC"
HOUR_MS = 3_600_000


def recording(store, owner, *, text="Hablamos del presupuesto.") -> str:
    """A finished, fully transcribed recording of two tracks."""
    identity, epoch = str(uuid4()), str(uuid4())
    for track in ("mic_local", "remote_app"):
        for sequence in range(2):
            put(store, owner, identity, track, sequence, capture_epoch_id=epoch)
    finalize(store, owner, identity, 2)
    while job := store.claim_transcription(1000):
        store.finish_transcription(job, text)
    return identity


def with_event(store, owner: str, recording: str, event: dict) -> None:
    """The calendar event a Mac sends in the closing manifest, set in place."""
    context = f"manifest:{owner}:{recording}"
    with store.transaction() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (owner, recording),
        ).fetchone()
        manifest = json.loads(store.decrypt(row["body"], context))
        manifest["event"] = event
        db.execute(
            "UPDATE recording_manifests SET body=? WHERE device_id=? AND recording=?",
            (store.encrypt(json.dumps(manifest).encode(), context), owner, recording),
        )


class Bench:
    """The real notes worker on the real store, driven one job at a time."""

    def __init__(self, store, owner, now: float):
        self.store, self.owner = store, owner
        self.clock = {"now": now}
        self.model = FakeModel()
        self.executor = ThreadPoolExecutor(max_workers=1)
        self.worker = NotesWorker(
            NoteRepository(store),
            lambda: self.clock["now"],
            signal=asyncio.Event(),
            store_executor=self.executor,
            client=self.model,
        )
        self.worker.repository.initialize()

    def close(self) -> None:
        self.executor.shutdown()

    def request(self, recording_id: str, *, template_id=None, event=None) -> dict:
        """Ask for the note as the hub does; `event` is the one its manifest carries."""
        if event is not None:
            with_event(self.store, self.owner, recording_id, event)
        return self.worker.repository.request(
            self.owner, recording_id, self.clock["now"], True, template_id=template_id
        )

    async def drain(self, limit: int = 20) -> int:
        """Run every job that is due now, one at a time; return how many ran."""
        ran = 0
        while ran < limit and await self.worker.run_once():
            ran += 1
        return ran

    def event(self, starts_at_ms: int, title: str = "Comité semanal", series: str = SERIES):
        return {"title": title, "starts_at_ms": starts_at_ms, "calendar_item_id": series}


@asynccontextmanager
async def hub(environment, model=None, now=None):
    """The real hub app in process, its own worker running, and a paired device."""
    from conftest import device

    settings, store, adapter = environment
    model = model or FakeModel()
    kwargs = {"now": now} if now is not None else {}
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, processing_client=model, **kwargs
    )
    owner = device(store)
    headers = {"Authorization": "Bearer " + owner["token"]}
    async with app.router.lifespan_context(app):
        transport = httpx.ASGITransport(app=app)
        async with httpx.AsyncClient(transport=transport, base_url="https://localhost") as client:
            yield client, headers, owner, app, model
