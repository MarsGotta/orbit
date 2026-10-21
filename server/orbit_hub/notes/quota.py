"""The stored cause of a note that the hub's storage quota refused.

`Store.ensure_capacity` refuses with its numbers (the bytes of the write, what
admission reserves for it, what the database uses and the quota); a note keeps
them with its failure, so whoever reads it knows it is the hub's disk and not
the model, and by how much.
"""

from ..protocol import HubError


def failure_of(failure: HubError) -> dict | None:
    """The stored cause of a quota refusal, or None for any other error."""
    if failure.code != "quota_exceeded":
        return None
    return {"code": "quota_exceeded", **(failure.numbers or {})}
