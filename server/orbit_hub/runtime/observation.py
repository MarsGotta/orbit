"""A read of one conversation's live turn, never a vendor resume or prompt."""

import json

from ..interactions import pending_data

# One live preview per running conversation. 96 KiB fits with a 32 KiB user
# message and the 100 KiB pending-question budget under a 1 MiB Mac frame,
# even with JSON control-character escaping. This bounds only the in-memory
# preview, not the final answer or stored history. Terminal events release it.
MAX_PREVIEW_BYTES = 96 * 1024
# Sixty-four previews keep at most 6 MiB of preview UTF-8 per
# device, regardless of how many upstream sessions it has opened. An evicted
# preview is unknown; its turn and final history remain on the hub.
MAX_PREVIEWS = 64


def preview(text: str) -> str:
    return text.encode()[:MAX_PREVIEW_BYTES].decode(errors="ignore")


def remember_preview(previews: dict[str, str], session_id: str, text: str) -> None:
    if session_id not in previews and len(previews) == MAX_PREVIEWS:
        previews.pop(next(iter(previews)))
    previews[session_id] = preview(previews.get(session_id, "") + text)


def observe(runtime, session_id: str) -> dict:
    binding = runtime.store.chat.session(runtime.device, session_id)
    turn_id = runtime.turn_ids.get(session_id)
    pending = []
    for request_id, (owner, kind, request) in runtime.pending.items():
        if owner != session_id:
            continue
        item = pending_data(kind, request, request_id)
        if len(pending) >= 128 or len(json.dumps([*pending, item]).encode()) > 100 * 1024:
            continue
        pending.append(item)
    session = {
        "session_id": session_id,
        "turn_id": turn_id,
        "status": binding["status"],
        "pending": pending,
        "retry": runtime.events.retries.get(session_id),
    }
    if turn_id and binding["status"] in {"submitting", "streaming"}:
        page = runtime.store.chat.session_messages(runtime.device, session_id, None, 1)
        messages = page["messages"]
        if messages and messages[-1]["role"] == "user":
            message = messages[-1]
            user = {
                "id": turn_id,
                "role": "user",
                "text": message["text"],
                "at_ms": message["at_ms"] or 0,
                "complete": True,
                "attachments": message.get("attachments", []),
            }
            if message.get("kind") == "voice":
                user["voice"] = {
                    "duration_ms": message.get("duration_ms", 0),
                    "transcript": message["text"],
                    "audio_available": False,
                }
            text = runtime.events.previews.get(session_id, "")
            answer = (
                {
                    "id": turn_id + ":assistant",
                    "role": "assistant",
                    "text": text,
                    "at_ms": user["at_ms"] + 1,
                    "complete": False,
                }
                if text
                else None
            )
            session["turn"] = {"user": user, "answer": answer}
    return {"revision": runtime.revision, "sessions": [session], "omitted": True}
