"""Bodies for the signed, backwards-compatible meeting listing route."""


def listing(device: str, since: int, limit: int, before: int | None, before_id: str | None) -> dict:
    request = {"device_id": device, "since_ms": since, "limit": limit}
    if before is not None:
        request["before_ms"] = before
    if before_id is not None:
        request["before_id"] = before_id
    return request
