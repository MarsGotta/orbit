"""Single-inference worker; interactive utterances precede durable meeting jobs.

A supervisor wraps the worker loop so an uncaught provider failure restarts the
task with a bounded backoff instead of leaving interactive voice unavailable
until the process restarts. While the supervisor is between attempts, new
``transcribe`` calls refuse with ``transcription_unavailable``.
"""

import asyncio
from concurrent.futures import Executor
from dataclasses import dataclass

from .log import report, stop_if_asked, supervised
from .protocol import HubError
from .transcription_failure import durable_code

PREPARE_TICK_SECONDS = 5.0
# Protects one interactive request while it waits in the bounded queue and
# then uses Hermes's 180 s provider call. The extra 60 s is queue allowance,
# and the daemon's 250 s outer request budget expires after this refusal can
# be returned. It is not the provider timeout or permission to retry audio.
INTERACTIVE_TRANSCRIPTION_TIMEOUT_SECONDS = 240


@dataclass
class Utterance:
    device: str
    body: bytes
    result: asyncio.Future


class TranscriptionWorker:
    def __init__(
        self,
        store,
        speech,
        now,
        *,
        meetings,
        prepare_signal: asyncio.Event | None = None,
        store_executor: Executor | None = None,
        group_finished: asyncio.Event | None = None,
        recording_changed=None,
    ):
        if meetings is None:
            raise ValueError("meeting_transcription requires a meeting transcriber")
        self.store, self.speech, self.now = store, speech, now
        self.meetings = meetings
        self.interactive: asyncio.Queue[Utterance] = asyncio.Queue(maxsize=8)
        self.prepare_signal = prepare_signal if prepare_signal is not None else asyncio.Event()
        self.store_executor = store_executor
        self.group_finished = group_finished
        # Tells the owning device's connections that a recording changed.
        self.recording_changed = recording_changed
        self._task = None
        self._supervisor = None

    def start(self):
        self._supervisor = asyncio.create_task(supervised("transcription", self._loop))

    async def _loop(self):
        # Each start first gives back the groups a stopped run left running:
        # its outcome could not be written, and nothing else would retry them.
        # While the disk is still full this fails too, and the supervisor
        # tries again after its backoff.
        await self._run_store(self.store.groups.requeue_running, self.now())
        self._task = asyncio.create_task(self._run())
        try:
            await self._task
        finally:
            # The supervisor catches exceptions from `_run`; cancellation here is
            # the deliberate close path and must propagate up.
            self._task = None

    async def close(self):
        targets = [task for task in (self._supervisor, self._task) if task is not None]
        for task in targets:
            task.cancel()
        if targets:
            await asyncio.gather(*targets, return_exceptions=True)
        self._supervisor = self._task = None
        while not self.interactive.empty():
            request = self.interactive.get_nowait()
            if not request.result.done():
                request.result.set_exception(HubError("shutting_down", "Server shutting down"))

    async def transcribe(self, device: str, body: bytes) -> str:
        if self._task is not None and self._task.done():
            raise HubError(
                "transcription_unavailable",
                "Transcription worker stopped; keep the audio",
                http_status=503,
            )
        future = asyncio.get_running_loop().create_future()
        try:
            self.interactive.put_nowait(Utterance(device, body, future))
            self.prepare_signal.set()
        except asyncio.QueueFull as exc:
            raise HubError("rate_limited", "Transcription queue full; keep the audio") from exc
        try:
            return await asyncio.wait_for(future, INTERACTIVE_TRANSCRIPTION_TIMEOUT_SECONDS)
        except TimeoutError as exc:
            timeout_ms = INTERACTIVE_TRANSCRIPTION_TIMEOUT_SECONDS * 1000
            raise HubError(
                "transcription_timeout",
                f"Transcription took longer than {timeout_ms} ms",
                http_status=504,
                numbers={"timeout_ms": timeout_ms},
            ) from exc

    async def transcribe_meeting(
        self, audio: bytes, language: str | None = None, prompt: str = "", terms=(), model=None
    ):
        """Send durable meeting audio only through its dedicated transcriber.

        A configured meeting transcriber that fails is NOT retried through the
        dictation path. Falling back would produce a transcript polluted by the
        dictation prompt while reporting success, which is the exact failure
        this split exists to end. A failure fails the job, and the original
        audio stays durable so it can be retried once the endpoint is back.
        """
        return await self.meetings.transcribe(
            audio,
            language,
            **({"prompt": prompt} if prompt else {}),
            **(
                {"terms": terms, "model": model}
                if getattr(self.meetings, "accepts_terms", False)
                else {}
            ),
        )

    async def _run(self):
        while True:
            stop_if_asked()
            if not self.interactive.empty():
                request = self.interactive.get_nowait()
                if request.result.done():
                    continue
                try:
                    if not self.store.devices.is_active(request.device):
                        raise HubError("revoked", "Device revoked")
                    text = (
                        await self.meetings.interactive(self.speech, request.body)
                        if hasattr(self.meetings, "interactive")
                        else await self.speech.transcribe(request.body)
                    )
                    if not request.result.done():
                        request.result.set_result(text)
                except HubError as exc:
                    if not request.result.done():
                        request.result.set_exception(exc)
                except Exception:  # noqa: BLE001 - redact arbitrary provider failure at worker boundary
                    report(
                        event="transcription_failed",
                        code="transcription_failed",
                        device=request.device,
                    )
                    if not request.result.done():
                        request.result.set_exception(
                            HubError("transcription_failed", "The transcription adapter failed")
                        )
                continue
            try:
                job = await self._claim()
            except HubError:
                # Quota admission can temporarily prevent sealing a group. Keep
                # original chunks durable and interactive voice service available.
                await asyncio.sleep(1)
                continue
            if job is None:
                await self._wait_for_work()
                continue
            try:
                result = await self.transcribe_meeting(
                    job["audio"],
                    job.get("language"),
                    job.get("prompt", ""),
                    job.get("terms", ()),
                    job.get("model"),
                )
                self.store.groups.finish(job, result.text, result.language)
                await self._meeting_finished(job)
            except HubError as exc:
                self.store.groups.fail(job, durable_code(exc), self.now())
                await self._meeting_finished(job)
            except Exception:  # noqa: BLE001 - preserve the durable job with a redacted failure code
                report(
                    event="meeting_transcription_failed",
                    code="transcription_failed",
                    device=job["device_id"],
                )
                self.store.groups.fail(job, "transcription_failed", self.now())
                await self._meeting_finished(job)

    async def _meeting_finished(self, job: dict) -> None:
        # A note waiting for the last group of its recording is written at once
        # instead of on the notes worker's next tick.
        if self.group_finished is not None:
            self.group_finished.set()
        if self.recording_changed is not None:
            self.recording_changed(job["device_id"], job["recording"])

    async def _run_store(self, function, *arguments):
        if self.store_executor is None:
            return function(*arguments)
        return await asyncio.get_running_loop().run_in_executor(
            self.store_executor, function, *arguments
        )

    async def _claim(self) -> dict | None:
        """Run the group claim (prepare + claim + decrypt/assemble) off the loop.

        The heavy reads decrypt up to 20 MiB of audio; doing them on the loop
        blocked every other client while a meeting job was being sealed.
        """

        def claim():
            from .meetings.dictionaries import prompt, terms_for

            job = self.store.groups.claim(self.now())
            if job is not None:
                if hasattr(self.meetings, "model") and job.get("model") is None:
                    job["model"] = self.meetings.model
                    with self.store.transaction() as db:
                        db.execute(
                            "UPDATE transcription_groups SET model=? WHERE device_id=? AND id=?",
                            (job["model"], job["device_id"], job["group_id"]),
                        )
                terms = terms_for(self.store, job["device_id"], job["recording"])
                job["prompt"] = prompt(terms)
                job["terms"] = [term["correct"] for term in terms]
            return job

        loop = asyncio.get_running_loop()
        if self.store_executor is not None:
            return await loop.run_in_executor(self.store_executor, claim)
        return claim()

    async def _wait_for_work(self) -> None:
        """Wait for an external signal (chunk persisted or group sealed) with a safety tick."""
        try:
            await asyncio.wait_for(self.prepare_signal.wait(), timeout=PREPARE_TICK_SECONDS)
        except TimeoutError:
            pass
        self.prepare_signal.clear()
