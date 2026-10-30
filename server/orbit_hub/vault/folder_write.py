"""Descriptor-relative atomic replacement, with a fresh hash check before rename."""

import os
from uuid import uuid4

from .models import MAX_FOLDER_DEPTH, MAX_NOTE_BYTES, refusal
from .paths import checked


def _parent(folder, path):
    parts = checked(path).split("/")[:-1]
    if len(parts) > MAX_FOLDER_DEPTH:
        raise refusal(
            "vault_depth_limit",
            f"The path exceeds {MAX_FOLDER_DEPTH} folder levels.",
            max_depth=MAX_FOLDER_DEPTH,
        )
    for index in range(len(parts)):
        parent = "/".join(parts[:index])
        with folder.directory(parent) as fd:
            try:
                os.mkdir(parts[index], mode=0o700, dir_fd=fd)
            except FileExistsError:
                pass
    return "/".join(parts)


def current(folder, path):
    from ..errors import HubError

    try:
        return folder.read(path)
    except HubError as exc:
        if exc.code in {"vault_note_missing", "vault_folder_missing"}:
            return None
        raise


def write(folder, path, text, expected):
    data = text.encode()
    if len(data) > MAX_NOTE_BYTES:
        folder._too_large(len(data))
    parent = _parent(folder, path)
    temporary = ".orbit-" + str(uuid4())
    name = path.rsplit("/", 1)[-1]
    with folder.directory(parent) as fd:
        note = os.open(
            temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd
        )
        try:
            with os.fdopen(note, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            if current(folder, path) != expected:
                raise refusal(
                    "distillation_content_changed", "The note changed before replacement.", 409
                )
            os.replace(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
            os.fsync(fd)
        finally:
            try:
                os.unlink(temporary, dir_fd=fd)
            except FileNotFoundError:
                pass


def remove(folder, path, expected):
    checked(path)
    if current(folder, path) != expected:
        raise refusal("distillation_content_changed", "The note changed before removal.", 409)
    parent, _, name = path.rpartition("/")
    with folder.directory(parent) as fd:
        os.unlink(name, dir_fd=fd)
        os.fsync(fd)
