"""Meeting speech-to-text, played by the test process itself.

The hub sends a meeting's audio straight to the service named in
``[meeting_transcription]``: ``POST {endpoint}/v1/audio/transcriptions``,
multipart with ``file`` (a WAV), ``model`` and ``response_format``
(``verbose_json``), answered with ``{"text": ...}``. ``SyntheticStt`` is that
service with scripted answers.
A test serves it on a port it binds itself (``support_client_e2e.tcp_server``),
so the hub's real client crosses a real socket to it and to nothing else.

``UnitStt`` serves one from a thread for the whole unit suite, so every fixture
that must name an endpoint (the section is mandatory) names one this process
owns. It answers 503: a unit test that needs text starts its own
``SyntheticStt``, and one that does not must not depend on how fast a
transcription finishes.
"""

import asyncio
import io
import re
import socket
import sys
import threading
import time
import wave

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse

MODEL = "openai/whisper-large-v3"
PATH = "/v1/audio/transcriptions"
# What the hub sends (orbit_hub/meeting_transcription.py): 16-bit mono at 16 kHz.
SAMPLE_RATE, CHANNELS, SAMPLE_WIDTH = 16_000, 1, 2
UNAVAILABLE = object()
# Starting a uvicorn server on an already listening socket takes milliseconds;
# the bound only keeps a server that cannot start from leaving the run waiting
# on the hub's own 10 s connect and 300 s request timeouts, test after test.
START_TIMEOUT_SECONDS = 10


def form_fields(content_type: str, body: bytes) -> dict[str, tuple[str | None, str | None, bytes]]:
    """``name -> (filename, content type, exact bytes)`` of a multipart/form-data body."""
    kind, _, parameters = content_type.partition(";")
    if kind.strip().lower() != "multipart/form-data":
        raise ValueError(f"se esperaba multipart/form-data y llegó {kind.strip() or 'nada'}")
    boundary = re.search(r'boundary="?([^";]+)"?', parameters)
    if boundary is None:
        raise ValueError("el multipart no declara su boundary")
    delimiter = b"\r\n--" + boundary.group(1).encode("latin-1")
    pieces = (b"\r\n" + body).split(delimiter)
    if len(pieces) < 3 or not pieces[-1].startswith(b"--"):
        raise ValueError("el multipart no termina con su boundary de cierre")
    fields = {}
    for piece in pieces[1:-1]:
        head, separator, content = piece.removeprefix(b"\r\n").partition(b"\r\n\r\n")
        if not separator:
            raise ValueError("una parte del multipart no separa cabeceras y contenido")
        headers = head.decode("latin-1")
        name = re.search(r'(?im)^content-disposition:.*?\bname="([^"]*)"', headers)
        if name is None:
            raise ValueError("una parte del multipart no tiene nombre")
        filename = re.search(r'(?im)^content-disposition:.*?\bfilename="([^"]*)"', headers)
        media = re.search(r"(?im)^content-type:\s*([^\r\n;]+)", headers)
        fields[name.group(1)] = (
            filename.group(1) if filename else None,
            media.group(1).strip() if media else None,
            content,
        )
    return fields


def meeting_wav(fields, model: str) -> bytes:
    """The WAV of an accepted request; a refusal names what is wrong."""
    if "file" not in fields:
        raise ValueError(f"falta el campo file; llegaron {sorted(fields)}")
    for name, expected in (("model", model), ("response_format", "verbose_json")):
        value = fields.get(name, (None, None, b""))[2].decode("utf-8", "replace")
        if value != expected:
            raise ValueError(f"{name} es {value!r} y se esperaba {expected!r}")
    audio = fields["file"][2]
    try:
        with wave.open(io.BytesIO(audio), "rb") as reader:
            shape = (reader.getframerate(), reader.getnchannels(), reader.getsampwidth())
    except (wave.Error, EOFError) as exc:
        raise ValueError(f"el fichero no es un WAV legible ({exc})") from None
    if shape != (SAMPLE_RATE, CHANNELS, SAMPLE_WIDTH):
        raise ValueError(
            f"WAV de {shape[0]} Hz, {shape[1]} canales y {shape[2]} bytes por muestra; "
            f"el hub manda {SAMPLE_RATE} Hz, {CHANNELS} canal y {SAMPLE_WIDTH} bytes"
        )
    return audio


