"""Bounded wire models for signed meeting reads."""

from typing import Literal

from pydantic import Field

from ..errors import PrivateError as MeetingsBridgeError
from ..identifiers import RecordingId
from ..private_auth import BridgeConfiguration as MeetingsBridgeConfiguration
from ..private_auth import PrivateModel

__all__ = ["MeetingsBridgeConfiguration", "MeetingsBridgeError"]

PROTOCOL = "orbit_meetings_bridge_v1"
LIST_PATH = "/internal/v1/meetings/list"
TRANSCRIPT_PATH = "/internal/v1/meetings/transcript"
NOTE_PATH = "/internal/v1/meetings/note"
HEADER_PREFIX = "x-orbit-meetings-"
REQUEST_BYTES = 16 * 1024
# The agent whose orbit-mcp asks (`ORBIT_MCP_AGENT`); none is the default agent.
AGENT_ID = Field(default=None, pattern=r"^[a-z0-9-]{1,32}$")


class ListRequest(PrivateModel):
    device_id: str = Field(min_length=1, max_length=128)
    since_ms: int = Field(ge=0, le=2**63 - 1)
    limit: int = Field(ge=1, le=100)
    # Continue a listing that answered `more: true`, from before the
    # start of the last meeting it returned.
    before_ms: int | None = Field(default=None, ge=1, le=2**63 - 1)
    agent_id: str | None = AGENT_ID


class NoteRequest(PrivateModel):
    device_id: str = Field(min_length=1, max_length=128)
    recording_id: RecordingId
    agent_id: str | None = AGENT_ID


class TranscriptRequest(PrivateModel):
    device_id: str = Field(min_length=1, max_length=128)
    recording_id: RecordingId
    format: Literal["text", "segments"] = "text"
    after_index: int = Field(default=0, ge=0, le=2**31 - 1)
    agent_id: str | None = AGENT_ID
