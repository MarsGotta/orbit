"""Durable final transcription: publish both tracks and voices in one transaction."""

import asyncio
import json
import time
import wave
from dataclasses import dataclass, field

from . import meeting_transcript_integrity as integrity
from .final_pass_audio import encode_window, memory_file, prepare_track, silence_report
from .group_storage import GroupRepository
from .log import report, supervised
from .meeting_diarization import separate
from .meeting_transcription import REQUEST_TIMEOUT_SECONDS
from .meeting_voice_identity import publish_voices, speaker_bound
from .meetings import muted, open_recordings, repository, turns
from .protocol import HubError

# Second real measurement (October 2026): at most 45 s, selecting the longest pause
# after 15 s, recovered 98% of the failed recording (whole track: 78–79%
# in eight runs), and 98% on the good ones in half the time. These bound
# Whisper's context, not meeting duration or upload size; windows + VAD lost
# no draft groups on nine real tracks from six meetings.
WINDOW_SECONDS = 45
MIN_WINDOW_SECONDS = 15
# Worst CPU separation measured in October 2026: 382 s for 7.4 min (444 s),
# on a loaded host.
# This is a measured worst case, not a p99 of the new complete pipeline.
CPU_SECONDS_PER_AUDIO_SECOND = 382 / 444
# Allow boundary jitter up to 250 ms, not arbitrary words in silence. Measured
# in October 2026, this retains all 1339 synthetic words (WER 2.2% unchanged), while removing
# 70 of 79 real-meeting artifacts, over 1 s from speech, median duration 0 ms.
MAX_WORD_VOICE_GAP_MS = 250
TRACKS = ("mic_local", "remote_app")


