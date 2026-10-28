"""Validate saved working folders; kernel mounts enforce tool isolation."""

from pathlib import Path

from ..errors import HubError
from .jail import hidden_paths, installed_paths, workspace_root
from .sign_in import private_folder

# POSIX PATH_MAX on the supported hub hosts. Bounds configuration storage and
# error frames, not tool output, file sizes or the length of a shell command.
MAX_WORKING_FOLDER_BYTES = 4096


class WorkspacePolicy:
    def __init__(self, settings, agent_id, configs=None):
        self.settings, self.agent_id, self.configs = settings, agent_id, configs
        self.root = workspace_root(settings)
        self.default = self.root / agent_id

    def forbidden(self):
        return hidden_paths(self.settings, self.agent_id, self.configs)

    def overlap_cause(self, path):
        # Never grant the common parent: a later assistant would otherwise
        # acquire a private folder inside an existing writable root.
        home = Path.home().resolve()
        if path == home or path in home.parents:
            return (
                "working_folder_home",
                "cannot be the hub user's home folder or an ancestor of it",
            )
        for installed in installed_paths():
            if path == installed or installed in path.parents or path in installed.parents:
                return (
                    "working_folder_installed",
                    "overlaps installed hub code or its Python environment",
                )
        root = self.root.resolve()
        if path == root or path in root.parents:
            return ("working_folder_reserved", "includes the reserved root for assistant folders")
        if any(
            path == root or root in path.parents or path in root.parents
            for root in self.forbidden()
        ):
            return (
                "working_folder_protected",
                "overlaps protected data or credentials, or another assistant's folder",
            )
        return None

    def validate(self, text):
        path = Path(text)
        cause = None
        if not path.is_absolute():
            cause = ("working_folder_absolute", "must be an absolute hub path")
        elif "\x00" in text or len(text.encode()) > MAX_WORKING_FOLDER_BYTES:
            cause = (
                "working_folder_size",
                f"must use at most {MAX_WORKING_FOLDER_BYTES} bytes and contain no NUL",
            )
        else:
            try:
                path = path.resolve(strict=True)
                if not path.is_dir():
                    cause = ("working_folder_directory", "is not a directory on the hub")
                else:
                    cause = self.overlap_cause(path)
            except (OSError, RuntimeError, ValueError):
                cause = (
                    "working_folder_unavailable",
                    "does not exist or cannot be opened on the hub",
                )
        if cause:
            raise HubError(
                "invalid_working_folder",
                f"The working folder {cause[1]}",
                http_status=400,
                numbers={
                    "cause": cause[1],
                    "cause_code": cause[0],
                    "bytes": len(text.encode()),
                    "max_bytes": MAX_WORKING_FOLDER_BYTES,
                },
            )
        return path

    def configured(self):
        # Reading settings must still work after a directory is moved/deleted,
        # so the person can repair its value from the assistant's page.
        stored = self.configs.working_folder(self.agent_id) if self.configs is not None else None
        return stored or str(self.default.absolute())

    def current(self):
        stored = self.configs.working_folder(self.agent_id) if self.configs is not None else None
        private_folder(self.root)
        if stored is None:
            private_folder(self.default)
        return self.validate(stored or str(self.default))
