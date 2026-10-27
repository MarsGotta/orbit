"""The configured model that turns meeting material into notes."""

from pathlib import Path

from pydantic import SecretStr

from .client import Completion, ProcessingClient, ProcessingError
from .config import ProcessingConfiguration

__all__ = [
    "Completion",
    "ProcessingClient",
    "ProcessingConfiguration",
    "ProcessingError",
    "open_client",
]


def open_client(configuration: ProcessingConfiguration) -> ProcessingClient:
    """Read the private credential once, at startup, and build the client."""
    from ..secret_file import read_secret

    if configuration.credential_file is None:
        return ProcessingClient(configuration, credential=SecretStr(""))
    secret = read_secret(Path(configuration.credential_file), 64 * 1024).decode("ascii").strip()
    return ProcessingClient(configuration, credential=SecretStr(secret))
