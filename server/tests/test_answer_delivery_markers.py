"""Hermes's platform-delivery markers never reach the conversation or the Mac.

An agent that used Hermes's `text_to_speech` answers with a `MEDIA:<path>`
line, Hermes's way of handing an audio file to a messaging platform; it may
also ask for a voice bubble with `[[audio_as_voice]]`. Orbit reads answers
aloud itself, and the path names a file on the server. The paths here are
invented.
"""

import json
import logging
from uuid import uuid4

from conftest import FakeConnection, device, only, said

from orbit_hub.assistant.delivery_markers import MarkerStream, without_markers
from orbit_hub.log import logger
from orbit_hub.protocol import SendParams
from orbit_hub.runtime import DeviceRuntime

TTS_PATH = "/srv/ejemplo/perfil/cache/audio/tts_20990101_000000_000000.mp3"
ANSWER = (
    f"MEDIA:{TTS_PATH}\n"
    "[[audio_as_voice]]\n"
    "La sala Norte tiene 8 plazas.\n"
    "```\n"
    "MEDIA:/ejemplo/en/un/bloque.mp3\n"
    "```"
)
CLEAN = "La sala Norte tiene 8 plazas.\n```\nMEDIA:/ejemplo/en/un/bloque.mp3\n```"
# How the answer arrives, cut where a model's pieces may fall: inside the tag.
PIECES = ["MED", "IA:/srv/ejemplo/perfil/cache/", ANSWER[len("MEDIA:/srv/ejemplo/perfil/cache/") :]]


def answering_with_markers(monkeypatch):
    async def rpc(connection, method, params):
        connection.calls.append((method, params))
        if method == "prompt.submit":
            for piece in PIECES:
                await connection.callback(
                    {
                        "type": "message.delta",
                        "session_id": params["session_id"],
                        "payload": {"text": piece},
                    }
                )
            await connection.callback(
                {
                    "type": "message.complete",
                    "session_id": params["session_id"],
                    "payload": {"text": ANSWER, "status": "success"},
                }
            )
        return {"status": "streaming" if method == "prompt.submit" else "ok"}

    monkeypatch.setattr(FakeConnection, "rpc", rpc)


async def test_an_answer_with_media_tags_is_stored_and_sent_without_them(environment, monkeypatch):
    answering_with_markers(monkeypatch)
    _, store, adapter = environment
    owner = device(store, "Mac de prueba")["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)
    published = runtime.subscribe()
    records = []

    class Capture(logging.Handler):
        def emit(self, record):
            records.append(record)

    handler = Capture()
    logger.addHandler(handler)
    try:
        _, receipt = await runtime.handle(
            "chat.send", SendParams(text="¿Cuántas plazas tiene la sala?"), str(uuid4())
        )
    finally:
        logger.removeHandler(handler)

    assert said(store, owner, receipt["session_id"])[-1] == {"role": "assistant", "text": CLEAN}
    frames = []
    while not published.empty():
        frames.append(published.get_nowait())
    streamed = "".join(frame["data"]["text"] for frame in frames if frame["event"] == "chat.delta")
    assert TTS_PATH not in streamed
    assert "audio_as_voice" not in streamed
    assert "La sala Norte tiene 8 plazas." in streamed
    [completed] = [frame for frame in frames if frame["event"] == "chat.completed"]
    assert completed["data"]["text"] == CLEAN
    removed = [r for r in records if getattr(r, "event", None) == "delivery_markers_removed"]
    assert [record.numbers for record in removed] == [{"markers": 2}]
    logged = json.dumps([vars(record) for record in records], default=str)
    assert TTS_PATH not in logged
    assert "plazas" not in logged


async def test_a_voice_note_turn_tells_the_agent_that_orbit_reads_the_answer_aloud(environment):
    _, store, adapter = environment
    owner = device(store, "Mac de prueba")["device_id"]
    runtime = DeviceRuntime(owner, store, only(adapter), lambda: 1001)

    _, receipt = await runtime.handle(
        "chat.send",
        SendParams.model_validate({"text": "Lo que dije", "voice": {"duration_ms": 2_000}}),
        str(uuid4()),
    )
    voice_prompt = adapter.connections[0].calls[-1][1]["text"]
    await runtime.handle(
        "chat.send",
        SendParams(text="Ahora por escrito", session_id=receipt["session_id"]),
        str(uuid4()),
    )
    typed_prompt = adapter.connections[0].calls[-1][1]["text"]

    header, context, said_text = voice_prompt.split("\n", 2)
    assert header.startswith("[Orbit · Mac de prueba ·")
    assert "aloud" in context
    assert said_text == "Lo que dije"
    assert "aloud" not in typed_prompt
    # What the user sees is what they said, without the line for the agent.
    assert said(store, owner, receipt["session_id"])[0] == {"role": "user", "text": "Lo que dije"}


def test_a_tag_inside_a_sentence_goes_and_ordinary_lines_stream_at_once():
    text, removed = without_markers('Aquí va el audio: MEDIA:"/srv/ejemplo/nota.ogg" y ya.')
    assert (text, removed) == ("Aquí va el audio:  y ya.", 1)
    assert without_markers("MEDIA: no es una ruta") == ("MEDIA: no es una ruta", 0)

    stream = MarkerStream()
    assert stream.feed("Ma") == "Ma"
    assert stream.feed("ñana a las 9.\n") == "ñana a las 9.\n"
    assert stream.feed("**ME") == ""
    assert stream.feed("TA** cumplida\n") == "**META** cumplida\n"
    assert stream.feed("[[audio_as_voice]]\nFin") == "Fin"
