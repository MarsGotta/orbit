from support_audio import wav_bytes

"""A meeting is transcribed with its own settings, or not at all.

The defect these cover: the hub had no transcription settings of its own, so a
meeting inherited the Hermes profile's Whisper prompt, which is written for short
dictation into a close microphone. Over a weak stretch of room audio that prompt
comes back as its own vocabulary instead of as speech.
"""

import httpx
import pytest

from orbit_hub.config import Settings
from orbit_hub.meeting_transcription import (
    DIGITAL_SILENCE_PEAK,
    MAX_AUDIO_BYTES,
    MeetingTranscriber,
    MeetingTranscriptionConfiguration,
)
from orbit_hub.protocol import HubError
from orbit_hub.transcription import TranscriptionWorker

# Loopback over plain http, as in production, on the one port nothing can listen
# on: every request in this file goes through an httpx.MockTransport.
LOCAL = "http://127.0.0.1:0"


def configured(**overrides):
    fields = {"endpoint": LOCAL, "model": "openai/whisper-large-v3"}
    fields.update(overrides)
    return MeetingTranscriptionConfiguration(**fields)


class Recorder:
    """Captures the request the transcriber would send, and answers it."""

    def __init__(self, text="lo que se dijo de verdad", status=200):
        self.text, self.status, self.requests = text, status, []

    def factory(self):
        async def handle(request: httpx.Request) -> httpx.Response:
            self.requests.append(request)
            if self.status != 200:
                return httpx.Response(self.status, json={"error": "se rompió"})
            return httpx.Response(200, json={"text": self.text})

        return httpx.AsyncClient(transport=httpx.MockTransport(handle))


class Dictation:
    """Stands in for the interactive path, so we can prove it was not used."""

    def __init__(self):
        self.calls = 0

    async def transcribe(self, audio):
        self.calls += 1
        return "transcrito con el prompt de dictado"


def test_a_configuration_must_name_its_endpoint_and_model():
    # Half-configured is the state that produces a hub which looks configured and
    # silently is not, so it is refused at load rather than at the first meeting.
    with pytest.raises(ValueError):
        MeetingTranscriptionConfiguration()
    with pytest.raises(ValueError):
        MeetingTranscriptionConfiguration(endpoint=LOCAL)


def test_meeting_audio_may_only_leave_the_machine_over_a_protected_connection():
    MeetingTranscriptionConfiguration(endpoint=LOCAL, model="m")
    MeetingTranscriptionConfiguration(endpoint="https://gpu.example", model="m")
    with pytest.raises(ValueError):
        MeetingTranscriptionConfiguration(endpoint="http://gpu.example", model="m")


@pytest.mark.asyncio
async def test_a_meeting_is_not_transcribed_with_the_dictation_prompt():
    recorder = Recorder()
    transcriber = MeetingTranscriber(configured(), client_factory=recorder.factory)
    assert (await transcriber.transcribe(b"audio")).text == "lo que se dijo de verdad"

    body = recorder.requests[0].content.decode("utf-8", "replace")
    assert "openai/whisper-large-v3" in body
    # The field itself is absent, which is what "no prompt" has to mean on the wire.
    assert 'name="prompt"' not in body
    assert recorder.requests[0].url.path == "/v1/audio/transcriptions"


def test_a_worker_without_a_meeting_transcriber_is_rejected():
    # There is no valid path from durable meeting audio to the dictation adapter.
    dictation = Dictation()
    with pytest.raises(ValueError):
        TranscriptionWorker(store=None, speech=dictation, now=lambda: 0, meetings=None)
    assert dictation.calls == 0


@pytest.mark.asyncio
async def test_remote_protocol_failure_names_transport():
    def broken_client():
        def disconnect(_request):
            raise httpx.RemoteProtocolError("synthetic upstream disconnected")

        return httpx.AsyncClient(transport=httpx.MockTransport(disconnect))

    transcriber = MeetingTranscriber(configured(), client_factory=broken_client)
    with pytest.raises(HubError) as failure:
        await transcriber.transcribe(b"synthetic audio")
    assert failure.value.code == "transcription_protocol_error"
    assert "synthetic upstream disconnected" not in str(failure.value)


@pytest.mark.asyncio
async def test_timeout_names_the_meeting_request_budget():
    def broken_client():
        def expire(_request):
            raise httpx.ReadTimeout("synthetic timeout")

        return httpx.AsyncClient(transport=httpx.MockTransport(expire))

    transcriber = MeetingTranscriber(configured(), client_factory=broken_client)
    with pytest.raises(HubError) as failure:
        await transcriber.transcribe(b"synthetic audio")
    assert failure.value.code == "transcription_timeout"
    assert failure.value.numbers == {"timeout_ms": 300_000}


