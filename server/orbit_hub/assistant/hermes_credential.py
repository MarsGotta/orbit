"""The credential of the hub's one Hermes, read from its file at each use."""

from __future__ import annotations

from typing import TYPE_CHECKING

from ..protocol import HubError
from ..secret_file import read_secret

if TYPE_CHECKING:
    from ..config import Settings


def credential(settings: Settings) -> str:
    value = read_secret(settings.hermes_credential_file, 64 * 1024).decode().strip()
    if not value or len(value) > 4096:
        raise HubError("configuration", "Hermes credential missing or invalid")
    return value
