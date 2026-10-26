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
            return "no puede ser la carpeta personal del usuario del hub ni una de sus antecesoras"
        for installed in installed_paths():
            if path == installed or installed in path.parents or path in installed.parents:
                return "se solapa con el código instalado del hub o su entorno de Python"
        root = self.root.resolve()
        if path == root or path in root.parents:
            return "incluye la raíz reservada para las carpetas de los asistentes"
        if any(
            path == root or root in path.parents or path in root.parents
            for root in self.forbidden()
        ):
            return (
                "se solapa con datos o credenciales protegidos, o con la carpeta de otro asistente"
            )
        return None

    def validate(self, text):
        path = Path(text)
        cause = None
        if not path.is_absolute():
            cause = "debe ser una ruta absoluta del hub"
        elif "\x00" in text or len(text.encode()) > MAX_WORKING_FOLDER_BYTES:
            cause = f"debe ocupar como máximo {MAX_WORKING_FOLDER_BYTES} bytes y no contener NUL"
        else:
            try:
                path = path.resolve(strict=True)
                if not path.is_dir():
                    cause = "no es un directorio del hub"
                else:
                    cause = self.overlap_cause(path)
            except (OSError, RuntimeError, ValueError):
                cause = "no existe o no se puede abrir en el hub"
        if cause:
            raise HubError(
                "invalid_working_folder",
                f"La carpeta de trabajo {cause}",
                http_status=400,
                numbers={"cause": cause},
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
