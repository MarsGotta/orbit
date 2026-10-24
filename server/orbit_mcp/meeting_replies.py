"""What a reply of the hub's meetings bridge must hold before a tool reads it.

Each check says whether the reply has the shape its route promises; the
client refuses one that does not as `invalid_response`, never as an empty
answer.
"""

from __future__ import annotations


def listing_valid(payload: dict) -> bool:
    return isinstance(payload.get("meetings"), list) and isinstance(payload.get("more"), bool)


def _incomplete_group_valid(group) -> bool:
    return (
        isinstance(group, dict)
        and isinstance(group.get("track"), str)
        and bool(group["track"])
        and type(group.get("first_index")) is int
        and type(group.get("last_index")) is int
        and isinstance(group.get("cause"), str)
        and bool(group["cause"])
        and type(group.get("attempts")) is int
    )


def transcript_valid(payload: dict, after_index: int) -> bool:
    text = payload.get("text")
    total = payload.get("total_chars")
    following = payload.get("next_after_index")
    groups = payload.get("incomplete_groups", [])
    if (
        not isinstance(text, str)
        or payload.get("after_index") != after_index
        or (following is not None and (type(following) is not int or following <= after_index))
        or (total is not None and type(total) is not int)
        or not isinstance(groups, list)
        or payload.get("capture", "closed") not in {"open", "closed"}
    ):
        return False
    return all(_incomplete_group_valid(group) for group in groups)


def note_valid(payload: dict) -> bool:
    return (
        isinstance(payload.get("note"), dict)
        and isinstance(payload.get("state"), str)
        and isinstance(payload.get("private_notes"), str)
        and isinstance(payload.get("edited"), dict)
    )


def attachment_valid(payload: dict, after_index: int) -> bool:
    attachment = payload.get("attachment")
    if not isinstance(attachment, dict) or not all(
        isinstance(attachment.get(key), str) for key in ("id", "name", "kind", "mime")
    ):
        return False
    if attachment["kind"] == "image":
        return isinstance(payload.get("data_base64"), str)
    return (
        isinstance(payload.get("text"), str)
        and payload.get("after_index") == after_index
        and type(payload.get("total_chars")) is int
    )
