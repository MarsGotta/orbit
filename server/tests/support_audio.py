"""Synthetic PCM only: the common WAV encoding used by hub tests."""

import io
import wave


def wav_bytes(pcm: bytes, *, rate=16000, channels=1) -> bytes:
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as audio:
        audio.setnchannels(channels)
        audio.setsampwidth(2)
        audio.setframerate(rate)
        audio.writeframes(pcm)
    return buffer.getvalue()
