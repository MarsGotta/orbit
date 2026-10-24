"""What Codex is told, written into its folder's AGENTS.md."""

import os
from contextlib import suppress
from pathlib import Path
from uuid import uuid4

from ..errors import HubError
from .agent_config import AgentInstructions, instructions_text

# What Codex reads from its folder, its `cwd`: every AGENTS.md from the
# project root down to it, concatenated (Codex 0.160.1, `agents_md.rs`).
INSTRUCTIONS_FILE = "AGENTS.md"


def write_instructions(folder: Path, current: AgentInstructions) -> None:
    """Write what Codex is told into its folder's AGENTS.md, or remove it.

    No skills folder is written: Codex lists a folder's skills by path and
    opens them with its shell, which the lock-down removes, so each skill
    reaches it through `orbit_skill` instead. The file is written whole or
    not at all (a temporary file, synced, then renamed over the old one), and
    a symbolic link in its place is refused rather than followed. The folder
    is the hub's alone.

    The agent's folder is private (`0700`, see `prepare_folder`); the file
    is `0644`, so an agent run as a separate system user with access to its
    folder can read it (docs/self-hosting.md).
    """
    path = folder / INSTRUCTIONS_FILE
    told = instructions_text(current)
    if told is not None:
        _write_whole(path, told)
    elif path.is_symlink() or path.exists():
        path.unlink()


def _write_whole(path: Path, text: str) -> None:
    if path.is_symlink():
        raise HubError("agent_folder_invalid", f"{path} es un enlace: no se escribe")
    temporary = path.with_name(f".{path.name}.{uuid4().hex}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            # The umask may have narrowed the mode; the agent's user must read it.
            os.fchmod(handle.fileno(), 0o644)
            handle.write(text.encode("utf-8"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        with suppress(FileNotFoundError):
            temporary.unlink()
        raise
