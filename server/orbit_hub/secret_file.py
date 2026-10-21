"""Bounded reads of private files, without following links or blocking on FIFOs."""

import os
import stat


def read_secret(path, size: int) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise ValueError("Secret file must be a regular file owned by this user")
        if info.st_mode & 0o077:
            raise ValueError("Secret file permissions must be 0600 or more restrictive")
        if info.st_size > size:
            raise ValueError("Secret file exceeds its size limit")
        with os.fdopen(descriptor, "rb", closefd=False) as handle:
            value = handle.read(size + 1)
        if len(value) > size:
            raise ValueError("Secret file exceeds its size limit")
        return value
    finally:
        os.close(descriptor)
