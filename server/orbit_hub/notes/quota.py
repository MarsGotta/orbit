"""The hub's storage quota, refused with its numbers.

`Store.ensure_capacity` refuses a write past the configured quota with a bare
`quota_exceeded`. A note, a template or a dictionary that cannot be stored must
say that, and how much: the bytes it needed, what the hub already uses and the
quota, so whoever reads the failure knows it is the hub's disk and not the
model, and by how much.
"""

from ..protocol import HubError


def reserve(store, payload_bytes: int) -> None:
    """`ensure_capacity`, with the numbers of a refusal."""
    try:
        store.ensure_capacity(payload_bytes)
    except HubError as failure:
        if failure.code != "quota_exceeded":
            raise
        numbers = {"bytes": payload_bytes, "used_bytes": store.physical_bytes()}
        # Store keeps its quota private; read, never changed, only to say it.
        quota = getattr(store, "_quota", None)
        if type(quota) is int:
            numbers["max_bytes"] = quota
        raise HubError(
            "quota_exceeded", failure.message, http_status=failure.http_status, numbers=numbers
        ) from None


def failure_of(failure: HubError) -> dict | None:
    """The stored cause of a quota refusal, or None for any other error."""
    if failure.code != "quota_exceeded":
        return None
    return {"code": "quota_exceeded", **(failure.numbers or {})}
