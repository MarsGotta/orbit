"""SIM-40: the hub's workers stop when asked, even if a call swallowed the request.

Found while testing SIM-40: the hub, closed while a meeting group's STT answer
was arriving, never finished closing (3 of 6 runs of one test). The call
returned normally under the cancel (`stt ok` with `cancelling() == 2`), and the
worker then waited on its 5 s tick for ever, each timeout turned into the
`TimeoutError` it ignores. These fakes swallow the first cancel on purpose, so
the mechanism is exercised every time rather than by a race; a later cancel
(the hang guard's) goes through, so a worker that ignores the first fails here
instead of hanging the run.
"""

import asyncio
from concurrent.futures import ThreadPoolExecutor

from conftest import device
from support_notes import FakeModel
from support_waits import wait_for
from test_meeting_views import TEXTS, meeting

from orbit_hub.meeting_transcription import Transcription
from orbit_hub.notes import NoteRepository, NotesWorker
from orbit_hub.transcription import TranscriptionWorker


class SwallowingTranscriber:
    """A transcriber that finishes its answer under the first cancel, as httpcore can."""

    def __init__(self):
        self.started = asyncio.Event()
        self.swallowed = False

    async def transcribe(self, audio, language=None):
        self.started.set()
        try:
            await asyncio.sleep(3600)
        except asyncio.CancelledError:
            if self.swallowed:
                raise
            self.swallowed = True
        return Transcription("respuesta que llegó al cerrar", "es")


async def test_the_transcription_worker_stops_even_if_its_stt_call_swallowed_the_cancel(
    environment,
):
    _settings, store, adapter = environment
    owner = device(store)["device_id"]
    meeting(store, owner, texts=TEXTS, finalize=False)
    transcriber = SwallowingTranscriber()
    worker = TranscriptionWorker(store, adapter, lambda: 1_800_000_000.0, meetings=transcriber)
    worker.start()
    await wait_for(transcriber.started.wait(), "la primera petición al STT")
    await wait_for(worker.close(), "el cierre del trabajador de transcripción")


class SwallowingModel(FakeModel):
    """A processing client that finishes its answer under the first cancel."""

    async def complete_json(self, **request):
        self.started.set()
        try:
            await asyncio.sleep(3600)
        except asyncio.CancelledError:
            if self.swallowed:
                raise
            self.swallowed = True
        return await super().complete_json(**request)


async def test_the_notes_worker_stops_even_if_its_model_call_swallowed_the_cancel(environment):
    _settings, store, _adapter = environment
    owner = device(store)["device_id"]
    recording = meeting(store, owner, texts=TEXTS)
    model = SwallowingModel()
    model.started, model.swallowed = asyncio.Event(), False
    executor = ThreadPoolExecutor(max_workers=1)
    worker = NotesWorker(
        NoteRepository(store),
        model.configuration,
        lambda: 1_800_000_000.0,
        signal=asyncio.Event(),
        store_executor=executor,
        client=model,
    )
    # `close()` swallows its own cancel, so a hang guard on it cannot fail:
    # what shows a worker that kept going is that it looked for work again.
    claimed_after_stop = []
    claim = worker.repository.claim

    def watched(now):
        if model.swallowed:
            claimed_after_stop.append(now)
        return claim(now)

    worker.repository.claim = watched
    try:
        worker.start()
        worker.repository.request(owner, recording, 1_800_000_000.0, True)
        worker.signal.set()
        await wait_for(model.started.wait(), "la petición al modelo")
        await wait_for(worker.close(), "el cierre del trabajador de notas")
    finally:
        executor.shutdown()
    assert model.swallowed and claimed_after_stop == []
