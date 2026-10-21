"""Canonical UUID identities on the hub's wire boundaries."""

from typing import Annotated
from uuid import UUID

from pydantic import AfterValidator, Field


def canonical_uuid(value: str) -> str:
    if type(value) is not str or len(value) != 36 or str(UUID(value)) != value:
        raise ValueError("Canonical UUID required")
    return value


RecordingId = Annotated[str, Field(min_length=36, max_length=36), AfterValidator(canonical_uuid)]
