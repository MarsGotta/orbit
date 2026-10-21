"""Final worker, encrypted draft and ffmpeg; only provider responses are fake."""

import asyncio
import hashlib
import io
import subprocess
import wave
from email.parser import BytesParser
from random import Random
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_audio import wav_bytes
from support_notes import write_note
from test_final_pass import final_worker

from orbit_hub import meeting_final_pass as final
from orbit_hub.meeting_transcription import MeetingTranscriber
from orbit_hub.meetings import turns


def audio_bytes(seconds, *, silent=False, silences=()):
    pcm = bytearray((b"\x00\x00" if silent else b"\x00\x20") * (seconds * 16000))
    for start, end in silences:
        pcm[start * 32000 : end * 32000] = bytes((end - start) * 32000)
    return wav_bytes(bytes(pcm))


def seed(store, body, *, drafts=(), track="mic_local", offset_ms=0):
    owner, identity = device(store)["device_id"], str(uuid4())
    started = 1_760_000_000_000
    bodies = body if isinstance(body, dict) else {track: body}
    texts = drafts if isinstance(drafts, dict) else {track: drafts}
    tracks, duration_ms = [], 0
    for name, data in bodies.items():
        with wave.open(io.BytesIO(data)) as audio:
            frames, sequence = audio.getnframes(), 0
            while pcm := audio.readframes(160000):
                chunk = wav_bytes(pcm)
                store.put_chunk(
                    owner,
                    f"{identity}-{name}-{sequence}",
                    identity,
                    name,
                    sequence,
                    hashlib.sha256(chunk).hexdigest(),
                    chunk,
                    {
                        "frames": len(pcm) // 2,
                        "sample_rate": 16000,
                        "channels": 1,
                        "first_sample": sequence * 160000,
                        "monotonic_ns": 1_000_000_000 + sequence * 10_000_000_000,
                        "started_at_ms": started + offset_ms + sequence * 10000,
                    },
                )
                sequence += 1
        tracks.append(
            {
                "track": name,
                "chunk_count": sequence,
                "last_sequence": sequence - 1,
                "total_frames": frames,
            }
        )
        duration_ms = max(duration_ms, frames // 16)
    store.finalize_recording(
        owner,
        identity,
        {
            "started_at_ms": started,
            "finalized_at_ms": started + offset_ms + duration_ms,
            "gaps": 0,
            "tracks": tracks,
        },
    )
    indices = dict.fromkeys(bodies, 0)
    while job := store.claim_transcription(1000):
        name = job["manifest"]["track"]
        available, index = texts.get(name, ()), indices[name]
        text = available[index] if index < len(available) else ""
        if text is not None:
            store.finish_transcription(job, text, language="es")
        indices[name] += 1
    final.FinalPassRepository(store).request(owner, identity)
    return owner, identity


def fake_whisper(answer, *, decode=False):
    calls = []

    async def respond(request):
        mime = BytesParser().parsebytes(
            f"Content-Type: {request.headers['content-type']}\r\nMIME-Version: 1.0\r\n\r\n".encode()
            + request.content
        )
        fields = {
            part.get_param("name", header="content-disposition"): part.get_payload(decode=True)
            for part in mime.get_payload()
        }
        assert fields["file"].startswith(b"OggS")
        if decode:
            converted = await asyncio.to_thread(
                subprocess.run,
                [
                    "ffmpeg",
                    "-nostdin",
                    "-v",
                    "error",
                    "-i",
                    "pipe:0",
                    "-ar",
                    "16000",
                    "-ac",
                    "1",
                    "-f",
                    "s16le",
                    "pipe:1",
                ],
                input=fields["file"],
                capture_output=True,
                check=True,
                timeout=final.REQUEST_TIMEOUT_SECONDS,
            )
            fields["frames"] = len(converted.stdout) // 2
        words = answer(len(calls))
        calls.append(fields)
        return httpx.Response(
            200, json={"text": " ".join(w["word"] for w in words), "language": "es", "words": words}
        )

    transcriber = MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(respond)),
    )
    return transcriber, calls


async def run_pass(
    environment,
    monkeypatch,
    body,
    answer,
    *,
    drafts=(),
    track="mic_local",
    offset_ms=0,
    decode=False,
):
    _, store, _ = environment
    owner, identity = seed(store, body, drafts=drafts, track=track, offset_ms=offset_ms)
    transcriber, calls = fake_whisper(answer, decode=decode)

    async def separate(configuration, fd, duration):
        return {"spans": [{"start_ms": 0, "end_ms": duration, "voice": "voice_1"}]}

    monkeypatch.setattr(final, "separate", separate)
    assert await final_worker(store, transcriber, object()).run_once()
    with store.read_snapshot() as db:
        state = final.status(db, owner, identity)
        transcript = turns.read(store, db, owner, identity)
    return store, owner, identity, state, transcript, calls


