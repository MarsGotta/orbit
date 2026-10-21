"""Where a recording's note is, with the numbers that explain it.

The note worker records each failure as a code plus its numbers (HTTP status,
seconds, tokens, bytes; never content). Deimos needs the same: a note that is
not there is either not requested, being written, waiting for its transcript,
queued for a retry or failed, and each of those says when or why.
"""

# Keys the bridge itself writes next to a failure's own numbers. A failure
# field with one of these names would overwrite the bridge's, so it is dropped.
_OWN_KEYS = frozenset({"code", "retry", "state", "cause", "attempt", "attempts", "retry_at_ms"})


def cause(error) -> dict:
    """The failure the worker recorded, flattened: its code as `cause` and its numbers."""
    if not isinstance(error, dict) or not isinstance(error.get("code"), str):
        return {}
    numbers: dict[str, int | str] = {"cause": error["code"]}
    for key, value in error.items():
        if key in _OWN_KEYS or isinstance(value, bool) or not isinstance(value, (int, str)):
            continue
        numbers[key] = value
    return numbers


def progress(document: dict) -> dict:
    """The numbers of the note's current state; empty when there is nothing to add."""
    state = document["state"]
    if state == "generating":
        return {"attempt": document["attempts"] + 1}
    if state == "waiting_transcript":
        return {"retry_at_ms": document["next_attempt_ms"]}
    if state == "queued":
        return {
            "retry_at_ms": document["next_attempt_ms"],
            "attempts": document["attempts"],
            **cause(document["error"]),
        }
    if state == "failed":
        return {"attempts": document["attempts"], **cause(document["error"])}
    return {}
