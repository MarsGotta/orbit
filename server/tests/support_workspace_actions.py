"""A deterministic vendor tool that records execution only after the hub allows it.

It never executes shell text or opens the requested paths. Those may deliberately
name protected fixture folders. The execution record is the observable effect.
"""

import json
import os
from pathlib import Path


def operation():
    return json.loads(Path(os.environ["ORBIT_TOOL_RECORD"]).with_name("operation.json").read_text())


def claude_start(send, hook_id=None):
    action = operation()
    send(
        {
            "type": "assistant",
            "message": {
                "model": "fixture",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "host-1",
                        "name": action["tool"],
                        "input": action["arguments"],
                    }
                ],
            },
        }
    )
    if hook_id:
        send(
            {
                "type": "control_request",
                "request_id": "host-hook",
                "request": {
                    "subtype": "hook_callback",
                    "callback_id": hook_id,
                    "tool_use_id": "host-1",
                    "input": {
                        **(
                            {"agent_id": "fixture-child", "agent_type": "general-purpose"}
                            if action.get("subagent")
                            else {}
                        ),
                        "hook_event_name": "PreToolUse",
                        "tool_name": action["tool"],
                        "tool_input": action["arguments"],
                    },
                },
            }
        )
        return
    claude_permission(send)


def claude_permission(send):
    action = operation()
    send(
        {
            "type": "control_request",
            "request_id": "host-approval",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": action["tool"],
                "input": action["arguments"],
                "tool_use_id": "host-1",
            },
        }
    )


def claude_end(send, record, allowed):
    if allowed:
        record({"executed": operation()})
    send(
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "host-1",
                        "is_error": not allowed,
                        "content": "done" if allowed else "denied",
                    }
                ]
            },
        }
    )


def codex_start(send, notify, thread, turn):
    action = operation()
    command = action["tool"] == "Bash"
    arguments = action["arguments"]
    item = {
        "id": "host-1",
        "type": "commandExecution" if command else "fileChange",
        "command": arguments.get("command"),
        "cwd": os.getcwd(),
        "changes": [{"path": arguments.get("file_path"), "kind": {"type": "update"}, "diff": ""}],
        "status": "inProgress",
    }
    notify("item/started", item=item)
    send(
        {
            "id": "host-approval",
            "method": "item/commandExecution/requestApproval"
            if command
            else "item/fileChange/requestApproval",
            "params": {
                "threadId": thread,
                "turnId": turn,
                "itemId": "host-1",
                "cwd": os.getcwd(),
                "command": arguments.get("command"),
                "reason": "Fixture action",
            },
        }
    )
    return item


def codex_end(record, notify, item, allowed):
    if allowed:
        record({"executed": operation()})
    notify("item/completed", item={**item, "status": "completed" if allowed else "declined"})


def claude_end_legacy(send, allowed):
    send(
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "command-1",
                        "is_error": not allowed,
                        "content": "done" if allowed else "denied",
                    }
                ]
            },
        }
    )


def codex_legacy_item(waiting):
    return {
        "id": "command-1",
        "type": "fileChange" if waiting == "edit" else "commandExecution",
        "command": "echo fixture",
        "cwd": os.getcwd(),
        "changes": [{"path": "fixture.md", "kind": {"type": "update"}, "diff": ""}],
        "status": "inProgress",
    }