def word(at, text="Palabra."):
    return {"start": at, "end": at + 0.1, "word": text}


async def test_a_draft_group_lost_by_the_final_pass_fails_with_numbers_and_draft_note(
    environment, monkeypatch
):
    drafts = [
        "Tenemos ocho palabras grabadas aquí para este borrador.",
        "Seguimos otras ocho palabras guardadas para cerrar este borrador.",
    ]
    # First group: 8 draft words and one final word (< quarter). The second
    # has 9 draft words and three final words, so only the first is lost.
    result = [word(5, "Una."), word(25, "Dos."), word(26, "Tres."), word(27, "Cuatro.")]
    store, owner, identity, state, transcript, calls = await run_pass(
        environment,
        monkeypatch,
        audio_bytes(40, silent=True),
        lambda _: result,
        drafts=drafts,
        track="remote_app",
    )
    assert state["state"] == "failed"
    error = state["error"]
    assert error["code"] == "final_pass_incomplete"
    assert error["phase"] == "whisper_remote_app"
    assert error["compared_groups"] == 2 and error["lost_groups"] == 1
    assert error["lost_draft_words"] == 8 and error["first_lost_group_ms"] == 0
    assert error["loop_words"] == 0
    assert len(calls) == 1
    assert store._db.execute("SELECT count(*) FROM recording_final_transcripts").fetchone()[0] == 0
    assert store._db.execute("SELECT count(*) FROM recording_voices").fetchone()[0] == 0
    assert drafts[0] in " ".join(s.text for s in transcript.segments)
    note = await write_note(store, owner, identity)
    assert note.repository.document(owner, identity)["note"]["final_pass_error"] == error
    assert drafts[0] in note.client.calls[0]["messages"][-1]["content"]


async def test_a_loop_fails_once_with_its_word_count(environment, monkeypatch):
    phrase = ["de", "lectura", "es", "un", "proyecto", "de", "la", "sala", "de", "lectura"]
    words = [word(1 + i * 0.2, phrase[i % len(phrase)]) for i in range(130)]
    store, owner, identity, state, transcript, calls = await run_pass(
        environment,
        monkeypatch,
        audio_bytes(40, silent=True),
        lambda _: words,
        drafts=["Borrador conservado."],
    )
    assert state["state"] == "failed"
    assert state["error"]["code"] == "final_pass_incomplete"
    assert state["error"]["loop_words"] == 130
    assert len(calls) == 1
    assert state["error"]["compared_groups"] == 0
    assert state["error"]["lost_groups"] == 0
    assert state["error"]["first_lost_group_ms"] is None
    assert store._db.execute("SELECT count(*) FROM recording_final_transcripts").fetchone()[0] == 0
    assert store._db.execute("SELECT count(*) FROM recording_voices").fetchone()[0] == 0
    assert "Borrador conservado." in " ".join(s.text for s in transcript.segments)
    note = await write_note(store, owner, identity)
    assert note.repository.document(owner, identity)["note"]["final_pass_error"] == state["error"]
    assert "Borrador conservado." in note.client.calls[0]["messages"][-1]["content"]


async def test_noise_with_no_draft_or_final_words_completes(environment, monkeypatch):
    _, _, _, state, transcript, calls = await run_pass(
        environment, monkeypatch, wav_bytes(Random(82).randbytes(30 * 32000)), lambda _: []
    )
    assert state == {"state": "completed", "error": None}
    assert not transcript.segments and len(calls) == 1


async def test_a_truly_silent_microphone_completes(environment, monkeypatch):
    _, _, _, state, transcript, calls = await run_pass(
        environment, monkeypatch, audio_bytes(30, silent=True), lambda _: []
    )
    assert state == {"state": "completed", "error": None}
    assert not transcript.segments and len(calls) == 1


async def test_long_tracks_use_silence_cuts_at_most_45_seconds_vad_and_recording_times(
    environment, monkeypatch
):
    # Longest eligible silence wins (32 rather than 21 s); 81.5 is before
    # 67 + 15, so it cannot cut that window. No silence after 145: cut at 190.
    quiet = [(20, 22), (29, 35), (47, 49), (65, 69), (80, 83), (90, 92), (120, 124), (140, 150)]
    _, _, _, state, transcript, calls = await run_pass(
        environment,
        monkeypatch,
        audio_bytes(200, silences=quiet),
        lambda index: [word(1, f"Ventana{index}.")],
        decode=True,
    )
    assert all(call["frames"] <= 45 * 16000 for call in calls)
    assert [call["frames"] for call in calls] == [
        32 * 16000,
        35 * 16000,
        24 * 16000,
        31 * 16000,
        23 * 16000,
        45 * 16000,
        10 * 16000,
    ]
    assert all(call["vad_filter"] == b"true" for call in calls)
    assert state == {"state": "completed", "error": None}
    assert [(s.at_ms, s.end_ms) for s in transcript.segments] == [
        (1000, 1100),
        (33000, 33100),
        (68000, 68100),
        (92000, 92100),
        (123000, 123100),
        (146000, 146100),
        (191000, 191100),
    ]


