"""Fast Note Sync REST writes using its read version and opaque contentHash.

Upstream REST currently ignores baseHash. Rechecks detect observed conflicts,
not an atomic compare-and-swap; callers journal partial/uncertain outcomes.
"""

from ..errors import HubError
from .models import MAX_NOTE_BYTES, refusal
from .paths import checked


def snapshot(sync, path):
    try:
        data = sync.get("note", path=checked(path))
    except HubError as exc:
        if exc.code == "vault_note_missing":
            return {"raw": None, "version": None}
        raise
    if (
        not isinstance(data, dict)
        or data.get("path") != path
        or not isinstance(data.get("content"), str)
    ):
        raise refusal("vault_sync_shape", "Fast Note Sync returned an invalid note.")
    if (
        type(data.get("version")) is not int
        or not isinstance(data.get("contentHash"), str)
        or not data["contentHash"]
    ):
        raise refusal(
            "vault_sync_version_missing",
            "Fast Note Sync did not supply a content hash and version; writing is refused.",
            409,
        )
    size = len(data["content"].encode())
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The note is {size} bytes; maximum {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    return {"raw": data["content"], "version": data["version"], "content_hash": data["contentHash"]}


def check(sync, path, previous):
    current = snapshot(sync, path)
    if current != previous:
        raise refusal(
            "distillation_content_changed",
            "The Fast Note Sync note changed since it was read.",
            409,
        )


def write(sync, path, text, previous):
    checked(path)
    size = len(text.encode())
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The note is {size} bytes; maximum {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    check(sync, path, previous)
    data = sync.request(
        "POST",
        "note",
        payload={
            "vault": sync.config.vault,
            "path": path,
            "content": text,
            "baseHash": previous.get("content_hash", ""),
            "createOnly": previous["raw"] is None,
            "context": "orbit-hub",
        },
    )
    # Read back: neither a lost receipt nor a malformed success permits an automatic retry.
    result = snapshot(sync, path)
    if (
        not isinstance(data, dict)
        or result["raw"] != text
        or result["version"] <= (previous["version"] or 0)
    ):
        raise refusal(
            "distillation_write_uncertain",
            "Fast Note Sync did not confirm the new content and version; inspect the proposal recovery state.",
            409,
        )
    return result


def remove(sync, path, previous):
    check(sync, path, previous)
    sync.request("DELETE", "note", parameters={"path": checked(path)})
    if snapshot(sync, path)["raw"] is not None:
        raise refusal(
            "distillation_write_uncertain", "Fast Note Sync did not confirm removal.", 409
        )
