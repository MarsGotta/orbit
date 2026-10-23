"""Durable final transcription: publish both tracks and voices in one transaction."""

import asyncio
import hashlib
import io
import json
import os
import subprocess
import time
import wave
from contextlib import contextmanager

from . import meeting_transcript_integrity as integrity
from .group_storage import GroupRepository
from .log import report, supervised
from .meeting_diarization import separate
from .meeting_transcription import MAX_AUDIO_BYTES, REQUEST_TIMEOUT_SECONDS
from .meetings import muted, open_recordings, repository, turns
from .protocol import HubError

# Second real measurement (October 2026): at most 45 s, selecting the longest pause
# after 15 s, recovered 98% of the failed recording (whole track: 78–79%
# in eight runs), and 98% on the good ones in half the time. These bound
# Whisper's context, not meeting duration or upload size; windows + VAD lost
# no draft groups on nine real tracks from six meetings.
WINDOW_SECONDS = 45
MIN_WINDOW_SECONDS = 15
OPUS_BITRATE = 24_000
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
        "SELECT state,error,progress,kept_draft FROM recording_final_passes "
        "WHERE device_id=? AND recording=?",
        (device, recording),
    ).fetchone()
    if row is None:
        return None
    found = {"state": row["state"], "error": json.loads(row["error"]) if row["error"] else None}
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

    def request(self, device, recording, *, configured=True):
        with self.store.transaction() as db:
            repository.require(db, device, recording)
            closed = db.execute(
                "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if closed is None:
                raise HubError(
                    "recording_still_open",
                    "La grabación todavía está abierta",
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
                    f"El hub no puede leer el audio de {unreadable} de {total} grupos",
                    http_status=409,
                    numbers={"groups": unreadable, "total": total},
                )
            if not configured:
                raise HubError(
                    "diarization_not_configured",
                    "La separación de voces no está disponible en el hub",
                    http_status=409,
                )
            return (
                db.execute(
                    "INSERT INTO recording_final_passes(device_id,recording,state) "
                    "VALUES (?,?,'queued') ON CONFLICT(device_id,recording) DO UPDATE SET "
                    "state='queued',error=NULL WHERE state IN ('failed','completed')",
                    (device, recording),
                ).rowcount
                > 0
            )

    def claim(self):
        with self.store.transaction() as db:
            row = db.execute(
                "SELECT f.device_id,f.recording FROM recording_final_passes f "
                "JOIN devices d ON d.id=f.device_id WHERE f.state='queued' "
                "AND d.revoked=0 ORDER BY f.rowid LIMIT 1"
            ).fetchone()
            if row is None:
                return None
            lengths = planned_lengths(self.store, db, row["device_id"], row["recording"])
            db.execute(
                "UPDATE recording_final_passes SET state='running',progress=? "
                "WHERE device_id=? AND recording=?",
                (
                    json.dumps({"done_ms": 0, "total_ms": sum(lengths.values())}),
                    *tuple(row),
                ),
            )
            chosen = repository.language(db, row["device_id"], row["recording"])
            return {
                **dict(row),
                "language": chosen[0] if chosen else None,
                "lengths": lengths,
                "muted": muted.read(self.store, db, row["device_id"], row["recording"]),
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
        voices = {"spans": result["spans"]}
        text = {key: result[key] for key in ("words", "audio_ms", "language")}
        # What a model heard in the microphone's muted silence is not speech.
        text["words"] = muted.heard(job.get("muted", ()), text["words"])
        sealed = repository._seal(self.store, voices, f"recording-voices:{device}:{recording}")
        transcript = repository._seal(
            self.store, text, f"recording-final-transcript:{device}:{recording}"
        )
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
            self.store.ensure_capacity(len(sealed) + len(transcript))
            # Voice identity survives retranscription in another language.
            db.execute(
                "INSERT OR IGNORE INTO recording_voices VALUES (?,?,?)", (device, recording, sealed)
            )
            db.execute(
                "INSERT INTO recording_final_transcripts VALUES (?,?,?) "
                "ON CONFLICT(device_id,recording) DO UPDATE SET body=excluded.body",
                (device, recording, transcript),
            )
            # Absent when nothing of the draft was kept, as from every hub before.
            kept = result.get("kept_draft") or {}
            db.execute(
                "UPDATE recording_final_passes SET state='completed',error=NULL,progress=NULL,"
                "kept_draft=? WHERE device_id=? AND recording=?",
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


@contextmanager
def memory_file():
    # Linux hub only: memfd has no directory entry or plaintext fallback.
    descriptor = os.memfd_create("orbit-final-audio", os.MFD_CLOEXEC)
    with os.fdopen(descriptor, "w+b") as handle:
        yield handle


def ffmpeg(arguments, descriptors, capture_stderr=False):
    try:
        completed = subprocess.run(
            ["ffmpeg", "-nostdin", "-v", "error", "-y", *arguments],
            pass_fds=descriptors,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE if capture_stderr else subprocess.DEVNULL,
            check=True,
            timeout=REQUEST_TIMEOUT_SECONDS,
        )
        return completed.stderr
    except subprocess.TimeoutExpired as exc:
        raise HubError(
            "final_audio_timeout",
            "La conversión de audio agotó su plazo",
            numbers={"timeout_ms": REQUEST_TIMEOUT_SECONDS * 1000},
        ) from exc
    except (OSError, subprocess.CalledProcessError) as exc:
        raise HubError(
            "final_audio_conversion_failed",
            "No se pudo convertir el audio",
            numbers={
                "exit_status": getattr(exc, "returncode", 0),
                "errno": getattr(exc, "errno", 0) or 0,
            },
        ) from exc


def prepare_track(repo, job, track, target):
    """Verify original chunks and preserve each wall-clock gap in the final timeline."""
    started, expected = None, None
    with repo.store.read_snapshot() as db:
        row = db.execute(
            "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
            (job["device_id"], job["recording"]),
        ).fetchone()
        manifest = repository._open(
            repo.store, row["body"], f"manifest:{job['device_id']}:{job['recording']}"
        )
        # A closed manifest declares exactly which tracks were uploaded.
        # A remote-only recording must not try to build an empty microphone WAV.
        if not any(item["track"] == track for item in manifest["tracks"]):
            return False
        began = manifest["started_at_ms"]
    with memory_file() as source:
        with wave.open(source, "wb") as output:
            for body, metadata, checksum in repo.chunks(job, track):
                if hashlib.sha256(body).hexdigest() != checksum:
                    raise ValueError("Corrupt audio")
                with wave.open(io.BytesIO(body), "rb") as audio:
                    shape = (audio.getnchannels(), audio.getsampwidth(), audio.getframerate())
                    if shape[0] not in (1, 2) or shape[1] != 2 or not 8000 <= shape[2] <= 192000:
                        raise ValueError("Invalid PCM format")
                    if started is None:
                        started, expected = began, shape
                        output.setnchannels(shape[0])
                        output.setsampwidth(shape[1])
                        output.setframerate(shape[2])
                    if shape != expected or metadata["frames"] != audio.getnframes():
                        raise ValueError("Inconsistent audio")
                    wall = metadata.get("started_at_ms")
                    if type(wall) is int:
                        if not began <= wall <= manifest["finalized_at_ms"]:
                            raise HubError(
                                "final_audio_time_out_of_range",
                                "El inicio del fragmento queda fuera de la grabación",
                                numbers={
                                    "started_at_ms": wall,
                                    "recording_start_ms": began,
                                    "recording_end_ms": manifest["finalized_at_ms"],
                                },
                            )
                        gap = max(0, (wall - began) * shape[2] // 1000 - output.getnframes())
                        # Write gaps in second-sized pieces, never one giant allocation.
                        while gap:
                            frames = min(gap, shape[2])
                            output.writeframesraw(bytes(frames * shape[0] * 2))
                            gap -= frames
                    pcm = audio.readframes(audio.getnframes())
                    if len(pcm) != audio.getnframes() * shape[0] * 2:
                        raise ValueError("Truncated PCM")
                    output.writeframesraw(pcm)
        if started is None:
            return False
        source.seek(0)
        ffmpeg(
            [
                "-i",
                f"/proc/self/fd/{source.fileno()}",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                f"/proc/self/fd/{target.fileno()}",
            ],
            (source.fileno(), target.fileno()),
        )
    target.seek(0)
    return True


def encode_window(pcm):
    with memory_file() as source, memory_file() as target:
        source.write(pcm)
        source.seek(0)
        ffmpeg(
            [
                "-f",
                "s16le",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-i",
                f"/proc/self/fd/{source.fileno()}",
                "-c:a",
                "libopus",
                "-b:a",
                str(OPUS_BITRATE),
                "-vbr",
                "off",
                "-f",
                "ogg",
                f"/proc/self/fd/{target.fileno()}",
            ],
            (source.fileno(), target.fileno()),
        )
        target.seek(0)
        audio = target.read(MAX_AUDIO_BYTES + 1)
        if len(audio) > MAX_AUDIO_BYTES:
            raise HubError(
                "final_window_too_large",
                "La ventana Opus supera el límite de Whisper",
                numbers={"bytes": len(audio), "max_bytes": MAX_AUDIO_BYTES},
            )
        return audio


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
        job = await self.run(self.repository.claim)
        if job is None:
            return False
        started = self.clock()
        lengths, done_ms = dict(job["lengths"]), 0
        self.changed(job["device_id"], job["recording"])
        job["language"] = job["language"] or self.transcriber.configuration.language
        phase = "prepare"
        budget = 0
        try:
            if self.configuration is None:
                raise HubError(
                    "diarization_not_configured", "Falta la configuración local de separación"
                )
            existing = await self.run(self.repository.voices, job)
            result = {
                "spans": [],
                "words": [],
                "audio_ms": 0,
                "language": job["language"],
                "kept_draft": {"spans": 0, "duration_ms": 0},
            }
            if existing is not None:
                result.update(existing)
            from .meetings.dictionaries import prompt, terms_for

            vocabulary = prompt(
                await self.run(terms_for, self.repository.store, job["device_id"], job["recording"])
            )
            for track in TRACKS:
                phase = f"audio_{track}"
                with memory_file() as audio:
                    if not await self.run(prepare_track, self.repository, job, track, audio):
                        lengths.pop(track, None)
                        continue
                    with wave.open(audio, "rb") as wav:
                        frames = wav.getnframes()
                        duration_ms = frames * 1000 // 16000
                        lengths[track] = duration_ms
                        if not frames:
                            continue
                        phase = f"windows_{track}"
                        diagnostics = await self.run(
                            ffmpeg,
                            [
                                "-v",
                                "info",
                                "-i",
                                f"/proc/self/fd/{audio.fileno()}",
                                "-af",
                                (
                                    f"silencedetect=noise={integrity.SILENCE_NOISE_DB}dB:"
                                    f"d={integrity.SILENCE_DURATION_MS / 1000}"
                                ),
                                "-f",
                                "null",
                                "-",
                            ],
                            (audio.fileno(),),
                            True,
                        )
                        ends = window_ends(frames, diagnostics)
                        # CPU worst observed, encode/request budgets for the
                        # ACTUAL number of windows, and lock/model startup.
                        # Diagnostic ffmpeg has its own existing 300 s budget.
                        budget = (
                            duration_ms / 1000 * CPU_SECONDS_PER_AUDIO_SECOND
                            + (2 * len(ends) + 1) * REQUEST_TIMEOUT_SECONDS
                        )
                        async with asyncio.timeout(budget):
                            words, offset = [], 0
                            phase = f"whisper_{track}"
                            for end in ends:
                                pcm = wav.readframes(end - wav.tell())
                                window_ms = len(pcm) * 1000 // 32000
                                encoded = await self.run(encode_window, pcm)
                                text = await self.transcriber.transcribe(
                                    encoded,
                                    job["language"],
                                    final=True,
                                    **({"prompt": vocabulary} if vocabulary else {}),
                                )
                                result["language"] = text.language or result["language"]
                                for word in text.words:
                                    if not 0 <= word["start_ms"] <= word["end_ms"] <= window_ms:
                                        raise HubError(
                                            "transcription_word_out_of_range",
                                            "Tiempo de palabra fuera de su ventana",
                                            numbers={
                                                "start_ms": word["start_ms"],
                                                "end_ms": word["end_ms"],
                                                "window_ms": window_ms,
                                            },
                                        )
                                    words.append(
                                        {
                                            **word,
                                            "start_ms": word["start_ms"] + offset,
                                            "end_ms": word["end_ms"] + offset,
                                        }
                                    )
                                offset = end * 1000 // 16000
                                done_ms += window_ms
                                await self.report_progress(job, started, done_ms, lengths)
                            groups = await self.run(self.repository.draft, job, track)
                            # Where the final lost a draft span or loops over
                            # it, that span keeps its draft; voices then apply
                            # to whichever words remain, by their times.
                            words, kept = integrity.choose(groups, words)
                            for key in ("spans", "duration_ms"):
                                result["kept_draft"][key] += kept[key]
                            if track == "remote_app" and existing is None:
                                phase = "diarization"
                                result.update(
                                    await separate(self.configuration, audio.fileno(), duration_ms)
                                )
                            result["words"].extend(attribute(words, result["spans"], track))
                            result["audio_ms"] = max(result["audio_ms"], duration_ms)
            phase = "persist"
            if not result["audio_ms"]:
                raise HubError(
                    "final_audio_empty",
                    "La grabación no tiene audio legible",
                    numbers={"audio_ms": 0},
                )
            await self.run(self.repository.complete, job, result)
        except TimeoutError:
            await self.run(
                self.repository.fail,
                job,
                {"code": "final_pass_timeout", "phase": phase, "timeout_ms": round(budget * 1000)},
            )
        except (wave.Error, EOFError, ValueError, KeyError):
            # Values are never quoted: the phase identifies the corrupt track.
            await self.run(
                self.repository.fail, job, {"code": "final_audio_invalid", "phase": phase}
            )
        except HubError as exc:
            await self.run(
                self.repository.fail, job, {"code": exc.code, "phase": phase, **(exc.numbers or {})}
            )
        except Exception:  # noqa: BLE001 - no audio, names, paths or provider diagnostics in evidence
            report(event="final_pass_failed", code="final_pass_internal_error")
            await self.run(
                self.repository.fail, job, {"code": "final_pass_internal_error", "phase": phase}
            )
        self.signal.set()
        self.changed(job["device_id"], job["recording"])
        return True