@pytest.mark.parametrize("delta_ms", [0, 1])
async def test_draft_times_use_recording_start_group_length_and_inclusive_one_second_margin(
    environment, monkeypatch, delta_ms
):
    words = [word(4 - delta_ms / 1000, "Borde."), word(26, "Final.")]
    _, _, _, state, _, calls = await run_pass(
        environment,
        monkeypatch,
        audio_bytes(40),
        lambda _: words,
        drafts=["Tenemos ocho palabras grabadas aquí para este borrador.", "Corto."],
        offset_ms=5000,
    )
    assert len(calls) == 1
    if delta_ms == 0:  # Exactly two words out of eight, inclusive at both edges.
        assert state == {"state": "completed", "error": None}
    else:
        assert state["state"] == "failed"
        assert state["error"]["code"] == "final_pass_incomplete"
        assert state["error"]["compared_groups"] == 1
        assert state["error"]["first_lost_group_ms"] == 5000
        assert state["error"]["lost_draft_words"] == 8


@pytest.mark.parametrize("count", [5, 6])
async def test_only_drafts_with_at_least_six_words_can_be_lost(environment, monkeypatch, count):
    draft = ["uno", "dos", "tres", "cuatro", "cinco", "seis"][:count]
    _, _, _, state, _, calls = await run_pass(
        environment, monkeypatch, audio_bytes(20), lambda _: [], drafts=[" ".join(draft)]
    )
    assert len(calls) == 1
    if count == 5:
        assert state == {"state": "completed", "error": None}
    else:
        assert state["state"] == "failed"
        assert state["error"]["lost_groups"] == 1
        assert state["error"]["lost_draft_words"] == 6


@pytest.mark.parametrize(
    "loop,total,failed", [(66, 66, False), (67, 300, True), (100, 500, False), (19, 1102, False)]
)
async def test_repetition_needs_both_measured_limits(environment, monkeypatch, loop, total, failed):
    words = [word(i * 0.03, "Sí." if i < loop else f"Distinta{i}.") for i in range(total)]
    _, _, _, state, transcript, calls = await run_pass(
        environment, monkeypatch, audio_bytes(40), lambda _: words
    )
    assert len(calls) == 1
    if failed:
        assert state["state"] == "failed"
        assert state["error"]["code"] == "final_pass_incomplete"
        assert state["error"]["loop_words"] == loop
        assert state["error"]["word_count"] == total
    else:
        assert state == {"state": "completed", "error": None}
        assert len(transcript.segments) == total


async def test_a_group_without_draft_text_is_unknown_not_a_lost_group(environment, monkeypatch):
    store, _, _, state, _, calls = await run_pass(
        environment, monkeypatch, audio_bytes(20), lambda _: [], drafts=[None]
    )
    assert state == {"state": "completed", "error": None}
    assert len(calls) == 1
    assert store._db.execute("SELECT count(*) FROM group_transcripts").fetchone()[0] == 0
    assert store._db.execute("SELECT state FROM transcription_groups").fetchone()[0] == "running"


async def test_other_track_draft_words_cannot_make_a_silent_microphone_incomplete(
    environment, monkeypatch
):
    _, _, _, state, transcript, calls = await run_pass(
        environment,
        monkeypatch,
        {"mic_local": audio_bytes(40), "remote_app": audio_bytes(40)},
        lambda index: [] if index == 0 else [word(5, "Uno."), word(6, "Dos.")],
        drafts={
            "mic_local": [""],
            "remote_app": ["Tenemos ocho palabras grabadas aquí para este borrador."],
        },
    )
    assert state == {"state": "completed", "error": None}
    assert len(calls) == 2
    assert {s.track for s in transcript.segments} == {"remote_app"}


async def test_fractional_silence_cuts_do_not_accumulate_word_timestamp_rounding(
    environment, monkeypatch
):
    pcm = bytearray(b"\x00\x20" * 1600000)
    # Longest pauses force midpoints 20.50175, 42.50325 and 64.50475 s.
    # Adding truncated window milliseconds would lose 1 ms by the third window.
    for start, end in [(288024, 368032), (656048, 704056), (1024072, 1040080)]:
        pcm[start * 2 : end * 2] = bytes((end - start) * 2)
    _, _, _, state, transcript, calls = await run_pass(
        environment,
        monkeypatch,
        wav_bytes(bytes(pcm)),
        lambda index: [word(1, f"Fracción{index}.")],
        decode=True,
    )
    assert state == {"state": "completed", "error": None}
    assert [call["frames"] for call in calls] == [328028, 352024, 352024, 567924]
    assert [(s.at_ms, s.end_ms) for s in transcript.segments] == [
        (1000, 1100),
        (21501, 21601),
        (43503, 43603),
        (65504, 65604),
    ]
