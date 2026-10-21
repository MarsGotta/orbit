"""Validated public protocol; upstream JSON-RPC is deliberately not exposed."""

import re
from typing import Any, Literal
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .errors import HubError  # noqa: F401 - public protocol re-export
from .identifiers import RecordingId


class StrictModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="ignore")


# Bounds the human recording title copied into the final manifest, and the
# name Mars gives a meeting since SIM-52 (`meetings.repository.meeting_title`):
# the same title, so the same bound, which the Mac keeps in
# `orbit_core::MAX_MANIFEST_TITLE_CHARACTERS` (both in contracts/constants.json
# as MAX_MEETING_TITLE_CHARS, compared by scripts/tests/test_constants_copies.py).
# Pydantic counts Unicode characters in `str`; this is not a byte, receipt,
# transcript, or source-content limit.
MAX_RECORDING_TITLE_CHARACTERS = 256
HUB_PROTOCOL = 1
# Same cause-label bound as the Mac; capability details remain opaque.
MAX_BRIDGE_CODE_BYTES = 64


class PairRequest(StrictModel):
    code: str = Field(min_length=16, max_length=160)
    name: str = Field(min_length=1, max_length=80)


class DeviceSummary(StrictModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    id: str = Field(min_length=1, max_length=128)
    name: str = Field(min_length=1, max_length=80)
    paired_at: float = Field(ge=0)
    connected: bool
    assistant_turn_active: bool | None = None
    last_seen: float = Field(ge=0)


class Frame(StrictModel):
    v: Literal[1]
    id: str
    method: Literal[
        "chat.send",
        "chat.interrupt",
        "approval.respond",
        "clarify.respond",
        "ping",
        "device.reply",
    ]
    params: dict = Field(default_factory=dict)
    # The action this frame belongs to (SIM-21 «Contrato con SIM-22»). Not part
    # of the request: an operation's fingerprint leaves it out, and one that is
    # not valid is replaced rather than refusing the frame (`log.bind_op`).
    op: Any = None

    def uuid(self) -> str:
        return str(UUID(self.id))


class SessionParams(StrictModel):
    session_id: str = Field(min_length=1, max_length=80)


class InterruptParams(SessionParams):
    turn_id: str = Field(min_length=1, max_length=80)


class VoiceParams(StrictModel):
    """SIM-30: the turn is a voice note of this length; its audio stays on the Mac."""

    duration_ms: int = Field(ge=0, le=2**63 - 1)


class MeetingParams(StrictModel):
    """SIM-30: the turn is a question about this meeting; the hub adds its
    last `minutes` of transcript. The title is the Mac's: the hub only learns
    it when the recording is finalized."""

    recording_id: RecordingId
    # Only selects transcript lines; the turn's size is bounded by
    # `meeting_context.MAX_TURN_BYTES`, so no ceiling is needed here.
    minutes: int = Field(ge=1)
    title: str = Field(default="", max_length=MAX_RECORDING_TITLE_CHARACTERS)


class SendParams(StrictModel):
    text: str = Field(min_length=1, max_length=32000)
    session_id: str | None = Field(default=None, max_length=80)
    call_active: bool = False
    # SIM-30: the Mac's id for the message, kept in the user's row so the Mac
    # can tell its own turn and its voice note's audio. A UUID; 80 is the
    # session id's bound.
    client_id: str | None = Field(default=None, min_length=1, max_length=80)
    voice: VoiceParams | None = None
    meeting: MeetingParams | None = None


class ApprovalParams(SessionParams):
    request_id: str = Field(min_length=1, max_length=160)
    approved: bool
    turn_id: str | None = Field(default=None, max_length=80)


class ClarifyParams(SessionParams):
    request_id: str = Field(min_length=1, max_length=160)
    answer: str = Field(max_length=16000)
    question_id: str | None = Field(default=None, max_length=160)


class SpeechRequest(StrictModel):
    text: str = Field(min_length=1, max_length=16000)


class EmptyParams(StrictModel):
    pass


class DeviceReplyParams(StrictModel):
    """A Mac answering something a remote assistant asked it.

    Either a refusal or a result, never both and never neither: "refused with a
    result" and "neither refused nor answered" are both shapes a caller would
    have to guess about, and guessing here means guessing about a picture of
    someone's screen.

    A frame is capped at 128 KB, so a screenshot cannot travel in one. Anything
    large is uploaded separately and named here by reference; that is a
    constraint of the transport, not a detail worth hiding from this contract.
    """

    request_id: str = Field(min_length=1, max_length=160)
    refusal: dict | None = None
    result: dict | None = None

    @model_validator(mode="before")
    @classmethod
    def current_envelope(cls, data):
        if isinstance(data, dict) and ("refused" in data or "code" in data):
            raise ValueError("La forma antigua refused + code ya no se admite")
        return data

    @model_validator(mode="after")
    def exactly_one_answer(self):
        if (self.refusal is None) == (self.result is None):
            raise ValueError("device reply needs exactly one result or refusal")
        if self.refusal is not None:
            code = self.refusal.get("code")
            if (
                not isinstance(code, str)
                or re.fullmatch(rf"[a-z0-9_]{{1,{MAX_BRIDGE_CODE_BYTES}}}", code) is None
            ):
                raise ValueError("a refusal requires its cause code")
        return self


class TrackManifest(StrictModel):
    track: Literal["mic_local", "remote_app"]
    chunk_count: int = Field(ge=1, le=1000000)
    last_sequence: int = Field(ge=0, le=1000000)
    total_frames: int = Field(ge=1, le=2**63 - 1)


class RecordingManifest(StrictModel):
    tracks: list[TrackManifest] = Field(max_length=2)
    gaps: int = Field(ge=0, le=2**63 - 1)
    started_at_ms: int = Field(ge=0, le=2**63 - 1)
    finalized_at_ms: int = Field(ge=0, le=2**63 - 1)
    title: str | None = Field(default=None, max_length=MAX_RECORDING_TITLE_CHARACTERS)
    # SIM-40: the Mac uploads its voice calls (⌃⌥D) as recordings too
    # (`recording_kind: voice_call`); a call is not a meeting, so the hub asks
    # for no note and the meetings list leaves it out. Absent: a meeting.
    kind: Literal["meeting", "voice_call"] = "meeting"
    # SIM-40: the calendar event the recording was started from (P9-11), so
    # the note the hub asks for at finalize has its title, invitees and series.
    # The finalize route validates it as `notes.models.EventContext`.
    event: dict | None = None


PARAMETERS = {
    "chat.send": SendParams,
    "chat.interrupt": InterruptParams,
    "approval.respond": ApprovalParams,
    "clarify.respond": ClarifyParams,
    "ping": EmptyParams,
    "device.reply": DeviceReplyParams,
}


def emit(name: str, data: dict, request_id: str | None = None) -> dict:
    return {"v": HUB_PROTOCOL, "id": request_id, "event": name, "data": data}