def status(db, device, recording):
    """The pass of a recording, or None when none was registered.

    A running pass carries `progress` once it has measured its audio:
    `done_ms` of `total_ms` of audio transcribed, and `remaining_ms` once a
    window is done (the time taken so far, scaled to the audio left). The
    queue gives no fraction or estimate: its wait depends on other passes.
    A completed pass that kept draft spans carries `kept_draft`: how many,
    and their summed length (`meeting_transcript_integrity.choose`).
    """
    row = db.execute(
        "SELECT state,error,progress,kept_draft,model FROM recording_final_passes "
        "WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return None
    found = {"state": row["state"], "error": json.loads(row["error"]) if row["error"] else None}
    if row["model"] is not None:
        found["model"] = (
            json.loads(row["model"])["model"] if row["model"].startswith("{") else row["model"]
        )
    if row["state"] == "running" and row["progress"]:
        found["progress"] = json.loads(row["progress"])
    if row["state"] == "completed" and row["kept_draft"]:
        found["kept_draft"] = json.loads(row["kept_draft"])
    return found


def note_ready(store, db, device, recording):
    final = status(db, device, recording)
    if final is None:
        return GroupRepository(store).capture(device, recording, db=db)["transcript_ready"]
    if final["state"] == "completed":
        return True
    if final["state"] != "failed":
        return False
    # A failed final pass uses the complete draft, including terminal failures.
    pending = db.execute(
        "SELECT 1 FROM transcription_jobs WHERE device_id=? AND chunk_id IN "
        "(SELECT id FROM chunks WHERE device_id=? AND recording=?) "
        "AND state IN ('pending','running') LIMIT 1",
        (device, device, recording),
    ).fetchone()
    active = db.execute(
        "SELECT 1 FROM transcription_groups WHERE device_id=? AND recording=? "
        "AND state IN ('pending','running') LIMIT 1",
        (device, recording),
    ).fetchone()
    return pending is None and active is None


def planned_lengths(store, db, device, recording):
    """Each track's expected audio, from the closing manifest, before it is read.

    A track lasts at most the recording (`prepare_track` keeps wall-clock
    gaps from its start); its real length replaces this once it is read.
    """
    row = db.execute(
        "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return {}
    manifest = repository._open(store, row["body"], f"manifest:{device}:{recording}")
    span = max(0, manifest["finalized_at_ms"] - manifest["started_at_ms"])
    declared = {item["track"] for item in manifest["tracks"]}
    return {track: span for track in TRACKS if track in declared}


class FinalPassRepository:
    def __init__(self, store):
        self.store = store

    def request(self, device, recording, *, configured=True, model=None):
        with self.store.transaction() as db:
            repository.require(db, device, recording)
            closed = db.execute(
                "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if closed is None:
                raise HubError(
                    "recording_still_open",
                    "The recording is still open",
                )
            total = db.execute(
                "SELECT count(*) FROM transcription_groups WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()[0]
            unreadable = db.execute(
                "SELECT count(*) FROM transcription_groups WHERE device_id=? AND recording=? "
                "AND state='failed' AND error_code='invalid_group_audio'",
                (device, recording),
            ).fetchone()[0]
            if unreadable:
                raise HubError(
                    "meeting_audio_unreadable",
                    f"The hub cannot read the audio for {unreadable} of {total} groups",
                    http_status=409,
                    numbers={"groups": unreadable, "total": total},
                )
            if not configured:
                raise HubError(
                    "diarization_not_configured",
                    "Voice separation is unavailable on the hub",
                    http_status=409,
                )
            return (
                db.execute(
                    "INSERT INTO recording_final_passes(device_id,recording,state,refresh_voices,model) "
                    "VALUES (?,?,'queued',1,?) ON CONFLICT(device_id,recording) DO UPDATE SET "
                    "state='queued',error=NULL,refresh_voices=1,model=excluded.model "
                    "WHERE state IN ('failed','completed')",
                    (device, recording, model),
                ).rowcount
                > 0
            )

    def claim(self, default_model=None):
        with self.store.transaction() as db:
            row = db.execute(
                "SELECT f.device_id,f.recording,f.refresh_voices,f.model FROM recording_final_passes f "
                "WHERE f.state='queued' ORDER BY f.rowid LIMIT 1"
            ).fetchone()
            if row is None:
                return None
            lengths = planned_lengths(self.store, db, row["device_id"], row["recording"])
            db.execute(
                "UPDATE recording_final_passes SET state='running',progress=?,model=? "
                "WHERE device_id=? AND recording=?",
                (
                    json.dumps({"done_ms": 0, "total_ms": sum(lengths.values())}),
                    row["model"] or default_model,
                    row["device_id"],
                    row["recording"],
                ),
            )
            chosen = repository.language(db, row["device_id"], row["recording"])
            return {
                **dict(row),
                "model": row["model"] or default_model,
                "language": chosen[0] if chosen else None,
                "lengths": lengths,
                "muted": muted.read(self.store, db, row["device_id"], row["recording"]),
                "max_speakers": speaker_bound(self.store, db, row["device_id"], row["recording"]),
            }

    def advance(self, job, progress):
        with self.store.transaction() as db:
            db.execute(
                "UPDATE recording_final_passes SET progress=? "
                "WHERE device_id=? AND recording=? AND state='running'",
                (json.dumps(progress), job["device_id"], job["recording"]),
            )

    def complete(self, job, result):
        device, recording = job["device_id"], job["recording"]
        with self.store.transaction() as db:
            running = db.execute(
                "SELECT 1 FROM recording_final_passes WHERE device_id=? AND recording=? "
                "AND state='running'",
                (device, recording),
            ).fetchone()
            if running is None:
                return  # The recording was deleted meanwhile: nothing comes back.
            chosen = repository.language(db, device, recording)
            if chosen is not None and chosen[0] != job["language"]:
                return  # A language change already queued its replacement.
            publish_voices(self.store, db, job, result)
            # Absent when nothing of the draft was kept, as from every hub before.
            kept = result.get("kept_draft") or {}
            db.execute(
                "UPDATE recording_final_passes SET state='completed',error=NULL,progress=NULL,"
                "kept_draft=?,refresh_voices=0 WHERE device_id=? AND recording=?",
                (json.dumps(kept) if kept.get("spans") else None, device, recording),
            )

    def voices(self, job):
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT body FROM recording_voices WHERE device_id=? AND recording=?",
                (job["device_id"], job["recording"]),
            ).fetchone()
            return (
                None
                if row is None
                else repository._open(
                    self.store,
                    row["body"],
                    f"recording-voices:{job['device_id']}:{job['recording']}",
                )
            )

    def fail(self, job, error):
        with self.store.transaction() as db:
            db.execute(
                "UPDATE recording_final_passes SET state='failed',error=?,progress=NULL,"
                "kept_draft=NULL WHERE device_id=? AND recording=? AND state='running'",
                (json.dumps(error), job["device_id"], job["recording"]),
            )

    def draft(self, job, track):
        """The track's draft groups with text, `(start_ms, end_ms, text)` in track time."""
        device, recording = job["device_id"], job["recording"]
        with self.store.read_snapshot() as db:
            began = turns.started_at(self.store, db, device, recording)
            rows = db.execute(
                "SELECT g.id,g.manifest,t.body FROM transcription_groups g "
                "JOIN group_transcripts t ON t.device_id=g.device_id AND t.group_id=g.id "
                "WHERE g.device_id=? AND g.recording=? AND g.track=? ORDER BY g.first_sequence",
                (device, recording, track),
            ).fetchall()
            groups = []
            for row in rows:
                group = repository._open(self.store, row["manifest"], f"group:{device}:{row['id']}")
                body = repository._open(
                    self.store, row["body"], f"group-transcript:{device}:{row['id']}"
                )
                wall = group.get("started_at_ms")
                at = max(0, wall - began) if type(wall) is int and type(began) is int else 0
                groups.append(
                    (at, at + open_recordings.length_ms(group), str(body.get("text") or ""))
                )
            # A group wholly in the microphone's muted silence lost nothing.
            return muted.spoken(job.get("muted", ()), groups) if track == "mic_local" else groups

    def chunks(self, job, track):
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT c.id,c.sha256,j.metadata FROM chunks c JOIN transcription_jobs j "
                "ON j.device_id=c.device_id AND j.chunk_id=c.id "
                "WHERE c.device_id=? AND c.recording=? AND c.track=? ORDER BY c.sequence",
                (job["device_id"], job["recording"], track),
            ).fetchall()
            for row in rows:
                body = db.execute(
                    "SELECT body FROM chunks WHERE device_id=? AND id=?",
                    (job["device_id"], row["id"]),
                ).fetchone()[0]
                yield (
                    self.store.decrypt(body, f"chunk:{job['device_id']}:{row['id']}"),
                    json.loads(
                        self.store.decrypt(
                            row["metadata"], f"metadata:{job['device_id']}:{row['id']}"
                        )
                    ),
                    row["sha256"],
                )


def window_ends(frames, diagnostics):
    """Cut each window at the midpoint of its longest silence, on exact PCM sample boundaries."""
    quiet = integrity.silences(diagnostics)
    ends, at = [], 0
    while frames - at > WINDOW_SECONDS * 16000:
        choices = [
            (end - start, int((start + end) / 2 * 16000))
            for start, end in quiet
            if at + MIN_WINDOW_SECONDS * 16000
            <= int((start + end) / 2 * 16000)
            <= at + WINDOW_SECONDS * 16000
        ]
        cut = max(choices)[1] if choices else at + WINDOW_SECONDS * 16000
        ends.append(cut)
        at = cut
    return [*ends, frames]


def attribute(words, spans, track):
    result = []
    for word in words:
        voice = track
        if track == "remote_app":
            candidates = [
                (
                    max(0, min(word["end_ms"], s["end_ms"]) - max(word["start_ms"], s["start_ms"])),
                    -max(0, s["start_ms"] - word["end_ms"], word["start_ms"] - s["end_ms"]),
                    s["voice"],
                )
                for s in spans
            ]
            if not candidates:
                continue
            _, negative_gap, voice = max(candidates)
            if -negative_gap > MAX_WORD_VOICE_GAP_MS:
                continue
        result.append({**word, "track": track, "voice": voice})
    return result


def _in_track_time(words, offset, window_ms):
    """A window's words moved to track time, refused when one falls outside it."""
    placed = []
    for word in words:
        if not 0 <= word["start_ms"] <= word["end_ms"] <= window_ms:
            raise HubError(
                "transcription_word_out_of_range",
                "Word timestamp outside its window",
                numbers={
                    "start_ms": word["start_ms"],
                    "end_ms": word["end_ms"],
                    "window_ms": window_ms,
                },
            )
        placed.append(
            {**word, "start_ms": word["start_ms"] + offset, "end_ms": word["end_ms"] + offset}
        )
    return placed


@dataclass
class _Pass:
    """One pass under way: where it is, for its failure, and what it has so far."""

    job: dict
    started: float
    lengths: dict
    done_ms: int = 0
    phase: str = "prepare"
    budget: float = 0
    existing: dict | None = None
    result: dict = field(default_factory=dict)
    vocabulary: str | None = None


class FinalPassWorker:
    def __init__(
        self, store, transcriber, configuration, signal, changed, executor, clock=time.monotonic
    ):
        self.repository = FinalPassRepository(store)
        self.transcriber, self.configuration = transcriber, configuration
        self.signal, self.changed, self.executor = signal, changed, executor
        # Seconds, for the time a running pass has left; never decides anything.
        self.clock = clock
        self.task = None

    async def report_progress(self, job, started, done_ms, lengths):
        total_ms = sum(lengths.values())
        progress = {"done_ms": done_ms, "total_ms": total_ms}
        if done_ms:
            taken_ms = (self.clock() - started) * 1000
            progress["remaining_ms"] = round(taken_ms * max(0, total_ms - done_ms) / done_ms)
        await self.run(self.repository.advance, job, progress)
        self.changed(job["device_id"], job["recording"])

    async def run(self, function, *args):
        future = asyncio.get_running_loop().run_in_executor(self.executor, function, *args)
        try:
            return await asyncio.shield(future)
        except asyncio.CancelledError:
            # memfd ownership cannot end while an encoder still uses it.
            await future
            raise

    def start(self):
        self.task = asyncio.create_task(supervised("final_pass", self.loop))

    async def close(self):
        if self.task is not None:
            self.task.cancel()
            await asyncio.gather(self.task, return_exceptions=True)

    async def loop(self):
        while True:
            if not await self.run_once():
                # Safety tick only; no deadline decides transcript readiness.
                await asyncio.sleep(1)

    async def run_once(self):
        job = await self.run(self.repository.claim, getattr(self.transcriber, "model", None))
        if job is None:
            return False
        run = _Pass(job, self.clock(), dict(job["lengths"]))
        self.changed(job["device_id"], job["recording"])
        job["language"] = job["language"] or self.transcriber.configuration.language
        try:
            await self._transcribe(run)
        except TimeoutError:
            await self.run(
                self.repository.fail,
                job,
                {
                    "code": "final_pass_timeout",
                    "phase": run.phase,
                    "timeout_ms": round(run.budget * 1000),
                },
            )
        except (wave.Error, EOFError, ValueError, KeyError):
            # Values are never quoted: the phase identifies the corrupt track.
            await self.run(
                self.repository.fail, job, {"code": "final_audio_invalid", "phase": run.phase}
            )
        except HubError as exc:
            await self.run(
                self.repository.fail,
                job,
                {"code": exc.code, "phase": run.phase, **(exc.numbers or {})},
            )
        except Exception:  # noqa: BLE001 - no audio, names, paths or provider diagnostics in evidence
            report(event="final_pass_failed", code="final_pass_internal_error")
            await self.run(
                self.repository.fail, job, {"code": "final_pass_internal_error", "phase": run.phase}
            )
        self.signal.set()
        self.changed(job["device_id"], job["recording"])
        return True

    async def _transcribe(self, run):
        """Both tracks, then the voices, persisted together."""
        job = run.job
        if self.configuration is None:
            raise HubError("diarization_not_configured", "Local separation configuration missing")
        run.existing = await self.run(self.repository.voices, job)
        if job.get("refresh_voices"):
            run.existing = None
        run.result = {
            "spans": [],
            "words": [],
            "audio_ms": 0,
            "language": job["language"],
            "kept_draft": {"spans": 0, "duration_ms": 0},
        }
        if run.existing is not None:
            run.result.update(run.existing)
        from .meetings.dictionaries import prompt, terms_for

        terms = await self.run(terms_for, self.repository.store, job["device_id"], job["recording"])
        run.vocabulary = prompt(terms)
        job["terms"] = [term["correct"] for term in terms]
        for track in TRACKS:
            run.phase = f"audio_{track}"
            with memory_file() as audio:
                if not await self.run(prepare_track, self.repository, job, track, audio):
                    run.lengths.pop(track, None)
                    continue
                with wave.open(audio, "rb") as wav:
                    await self._track(run, track, audio, wav)
        run.phase = "persist"
        if not run.result["audio_ms"]:
            raise HubError(
                "final_audio_empty",
                "The recording has no readable audio",
                numbers={"audio_ms": 0},
            )
        await self.run(self.repository.complete, job, run.result)

    async def _track(self, run, track, audio, wav):
        """One prepared track: its windows through the selected transcriber, the draft it keeps,
        and the voices of the remote one."""
        result = run.result
        frames = wav.getnframes()
        duration_ms = frames * 1000 // 16000
        run.lengths[track] = duration_ms
        if not frames:
            return
        run.phase = f"windows_{track}"
        ends = window_ends(frames, await self.run(silence_report, audio.fileno()))
        # CPU worst observed, encode/request budgets for the
        # ACTUAL number of windows, and lock/model startup.
        # Diagnostic ffmpeg has its own existing 300 s budget.
        run.budget = (
            duration_ms / 1000 * CPU_SECONDS_PER_AUDIO_SECOND
            + (2 * len(ends) + 1) * REQUEST_TIMEOUT_SECONDS
        )
        async with asyncio.timeout(run.budget):
            run.phase = f"{run.job.get('model') or 'whisper'}_{track}"
            words = await self._words(run, wav, ends)
            groups = await self.run(self.repository.draft, run.job, track)
            # Where the final lost a draft span or loops over
            # it, that span keeps its draft; voices then apply
            # to whichever words remain, by their times.
            words, kept = integrity.choose(groups, words)
            for key in ("spans", "duration_ms"):
                result["kept_draft"][key] += kept[key]
            if track == "remote_app" and run.existing is None:
                run.phase = "diarization"
                bound = run.job.get("max_speakers")
                result.update(
                    await separate(
                        self.configuration,
                        audio.fileno(),
                        duration_ms,
                        **({"max_speakers": bound} if bound is not None else {}),
                    )
                )
            result["words"].extend(attribute(words, result["spans"], track))
            result["audio_ms"] = max(result["audio_ms"], duration_ms)

    async def _words(self, run, wav, ends):
        """The track's words, window by window, in track time."""
        words, offset = [], 0
        for end in ends:
            pcm = wav.readframes(end - wav.tell())
            window_ms = len(pcm) * 1000 // 32000
            encoded = await self.run(encode_window, pcm)
            transcriber = (
                self.transcriber.for_model(run.job["model"])
                if hasattr(self.transcriber, "for_model")
                else self.transcriber
            )
            from .openrouter_transcription import OpenRouterTranscriber

            text = await transcriber.transcribe(
                encoded,
                run.job["language"],
                final=True,
                **({"prompt": run.vocabulary} if run.vocabulary else {}),
                **(
                    {"terms": run.job["terms"]}
                    if isinstance(transcriber, OpenRouterTranscriber)
                    else {}
                ),
            )
            run.result["language"] = text.language or run.result["language"]
            words.extend(_in_track_time(text.words, offset, window_ms))
            offset = end * 1000 // 16000
            run.done_ms += window_ms
            await self.report_progress(run.job, run.started, run.done_ms, run.lengths)
        return words
