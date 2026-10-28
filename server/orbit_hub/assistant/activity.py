"""What the agent does in a turn, as the Mac reads it: `chat.item` (hub-v1).

The backends translate their own events into `ActivityItem`s (`backend`);
this module writes the public item: copy codes with an English fallback for Orbit's own tools (orbit-mcp)
and the tool's own name for any other.
"""

from .activity_copy import SUBJECTS, label_text
from .backend import ActivityItem

# A label is one line the chat shows while a step runs, or the reasoning
# summary folded under it. 500 characters is Hermes's own reasoning preview
# (`reasoning.available`); past that it is no longer a line read in passing.
# It protects the frame and the Mac's assistant topic from upstream text of
# any length (a goal, a raw tool name, a thought). It does not limit what the
# agent does or says: the answer streams as `chat.delta`.
MAX_ITEM_LABEL_CHARS = 500
# What a tool step used, for the summary (“Used the screen and calendar”):
# a noun phrase, or a tool's own name, which upstream bounds nowhere.
MAX_ITEM_SUBJECT_CHARS = 80
# Why a step failed, in the agent's words: one line, as `the vendor refusal bound`.
MAX_ITEM_CAUSE_CHARS = 300
# The steps of a plan the Mac lists. Each update carries the whole plan, so
# this bounds every frame of it; the label still counts every step.
MAX_PLAN_STEPS = 20

# Orbit MCP tools, by the name the agent calls (an MCP client may
# prefix it with the server's name: `mcp__orbit__orbit_look`, `mcp_orbit_orbit_look`).
ORBIT_TOOLS = (
    "orbit_devices",
    "orbit_windows",
    "orbit_screenshot",
    "orbit_look",
    "orbit_selected_text",
    "orbit_control_tools",
    "orbit_perform_action",
    "orbit_action_result",
    "orbit_calendar",
    "orbit_notes_search",
    "orbit_note_read",
    "orbit_notes_index",
    "orbit_meetings",
    "orbit_transcript",
    "orbit_note",
    "orbit_files_list",
    "orbit_files_search",
    "orbit_files_read",
    "orbit_attachment",
    "orbit_skill",
)

_FIXED = {
    "orbit_devices": "macs",
    "orbit_windows": "windows",
    "orbit_look": "screen",
    "orbit_selected_text": "selected_text",
    "orbit_control_tools": "mac",
    "orbit_action_result": "mac",
    "orbit_calendar": "calendar",
    "orbit_notes_search": "notes",
    "orbit_note_read": "note",
    "orbit_notes_index": "notes_index",
    "orbit_meetings": "meetings",
    "orbit_transcript": "meetings",
    "orbit_note": "meetings",
    "orbit_files_list": "folders",
    "orbit_files_search": "folders",
    "orbit_files_read": "folders",
    "orbit_attachment": "attachments",
    "orbit_skill": "skills",
}


def one_line(text: object, limit: int) -> str:
    """`text` on one line, its runs of space and control characters made one
    space, cut to `limit` characters with an ellipsis."""
    if not isinstance(text, str):
        return ""
    words = "".join(" " if ord(char) < 32 or char == "\x7f" else char for char in text).split()
    line = " ".join(words)
    if len(line) <= limit:
        return line
    return line[: limit - 1] + "…"


def orbit_tool(name: str) -> str | None:
    """The orbit-mcp tool `name` calls, whatever prefix its client added."""
    for tool in ORBIT_TOOLS:
        if name == tool or name.endswith(("__" + tool, "/" + tool, "_" + tool, "." + tool)):
            return tool
    return None


def _app(arguments: object) -> str:
    if not isinstance(arguments, dict):
        return ""
    for key in ("app", "app_name", "application", "name", "bundle_id"):
        if value := one_line(arguments.get(key), MAX_ITEM_SUBJECT_CHARS):
            return value
    return ""


