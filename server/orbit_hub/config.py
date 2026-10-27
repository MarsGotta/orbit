"""Explicit, typed server configuration."""

import os
import tomllib
from functools import cached_property
from pathlib import Path
from typing import Any, ClassVar, Literal

from pydantic import ConfigDict
from pydantic.dataclasses import dataclass

from .assistant.agent_kinds import AgentConfiguration, configured_agents
from .assistant.tool_settings import ToolSettings, default_tools, parse_assistant_tool, parse_jail
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
    # The implicit agent's, when there is no `[[agents]]` (`assistant.agents`).
    config_file: Path | None = None
    tls_key_file: Path | None = None
    profile: str = "default"
    assistant_name: str = "Deimos"
    # `[[agents]]` as TOML read them; each kind reads its own keys.
    agents: tuple[dict[str, Any], ...] | None = None
    default_agent: str | None = None
    # `[assistant_tools.claude-code]` and `[assistant_tools.codex]`: the official tool the
    # hub starts for each agent added from Orbit. Optional overrides of the
    # bundled/shipped commands, generated MCP configuration and environment.
    # `[assistant_tools.jail]` optionally adds host secret discovery roots.
    assistant_tools: dict[str, dict[str, Any]] | None = None
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
    # A cap, in bytes, on the database (DB+WAL+SHM) the hub admits content
    # into; absent, the disk sets the limit (`storage.STORAGE_FREE_FLOOR_BYTES`
    # says how). What it protects: a disk the hub shares with other things the
    # operator wants to keep room for. It is admission, not retention: nothing
    # is deleted to stay under it, and the Mac keeps what was refused.
    storage_quota_bytes: int | None = None
    device_bridge: BridgeConfiguration | None = None
    meetings_bridge: BridgeConfiguration | None = None
    processing: ProcessingConfiguration | None = None
    error_reports: ErrorReportsConfiguration | None = None
    diarization: DiarizationConfiguration | None = None

    def __post_init__(self) -> None:
        if self.storage_quota_bytes is not None and self.storage_quota_bytes <= 0:
            raise ValueError(
                "storage_quota_bytes must be a positive number of bytes; remove it to let "
                "the disk set the limit"
            )
        protected_url(self.hermes_url, allow_insecure_localhost=self.allow_insecure_localhost)
        self.agent_configurations()
        self.assistant_tool_settings()
        self.assistant_jail_roots()

    def assistant_jail_roots(self) -> tuple[Path, ...]:
        return parse_jail((self.assistant_tools or {}).get("jail", {}))

    @cached_property
    def assistant_jail_cache(self):
        from .assistant.jail_secret_cache import SecretCache

        return SecretCache(self.assistant_jail_roots(), self.data_dir)

    def assistant_tool_settings(self) -> dict[str, ToolSettings]:
        """Both official services, with operator overrides when supplied."""
        defaults = default_tools(self.data_dir)
        overrides = {
            service: parse_assistant_tool(service, keys, defaults.get(service))
            for service, keys in (self.assistant_tools or {}).items()
            if service != "jail"
        }
        return {
            **overrides,
            **{key: value for key, value in defaults.items() if key not in overrides},
        }

    def agent_configurations(self) -> tuple[tuple[AgentConfiguration, ...], str]:
        """The agents this configuration describes and the default's id."""
        return configured_agents(
            self.agents,
            self.default_agent,
            assistant_name=self.assistant_name,
            profile=self.profile,
        )

    @classmethod
    def load(cls, path: Path) -> "Settings":
        with path.open("rb") as handle:
            values = tomllib.load(handle)
        if "meetings" in values:
            raise ValueError("The [meetings] section is now called [meetings_bridge]")
        for key in ("data_dir", "key_file", "hermes_credential_file"):
            if key in values:
                values[key] = Path(values[key]).expanduser()
        if "agents" in values:
            # Beside `[[agents]]` these would be read by nothing: refused, not ignored.
            for key in ("profile", "assistant_name"):
                if key in values:
                    raise ValueError(
                        f"{key} configures the implicit agent; with [[agents]] each entry "
                        f"has its own: remove {key}"
                    )
            for variable in ("ORBIT_HUB_PROFILE", "ORBIT_HUB_ASSISTANT_NAME"):
                if variable in os.environ:
                    raise ValueError(
                        f"{variable} configures the implicit agent; with [[agents]] each "
                        f"entry has its own: unset {variable}"
                    )
        if "ORBIT_HUB_PROFILE" in os.environ:
            values["profile"] = os.environ["ORBIT_HUB_PROFILE"]
        if "ORBIT_HUB_ASSISTANT_NAME" in os.environ:
            values["assistant_name"] = os.environ["ORBIT_HUB_ASSISTANT_NAME"]
        values["config_file"] = path.resolve()
        return cls(**values)