@pytest.mark.asyncio
async def test_a_failure_says_nothing_about_what_was_being_said():
    # Hub errors reach logs, and logs must never carry conversation content: the
    # endpoint's own error text can quote the transcript back at us.
    class Leaky:
        def __call__(self):
            async def handle(request):
                return httpx.Response(400, text="bad prompt near 'la nomina de octubre'")

            return httpx.AsyncClient(transport=httpx.MockTransport(handle))

    transcriber = MeetingTranscriber(configured(), client_factory=Leaky())
    with pytest.raises(HubError) as failure:
        await transcriber.transcribe(b"audio")
    assert "nomina" not in str(failure.value)


def write_settings(tmp_path, meeting: str) -> Settings:
    path = tmp_path / "hub.toml"
    path.write_text(
        'data_dir = "' + str(tmp_path / "data") + '"\n'
        'key_file = "' + str(tmp_path / "key") + '"\n'
        'hermes_url = "http://localhost:0"\n'
        "allow_insecure_localhost = true\n"
        'hermes_credential_file = "' + str(tmp_path / "cred") + '"\n' + meeting,
        encoding="utf-8",
    )
    return Settings.load(path)


def test_settings_loads_a_validated_meeting_transcription_section(tmp_path):
    loaded = write_settings(
        tmp_path,
        '[meeting_transcription]\nendpoint = "' + LOCAL + '"\nmodel = "openai/whisper-large-v3"\n',
    )
    assert isinstance(loaded.meeting_transcription, MeetingTranscriptionConfiguration)
    assert loaded.meeting_transcription.endpoint == LOCAL
    assert loaded.meeting_transcription.model == "openai/whisper-large-v3"
    assert loaded.meeting_transcription.language is None


def test_settings_rejects_the_removed_meeting_transcription_enabled_key(tmp_path):
    with pytest.raises(ValueError, match="meeting_transcription"):
        write_settings(
            tmp_path,
            "[meeting_transcription]\n"
            "enabled = true\n"
            'endpoint = "' + LOCAL + '"\n'
            'model = "openai/whisper-large-v3"\n',
        )


@pytest.mark.asyncio
async def test_audio_outside_the_bounds_is_refused_before_any_request():
    recorder = Recorder()
    transcriber = MeetingTranscriber(configured(), client_factory=recorder.factory)
    for audio in (b"", b"x" * (MAX_AUDIO_BYTES + 1)):
        with pytest.raises(HubError):
            await transcriber.transcribe(audio)
    assert recorder.requests == []


# On 2026-10-17 Whisper invented «¿Qué pasa con el presupuesto?» over a
# group that began with seconds of digital silence, and lost the meeting's first
# twenty seconds. The same audio, trimmed, came back right.


def pcm_wav(*parts: tuple[int, float], rate: int = 16000) -> bytes:
    """A mono 16-bit WAV of (amplitude, seconds) stretches: 0 is digital silence,
    anything else a square wave of that amplitude."""
    import array

    samples = array.array("h")
    for amplitude, seconds in parts:
        count = int(rate * seconds)
        samples.extend(amplitude if index % 2 else -amplitude for index in range(count))
    return wav_bytes(samples.tobytes(), rate=rate)


def frames_sent(request: httpx.Request) -> int:
    import io
    import wave

    body = request.content
    start = body.index(b"RIFF")
    with wave.open(io.BytesIO(body[start:]), "rb") as audio:
        return audio.getnframes()


async def test_digital_silence_around_the_words_is_trimmed_before_whisper():
    recorder = Recorder()
    transcriber = MeetingTranscriber(configured(), client_factory=recorder.factory)
    await transcriber.transcribe(pcm_wav((0, 4.0), (3000, 2.0), (0, 1.0)))
    assert frames_sent(recorder.requests[0]) == 2 * 16000


async def test_a_group_of_digital_silence_is_not_sent_and_says_nothing():
    recorder = Recorder()
    transcriber = MeetingTranscriber(configured(language="es"), client_factory=recorder.factory)
    heard = await transcriber.transcribe(pcm_wav((0, 20.0)))
    assert heard.text == "" and heard.language == "es"
    assert recorder.requests == []


async def test_a_microphone_floor_is_sound_and_is_never_trimmed():
    recorder = Recorder()
    transcriber = MeetingTranscriber(configured(), client_factory=recorder.factory)
    # A quiet room through a real microphone: just above digital silence.
    quiet = DIGITAL_SILENCE_PEAK + 4
    await transcriber.transcribe(pcm_wav((quiet, 3.0), (3000, 1.0)))
    assert frames_sent(recorder.requests[0]) == 4 * 16000
