"""What a paired device is and which groups of orders it handles.

A device says both when it pairs (`protocol.PairRequest`). One that says
nothing is a Mac that handles every group, as every Orbit before them; an
empty list is a device that handles none. These are not an agent's access
groups (`assistant.access.CAPABILITY_GROUP`, Settings › Assistants): those say
what an agent may use, these what a device can answer.
"""

from types import MappingProxyType

DEVICE_KINDS = ("mac", "phone")
# Every group, in the order the inventory lists them; orbit-mcp imports this
# same tuple. Registered in contracts/constants.json as DEVICE_GROUPS, which
# server/tests/test_constants_registry.py compares with it. The microphone is
# not a group: every device has one.
DEVICE_GROUPS = (
    "system_audio",
    "screen",
    "selected_text",
    "folders",
    "calendar",
    "call_detection",
    "shortcuts",
    "control",
)

# The device group each order the hub relays belongs to. An order missing
# here is relayed as before: the device answers it or refuses it itself.
# `system_audio`, `call_detection` and `shortcuts` are the Mac's own work,
# never an order of the bridge, so no entry names them.
DEVICE_GROUP = MappingProxyType(
    {
        "screen_sources": "screen",
        "screenshot": "screen",
        "selected_text": "selected_text",
        "files_list": "folders",
        "files_search": "folders",
        "files_read": "folders",
        "calendar_read": "calendar",
        "control_tools": "control",
        "perform_action": "control",
    }
)


def handled(capabilities: list[str] | None) -> list[str]:
    """The groups a device handles: every one when it said none."""
    return list(DEVICE_GROUPS) if capabilities is None else list(capabilities)


def refusal_message(numbers: dict) -> str:
    """Why the hub did not relay an order: the device's kind, the group and the order."""
    return (
        f"That device ({numbers.get('kind')}) does not handle «{numbers.get('group')}»: "
        f"it cannot answer {numbers.get('capability')}."
    )
