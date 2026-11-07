"""The private files that hold a final pass's audio while ffmpeg and the
separator read and write it.

On Linux each one is a memfd: no directory lists it and it never reaches the
disk, so the production hub keeps no plaintext audio on disk. Elsewhere (a hub
on a Mac) it is a file in the system temp folder (`tempfile.gettempdir()`,
which follows TMPDIR; on macOS it is the user's own), created exclusively
under an unpredictable name with mode 0600 asked for at creation, never left
to the umask (077 on a server, 022 on a Mac). ffmpeg rewrites a file it is
given without changing that mode. The file is removed when its context ends:
on success, on failure and on cancellation alike.
"""

import os
import secrets
import tempfile
from contextlib import contextmanager, suppress

from . import host_system


@contextmanager
def private_file():
    """A read-write binary file for one piece of a final pass's audio."""
    if host_system.linux():
        descriptor = os.memfd_create("orbit-final-audio", os.MFD_CLOEXEC)
        with os.fdopen(descriptor, "w+b") as handle:
            yield handle
        return
    path = os.path.join(tempfile.gettempdir(), f"orbit-final-audio-{secrets.token_hex(16)}")
    # "x": created here or not at all; never a file or link already there,
    # which is then not this pass's to remove.
    with open(path, "x+b", opener=_owner_only) as handle:
        try:
            yield handle
        finally:
            # Also when the pass is cancelled, which is not an Exception.
            with suppress(FileNotFoundError):
                os.unlink(path)


def _owner_only(path, flags):
    return os.open(path, flags | os.O_NOFOLLOW, 0o600)


def for_child(handle) -> tuple[str, tuple[int, ...]]:
    """What a child process opens to reach `handle`, and the descriptors it
    must inherit for that: the memfd through /proc, or the file by its path."""
    if isinstance(handle.name, str):
        return handle.name, ()
    descriptor = handle.fileno()
    return f"/proc/self/fd/{descriptor}", (descriptor,)