class SyntheticStt:
    """The OpenAI-compatible transcription service, scripted.

    Each outcome is text, an HTTP status, None to hold the request, a function
    of the request number and WAV, or UNAVAILABLE for 503. The last repeats.
    Each accepted WAV is kept in ``audio``; a malformed request is answered 422
    with its cause, kept in ``refusals`` and written to stderr, which pytest
    shows with the failure.
    """

    def __init__(self, *outcomes, model: str = MODEL):
        self.app = FastAPI()
        self.model = model
        self.outcomes = outcomes
        self.requests = []
        self.audio: list[bytes] = []
        self.refusals: list[str] = []

        @self.app.post(PATH)
        async def transcriptions(request: Request):
            try:
                body = await request.body()
                content_type = request.headers.get("content-type", "")
                fields = form_fields(content_type, body)
                audio = meeting_wav(fields, self.model)
            except ValueError as exc:
                return self._refuse(f"invalid request to the synthetic STT: {exc}")
            outcome = self.outcomes[min(len(self.audio), len(self.outcomes) - 1)]
            if outcome is UNAVAILABLE:
                return JSONResponse(
                    {"error": "the unit suite's synthetic STT does not transcribe"},
                    status_code=503,
                )
            self.audio.append(audio)
            self.requests.append({"body": body, "content_type": content_type})
            if isinstance(outcome, int):
                return JSONResponse({"error": "synthetic STT failure"}, status_code=outcome)
            if outcome is None:
                await asyncio.Future()
            if not callable(outcome):
                return {"text": outcome}
            try:
                return {"text": outcome(len(self.audio), audio)}
            except AssertionError as exc:
                return self._refuse(f"the synthetic STT refused request {len(self.audio)}: {exc}")

    def _refuse(self, cause: str) -> JSONResponse:
        self.refusals.append(cause)
        print(cause, file=sys.stderr)
        return JSONResponse({"error": cause}, status_code=422)

    def configuration(self, endpoint: str) -> dict:
        """The ``[meeting_transcription]`` section that points the hub here."""
        return {"endpoint": endpoint, "model": self.model}


class UnitStt:
    """A ``SyntheticStt`` that answers 503, served from a thread for a session."""

    def __init__(self):
        self.stt = SyntheticStt(UNAVAILABLE)
        self._socket = socket.socket()
        self._socket.bind(("127.0.0.1", 0))
        self._socket.listen(64)
        self.endpoint = f"http://127.0.0.1:{self._socket.getsockname()[1]}"
        config = uvicorn.Config(
            self.stt.app,
            log_level="critical",
            access_log=False,
            lifespan="off",
            timeout_graceful_shutdown=2,
        )
        self._server = uvicorn.Server(config)
        self._thread = threading.Thread(
            target=self._server.run,
            kwargs={"sockets": [self._socket]},
            name="orbit-unit-stt",
            daemon=True,
        )

    def start(self) -> "UnitStt":
        self._thread.start()
        deadline = time.monotonic() + START_TIMEOUT_SECONDS
        while not self._server.started:
            if not self._thread.is_alive() or time.monotonic() > deadline:
                raise RuntimeError(
                    f"the suite's synthetic STT did not start within {START_TIMEOUT_SECONDS} s "
                    f"at {self.endpoint}"
                )
            time.sleep(0.01)
        return self

    def stop(self) -> None:
        self._server.should_exit = True
        self._thread.join(START_TIMEOUT_SECONDS)
        self._socket.close()

    def configuration(self) -> dict:
        return self.stt.configuration(self.endpoint)


_SESSION: UnitStt | None = None


def start_session_stt() -> UnitStt:
    global _SESSION
    if _SESSION is None:
        _SESSION = UnitStt().start()
    return _SESSION


def stop_session_stt() -> None:
    global _SESSION
    if _SESSION is not None:
        _SESSION.stop()
        _SESSION = None


def unit_stt_configuration() -> dict:
    """The mandatory section for a unit test that does not test transcription."""
    if _SESSION is None:
        raise RuntimeError(
            "the suite's synthetic STT is not running; conftest.py starts it before the "
            "first test, so this cannot be called at import time"
        )
    return _SESSION.configuration()
