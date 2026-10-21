"""Explicit, typed server configuration."""

import os
import tomllib
import unicodedata
from pathlib import Path
from typing import ClassVar, Literal

from pydantic import ConfigDict
from pydantic.dataclasses import dataclass

from .error_reports import ErrorReportsConfiguration
from .meeting_diarization import DiarizationConfiguration
from .meeting_transcription import MeetingTranscriptionConfiguration
from .private_auth import BridgeConfiguration
from .processing.config import ProcessingConfiguration
from .urls import protected_url


@dataclass(frozen=True, config=ConfigDict(extra="forbid", hide_input_in_errors=True))
class Settings:
    data_dir: Path
    key_file: Path
    hermes_url: str
    hermes_credential_file: Path
    meeting_transcription: MeetingTranscriptionConfiguration
    profile: str = "default"
    assistant_name: str = "Deimos"
    # Bound the spoken invocation and UI label to a short name (48 Unicode
    # scalars, including spaces), independently of prompts and profile IDs.
    max_assistant_name_chars: ClassVar[int] = 48
    hermes_ws_auth: Literal["token"] = "token"
    allow_insecure_localhost: bool = False
    allowed_hosts: tuple[str, ...] = ("localhost", "127.0.0.1")
    # Preserve the published 8 MiB upload ceiling: it bounds one HTTP
    # allocation and admits grouped PCM with its WAV header. It is independent
    # of websocket frames and does not limit a durable meeting's length.
    max_chunk_bytes: ClassVar[int] = 8 * 1024 * 1024
    # Interactive dictation can carry three capture-sized pieces. This bounds
    # its HTTP allocation, not the length of a durable meeting.
    max_audio_bytes: ClassVar[int] = 24 * 1024 * 1024
    # Shared with the daemon's outgoing frame ceiling, guarded across languages.
    # A field rather than a ClassVar because contracts/constants.json reads its
    # default through `__dataclass_fields__`.
    max_frame_bytes: int = 128 * 1024
    # Preventive DB+WAL+SHM growth admission, not a filesystem quota. 10 GiB
    # leaves room for durable audio while retaining revocation/state headroom.
    storage_quota_bytes: ClassVar[int] = 10 * 1024 * 1024 * 1024
    device_bridge: BridgeConfiguration | None = None
    meetings_bridge: BridgeConfiguration | None = None
    processing: ProcessingConfiguration | None = None
    error_reports: ErrorReportsConfiguration | None = None
    diarization: DiarizationConfiguration | None = None

    def __post_init__(self) -> None:
        protected_url(self.hermes_url, allow_insecure_localhost=self.allow_insecure_localhost)
        count = len(self.assistant_name)
        if any(unicodedata.category(c) == "Cc" for c in self.assistant_name):
            cause = "contains control characters"
        elif not self.assistant_name.strip():
            cause = "is empty"
        elif count > self.max_assistant_name_chars:
            cause = "is too long"
        else:
            cause = None
        if cause is not None:
            raise ValueError(
                f"assistant_name {cause}: {count} characters; "
                f"maximum {self.max_assistant_name_chars}"
            )
        if not self.profile or len(self.profile) > 64 or "/" in self.profile:
            raise ValueError("Invalid server-selected Hermes profile")

    @classmethod
    def load(cls, path: Path) -> "Settings":
        with path.open("rb") as handle:
            values = tomllib.load(handle)
        if "meetings" in values:
            raise ValueError("The [meetings] section is now called [meetings_bridge]")
        for key in ("data_dir", "key_file", "hermes_credential_file"):
            if key in values:
                values[key] = Path(values[key]).expanduser()
        if "ORBIT_HUB_PROFILE" in os.environ:
            values["profile"] = os.environ["ORBIT_HUB_PROFILE"]
        if "ORBIT_HUB_ASSISTANT_NAME" in os.environ:
            values["assistant_name"] = os.environ["ORBIT_HUB_ASSISTANT_NAME"]
        return cls(**values)
