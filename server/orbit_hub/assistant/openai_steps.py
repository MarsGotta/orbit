"""What an `openai` agent's turn is doing, from pydantic-ai's stream, as the
hub's activity items."""

from uuid import uuid4

from pydantic_ai import (
    FunctionToolCallEvent,
    FunctionToolResultEvent,
    PartDeltaEvent,
    PartStartEvent,
    RetryPromptPart,
    ThinkingPart,
    ThinkingPartDelta,
)

from .backend import ActivityItem, EventSink


class TurnSteps:
    """One turn's steps as `ActivityItem`s, from pydantic-ai's stream: a tool
    call from its call to its result, and a thought from its first piece to
    the start of the next part or the end of the run, with the whole thought."""

    def __init__(self, session_id: str, on_event: EventSink):
        self.session_id, self.on_event = session_id, on_event
        # This turn's calls by id: the name and arguments only the call carries.
        self.calls: dict[str, tuple[str, dict | None]] = {}
        self.thought: list[str] = []
        self.thought_id: str | None = None

    async def on(self, event: object) -> None:
        if isinstance(event, PartStartEvent):
            await self.end_thought()
            if isinstance(event.part, ThinkingPart):
                self.thought_id = f"reasoning-{uuid4()}"
                self.thought = [event.part.content]
                await self.on_event(
                    ActivityItem(self.session_id, self.thought_id, "reasoning", "started")
                )
        elif isinstance(event, PartDeltaEvent) and isinstance(event.delta, ThinkingPartDelta):
            if self.thought_id is not None and event.delta.content_delta:
                self.thought.append(event.delta.content_delta)
        elif isinstance(event, FunctionToolCallEvent):
            await self.end_thought()
            part = event.part
            try:
                arguments: dict | None = part.args_as_dict(raise_if_invalid=True)
            except (ValueError, AssertionError):
                # Arguments that are not a JSON object: pydantic-ai answers the
                # model with a retry, which ends this step as failed.
                arguments = None
            self.calls[part.tool_call_id] = (part.tool_name, arguments)
            await self.on_event(
                ActivityItem(
                    self.session_id, part.tool_call_id, "tool", "started", part.tool_name, arguments
                )
            )
        elif isinstance(event, FunctionToolResultEvent):
            part = event.part
            name, arguments = self.calls.pop(part.tool_call_id, (part.tool_name or "", None))
            if isinstance(part, RetryPromptPart):
                phase, cause = "failed", _retry_cause(part)
            elif part.outcome != "success":
                # orbit-mcp's refusals (`ToolFailed`) arrive as a failed return.
                phase, cause = "failed", part.model_response_str(wrap_if_error=False)
            else:
                phase, cause = "completed", None
            await self.on_event(
                ActivityItem(
                    self.session_id, part.tool_call_id, "tool", phase, name, arguments, cause=cause
                )
            )

    async def end_thought(self) -> None:
        if self.thought_id is None:
            return
        item_id, text = self.thought_id, "".join(self.thought)
        self.thought, self.thought_id = [], None
        await self.on_event(
            ActivityItem(self.session_id, item_id, "reasoning", "completed", text=text)
        )


def _retry_cause(part: RetryPromptPart) -> str:
    """Why pydantic-ai asked the model to try again, in its words."""
    if isinstance(part.content, str):
        return part.content
    return "; ".join(str(error.get("msg", "")) for error in part.content)
