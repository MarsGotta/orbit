"""Turning a request body into a typed model, or a refusal that names field and numbers."""

import json

from pydantic import ValidationError

from ..protocol import HubError
from .series_key import SeriesKeyError


def _numbers(error: dict) -> dict:
    """The bound the field broke and, when known, by how much (never its value)."""
    context = error.get("ctx") or {}
    numbers: dict = {}
    for source, target in (
        ("max_length", "max"),
        ("min_length", "min"),
        ("actual_length", "actual"),
        ("ge", "min"),
        ("lt", "below"),
    ):
        if isinstance(context.get(source), int) and not isinstance(context.get(source), bool):
            numbers[target] = context[source]
    cause = context.get("error")
    if isinstance(cause, SeriesKeyError):
        numbers.update(cause.numbers, cause=cause.cause)
    return numbers


def refusal(failure: ValidationError) -> tuple[str, str, dict]:
    """The first broken field, a sentence about it and its numbers."""
    error = failure.errors(include_input=False)[0]
    field = ".".join(str(part) for part in error["loc"]) or "cuerpo"
    cause = (error.get("ctx") or {}).get("error")
    reason = cause.reason if isinstance(cause, SeriesKeyError) else error["type"]
    return field, reason, {"field": field, **_numbers(error)}


def parse(body: bytes, model, code: str, what: str):
    try:
        return model.model_validate(json.loads(body or b"{}"))
    except ValueError as failure:
        if not isinstance(failure, ValidationError):
            raise HubError(code, f"{what} invalid: the body is not JSON") from None
        field, reason, numbers = refusal(failure)
        raise HubError(code, f"{what} invalid: {field} ({reason})", numbers=numbers) from None