def _action_copy(arguments: dict) -> dict:
    action = arguments.get("action")
    action = action if isinstance(action, dict) else {}
    tool = one_line(action.get("tool"), MAX_ITEM_SUBJECT_CHARS)
    app = _app(action.get("arguments"))
    if "click" in tool:
        return {"code": "click_app", "value": app} if app else {"code": "click"}
    if tool.startswith("type"):
        # Never include typed text: it can be anything on the user's screen.
        return {"code": "type"}
    if tool in {"press_key", "hotkey"}:
        return {"code": "keys"}
    if tool == "scroll":
        return {"code": "scroll_app", "value": app} if app else {"code": "scroll"}
    if tool in {"launch_app", "bring_to_front"}:
        return {"code": "open_app", "value": app} if app else {"code": "open"}
    return {"code": "action_tool", "value": tool} if tool else {"code": "action"}


def tool_copy(name: str, arguments: dict | None) -> tuple[dict | None, str | None]:
    """Only known labels get codes; upstream names and content stay data."""
    arguments = arguments if isinstance(arguments, dict) else {}
    tool = orbit_tool(name)
    if tool in _FIXED:
        return {"code": tool}, _FIXED[tool]
    if tool == "orbit_screenshot":
        app = one_line(arguments.get("application"), MAX_ITEM_SUBJECT_CHARS)
        return (
            {"code": "screenshot_app", "value": app} if app else {"code": "screenshot"}
        ), "screen"
    if tool:
        return _action_copy(arguments), "mac"
    if name in {"Bash", "command"}:
        command = one_line(arguments.get("command"), MAX_ITEM_LABEL_CHARS - 12)
        return (
            {"code": "command_value", "value": command} if command else {"code": "command"}
        ), "hub_terminal"
    if name in {"Edit", "Write", "NotebookEdit", "edit"}:
        paths = arguments.get("paths") or [
            arguments.get("file_path") or arguments.get("notebook_path")
        ]
        value = one_line(
            ", ".join(path for path in paths if isinstance(path, str)), MAX_ITEM_LABEL_CHARS - 10
        )
        return ({"code": "edit_value", "value": value} if value else {"code": "edit"}), "hub_files"
    return None, None


def tool_label(name: str, arguments: dict | None) -> tuple[str, str]:
    """English fallback label and subject for an older Mac."""
    copy, subject = tool_copy(name, arguments)
    own = one_line(name, MAX_ITEM_SUBJECT_CHARS) or "tool"
    return (label_text(copy), SUBJECTS[subject]) if copy else (own, own)


def public_item(event: ActivityItem) -> dict:
    """The `chat.item` data of one step, without its session id."""
    data: dict = {"item_id": event.item_id, "kind": event.kind, "phase": event.phase}
    if event.kind == "tool":
        label, subject = tool_label(event.tool or "", event.arguments)
        data["label"] = one_line(label, MAX_ITEM_LABEL_CHARS)
        data["subject"] = subject
        copy, code = tool_copy(event.tool or "", event.arguments)
        if copy:
            data["label_copy"] = copy
            data["subject_code"] = code
    elif event.kind == "plan":
        data["label_copy"] = {
            "code": "plan",
            "done": sum(step.status in {"completed", "cancelled"} for step in event.steps),
            "total": len(event.steps),
        }
        data["label"] = label_text(data["label_copy"])
        data["steps"] = [
            {"text": one_line(step.text, MAX_ITEM_LABEL_CHARS), "status": step.status}
            for step in event.steps[:MAX_PLAN_STEPS]
        ]
    else:
        data["label"] = one_line(event.text, MAX_ITEM_LABEL_CHARS)
    if event.phase == "failed":
        data["cause"] = one_line(event.cause, MAX_ITEM_CAUSE_CHARS) or "No failure cause supplied"
        if event.cause_code or not event.cause:
            data["cause_code"] = event.cause_code or "tool_failure_unreported"
    return data
