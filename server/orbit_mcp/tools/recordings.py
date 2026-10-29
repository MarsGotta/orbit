"""The Mac's recorded meetings, read from the hub: the list, a transcript, a note."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult

from orbit_hub.identifiers import canonical_uuid
from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

from .. import meetings
from ..hub import HubClientError
from .runner import ToolRunner, ok, page_start, text

TRANSCRIPT_DESCRIPTION = (
    "Reads an Orbit recording transcript, one line per turn: [hh:mm:ss] Name: text, "
    "with time since the recording began. Names match those the Orbit user sees: "
    "the Mac microphone uses its owner's name, each voice its assigned name; "
    "separate unnamed voices are ‘Person 1’, ‘Person 2’… and the draft says ‘Another person’. "
    "For an ongoing meeting, returns the transcript so far, marked partial, with "
    "how much audio remains to transcribe. Long transcripts are paginated with up to "
    f"{MAX_ANSWER_CHARS} characters: the first line states the range and after_index "
    "to continue. format=segments gives at_ms/end_ms times, track and speaker_key "
    "to identify a turn, one JSON line per transcribed chunk."
)

NOTE_DESCRIPTION = (
    "Reads a recording note as the Orbit user sees it: title, short summary, "
    "template sections with their edits applied, action items with owner, deadline "
    "and completion state, decisions, open questions, private notes, template, "
    "the model that wrote it and when. If unwritten, states its status and cause "
    "with numbers (for example, when the next attempt is due). Read-only. Long notes "
    f"are paginated with up to {MAX_ANSWER_CHARS} characters and after_index to continue."
)


def _recording(value, field: str = "recording_id") -> str:
    try:
        return canonical_uuid(value)
    except ValueError:
        raise HubClientError(
            "invalid_parameters",
            detail={"refusal": {"code": "invalid_parameters", "parameter": field}},
        ) from None


def _since_ms(clock, since_days: int | None, before_ms: int | None) -> int:
    days = 7 if since_days is None else since_days
    if type(days) is not int or not 1 <= days <= 365:
        raise HubClientError("invalid_parameters")
    if before_ms is not None and (type(before_ms) is not int or before_ms < 1):
        raise HubClientError("invalid_parameters")
    return max(0, int(clock() * 1000) - days * 24 * 60 * 60 * 1000)


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Lists the Orbit user's meetings across all their Macs, from latest to earliest "
            "start, including ongoing meetings (marked IN PROGRESS). Each states the real "
            "status of its note — ready with title and short summary, being written, waiting "
            "for transcription, queued, failed with cause, not requested or no model configured — "
            "and its transcription. Returns at most "
            f"{meetings.MEETINGS_PER_ANSWER}; if more exist, reports it and gives before_ms "
            "and before_id to continue. since_days (1–365, default 7) bounds the period."
        ),
        structured_output=False,
    )
    async def orbit_meetings(
        device: str | None = None,
        since_days: int | None = None,
        before_ms: int | None = None,
        before_id: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            since_ms = _since_ms(runner.clock, since_days, before_ms)
            listing = await hub_client.meetings(
                selection.device_id,
                since_ms,
                meetings.MEETINGS_PER_ANSWER,
                before_ms,
                None if before_id is None else _recording(before_id, "before_id"),
            )
            return ok(text(meetings.render_meetings(listing, since_ms)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=TRANSCRIPT_DESCRIPTION,
        structured_output=False,
    )
    async def orbit_transcript(
        recording_id: str,
        device: str | None = None,
        format: str | None = None,
        after_index: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            selected_format = "text" if format is None else format
            if selected_format not in {"text", "segments"}:
                raise HubClientError("invalid_parameters")
            recording = _recording(recording_id)
            start = page_start(after_index, "after_index")
            # The hub's page holds far more than one answer; the rest is one
            # `after_index` away, so it is never fetched only to be thrown away.
            page = await hub_client.transcript(
                selection.device_id, recording, selected_format, start
            )
            return ok(text(meetings.render_transcript(page, start)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=NOTE_DESCRIPTION,
        structured_output=False,
    )
    async def orbit_note(
        recording_id: str,
        device: str | None = None,
        after_index: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            recording = _recording(recording_id)
            start = page_start(after_index, "after_index")
            document = await hub_client.note(selection.device_id, recording)
            return ok(text(meetings.render_note(document, start)))

        return await runner.invoke(operation, device)
