"""Actionable vendor installation refusals, retaining the older Mac code."""

import errno
from pathlib import Path

from ..errors import HubError

NAMES = {"claude-code": "Claude (claude-code)", "codex": "Codex (codex)"}


class ToolUnavailable(ValueError):
    def __init__(self, path: Path, cause: str, agent_id: str, service: str):
        self.path, self.cause = str(path), cause
        super().__init__(f"agent {agent_id!r}: {service} command {path}: {cause}")


def missing(service: str, path: str, cause: str) -> HubError:
    return HubError(
        "agent_adapter_missing",
        f"{NAMES.get(service, service)} no está disponible en tu hub: {path} ({cause}); "
        "publica de nuevo el hub para instalarlo",
        http_status=409,
        numbers={"service": service, "path": path, "cause": cause},
    )


def launch_failure(service: str, path: str, error: OSError) -> HubError:
    cause = {
        errno.ENOENT: "no existe el ejecutable o su intérprete en esta plataforma",
        errno.ENOEXEC: "el binario no se puede ejecutar en esta plataforma",
        errno.EACCES: "no tiene permiso de ejecución",
    }.get(error.errno, f"el sistema rechazó el arranque (errno {error.errno})")
    return missing(service, path, cause)


def validate_linux_executable(service, path):
    """Retain exec-format diagnostics when bwrap becomes the direct child.

    The Linux kernel accepts ELF and scripts with an interpreter line; unlike
    some shells it does not execute headerless text. No vendor is run here.
    """
    try:
        with Path(path).open("rb") as handle:
            header = handle.read(4)
        if not (header.startswith(b"#!") or header == b"\x7fELF"):
            raise OSError(errno.ENOEXEC, "invalid executable format")
    except OSError as error:
        raise launch_failure(service, str(path), error) from None
