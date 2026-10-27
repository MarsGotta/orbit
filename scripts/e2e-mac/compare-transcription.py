#!/usr/bin/env python3
"""Manual provider acceptance on synthetic meeting-script audio; never run by the gate.

Use a 16 kHz mono PCM16 WAV synthesized from meeting-script.txt (all three
parts, without comments, speaker prefixes or separators). The reviewer supplies
it with --audio. No Mac control, recording, hub setting or database is touched.
Only word-recovery percentages go to stdout; credentials come from the environment.
"""

import argparse
import asyncio
import logging
import os
import re
import sys
import unicodedata
import wave
from collections import Counter
from pathlib import Path

# Run with the locked hub environment from a checkout; no package installation.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "server"))

from orbit_hub.final_pass_audio import encode_window
from orbit_hub.meeting_final_pass import WINDOW_SECONDS
from orbit_hub.meeting_transcription import MeetingTranscriber
from orbit_hub.openrouter_transcription import OpenRouterTranscriber
from orbit_hub.protocol import HubError


def words(text):
    plain = unicodedata.normalize("NFKD", text.casefold())
    return Counter(
        re.findall(r"[^\W_]+", "".join(c for c in plain if not unicodedata.combining(c)))
    )


def reference():
    lines = Path(__file__).with_name("meeting-script.txt").read_text().splitlines()
    return " ".join(
        line.split(":", 1)[1].strip() for line in lines if line.startswith(("A:", "B:"))
    )


async def compare(arguments):
    key = os.environ.get("OPENROUTER_API_KEY")
    if not key:
        raise SystemExit("OPENROUTER_API_KEY is required")
    whisper = MeetingTranscriber(
        {
            "endpoint": arguments.whisper_endpoint,
            "model": arguments.whisper_model,
            "language": "es",
        }
    )
    mai = OpenRouterTranscriber(key, whisper.configuration)
    # The whole recording never becomes one provider request. Same 45 s
    # ceiling and Opus encoder as production; input is streamed window by window.
    recovered = {"Whisper": "", "MAI": ""}
    with wave.open(str(arguments.audio), "rb") as source:
        if (source.getframerate(), source.getnchannels(), source.getsampwidth()) != (
            16000,
            1,
            2,
        ):
            raise SystemExit("Synthetic audio must be 16 kHz mono PCM16 WAV")
        while pcm := source.readframes(WINDOW_SECONDS * 16000):
            encoded = await asyncio.to_thread(encode_window, pcm)
            for name, transcriber in (("Whisper", whisper), ("MAI", mai)):
                result = await transcriber.transcribe(encoded, "es", final=True)
                recovered[name] += " " + result.text
    expected = words(reference())
    if not expected:
        raise SystemExit("Synthetic script is empty")
    for name, text in recovered.items():
        percentage = sum((expected & words(text)).values()) * 100 / sum(expected.values())
        print(f"{name} word recovery: {percentage:.2f}%")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--whisper-endpoint", required=True)
    parser.add_argument("--whisper-model", required=True)
    arguments = parser.parse_args()
    logging.disable(logging.CRITICAL)
    try:
        asyncio.run(compare(arguments))
    except HubError as error:
        raise SystemExit(f"Acceptance failed: {error.code}") from None
    except Exception:  # noqa: BLE001 - never reveal provider exceptions or credentials
        # Never print a provider exception, key, audio path or transcript.
        raise SystemExit("Acceptance failed before both models completed") from None


if __name__ == "__main__":
    main()
