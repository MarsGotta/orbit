"""Descriptor-relative folder reads: symlinks are never followed, including races."""

import os
import stat
from contextlib import contextmanager

from .models import MAX_FOLDER_DEPTH, MAX_NOTE_BYTES, MAX_SCAN_ENTRIES, refusal
from .paths import checked, visible


class Folder:
    def __init__(self, configuration):
        self.root = configuration.path
        if any(ord(character) < 32 for character in self.root):
            raise refusal(
                "vault_path_refused",
                "The vault path contains control characters.",
                message_code="vault_path_controls",
            )
        if not os.path.isabs(self.root):
            raise refusal("vault_folder_absolute", "Enter an absolute path to a folder on the hub.")

    def close(self):
        pass

    @contextmanager
    def directory(self, path=""):
        """Open every component without following links, pinning each directory."""
        fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        try:
            for part in [p for p in self.root.split("/") if p] + (
                [p for p in path.split("/") if p]
            ):
                if part in {".", ".."}:
                    raise refusal(
                        "vault_path_refused",
                        "The folder cannot contain “.” or “..” components.",
                        message_code="vault_path_components",
                    )
                next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                os.close(fd)
                fd = next_fd
            yield fd
        except FileNotFoundError:
            raise refusal(
                "vault_folder_missing", "The vault folder does not exist on the hub."
            ) from None
        except PermissionError:
            raise refusal(
                "vault_folder_permission",
                "The hub does not have permission to read the vault folder.",
            ) from None
        except OSError:
            raise refusal(
                "vault_path_refused",
                "The folder or note cannot be opened without following symbolic links.",
                message_code="vault_path_symlink",
            ) from None
        finally:
            os.close(fd)

    def catalog(self):
        entries, visited = [], [0]
        with self.directory() as fd:
            self._walk(fd, "", entries, visited)
        return sorted(entries, key=lambda entry: entry["path"])

    def _walk(self, fd, prefix, entries, visited):
        # Directory traversal has a separate descriptor per depth;
        # bound depth as path length, and entries before collecting/sorting.
        with os.scandir(fd) as listing:
            for entry in listing:
                visited[0] += 1
                if visited[0] > MAX_SCAN_ENTRIES:
                    raise refusal(
                        "vault_scan_limit",
                        f"The scan exceeds {MAX_SCAN_ENTRIES} entries.",
                        entries=visited[0],
                        max_entries=MAX_SCAN_ENTRIES,
                    )
                path = prefix + entry.name
                if not visible(path) or entry.is_symlink():
                    continue
                if entry.is_dir(follow_symlinks=False):
                    entries.append({"path": path, "kind": "folder"})
                    if len(path.split("/")) > MAX_FOLDER_DEPTH:
                        raise refusal(
                            "vault_depth_limit",
                            f"The vault exceeds {MAX_FOLDER_DEPTH} folder levels.",
                            max_depth=MAX_FOLDER_DEPTH,
                        )
                    try:
                        child = os.open(
                            entry.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                        )
                    except OSError:
                        raise refusal(
                            "vault_folder_changed",
                            "A folder changed or cannot be read; search again.",
                        ) from None
                    try:
                        self._walk(child, path + "/", entries, visited)
                    finally:
                        os.close(child)
                elif entry.is_file(follow_symlinks=False) and path.lower().endswith(".md"):
                    info = entry.stat(follow_symlinks=False)
                    stamp = [
                        getattr(info, "st_" + name)
                        for name in (
                            "dev",
                            "ino",
                            "size",
                            "mtime_ns",
                            "ctime_ns",
                            "mode",
                            "uid",
                            "gid",
                        )
                    ]
                    entries.append({"path": path, "kind": "note", "stamp": stamp})

    def read(self, path):
        checked(path)
        parent, _, name = path.rpartition("/")
        with self.directory(parent) as fd:
            try:
                note = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
            except FileNotFoundError:
                raise refusal(
                    "vault_note_missing", "The note does not exist in the vault.", 404
                ) from None
            try:
                info = os.fstat(note)
                if not stat.S_ISREG(info.st_mode):
                    raise refusal(
                        "vault_path_refused",
                        "The note must be a regular file in the vault.",
                        message_code="vault_path_regular_file",
                    )
                if info.st_size > MAX_NOTE_BYTES:
                    self._too_large(info.st_size)
                with os.fdopen(note, "rb", closefd=False) as stream:
                    data = stream.read(MAX_NOTE_BYTES + 1)
                if len(data) > MAX_NOTE_BYTES:
                    self._too_large(len(data))
                return data.decode("utf-8")
            except UnicodeError:
                raise refusal("vault_encoding", "The note is not encoded as UTF-8.") from None
            finally:
                os.close(note)

    @staticmethod
    def _too_large(size):
        raise refusal(
            "vault_note_too_large",
            f"The note is {size} bytes; the maximum is {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
