"""Dictation and read-aloud, apart from the assistant.

Separate from `AssistantBackend` because an assistant without a voice of its
own needs another service for it. Meeting transcription is not here: it has
its own transcriber (`meeting_transcription`).
"""

from dataclasses import dataclass
from typing import Protocol

# The longest clip the Mac plays (orbit_core::limits::MAX_SPEECH_CLIP_BYTES),
# so every speech backend refuses a longer one as `speech_too_large`. A copy
# of the registry value (contracts/constants.json), compared by
# scripts/tests/test_constants_copies.py.
MAX_SPEECH_CLIP_BYTES = 4 * 1024 * 1024


@dataclass(frozen=True)
class SpeechClip:
    mime_type: str
    provider: str
    body: bytes


class SpeechBackend(Protocol):
    async def transcribe(self, audio: bytes) -> str:
        """The text of one dictated PCM16 WAV utterance."""

    async def synthesize(self, text: str) -> SpeechClip:
        """One clip reading `text` aloud, at most `MAX_SPEECH_CLIP_BYTES`."""
