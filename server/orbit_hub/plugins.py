"""The hub's plugins: what each one declares, its routes and its meeting tables.

A plugin is its manifest (`orbit-plugin.json`, `contracts/plugin-v1.md`), a
function that mounts its routes, and the tables of its own keyed by
`(device_id, recording)`, which leave the hub with that meeting. The hub reads
every manifest each time it is built, and once on import, for the assistant's
access groups (`assistant/access.py`). A manifest outside the contract stops the
hub with its file and its cause: the plugins are code of this repository, and a
hub started without one's routes would leave its view broken with no cause.
"""

import json
import re
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from fastapi import Request
from fastapi.responses import JSONResponse

# The longest plugin `id`. It protects the call route and every notice, where
# the `id` travels; the same measure as an agent's `id`, not measured. It does
# not limit a plugin's `name` or the names of its calls.
PLUGIN_ID_MAX_CHARS = 32
API_VERSION = 1
FIELDS = ("id", "version", "name", "orbit_plugin_api", "views", "permissions", "logic")
# A plugin's refusals are `<id>_<cause>`: these would read as the hub's own.
RESERVED_IDS = frozenset({"hub", "plugin"})
RESERVED_PREFIXES = ("hub_", "plugin_")
VIEWS = ("sidebar", "settings", "slots")
PERMISSIONS = frozenset({"data", "notices", "read:meetings", "read:notes", "assistant_tools"})
EVENTS = frozenset({"note.ready", "recording.deleted"})
_ID = re.compile(rf"[a-z0-9_]{{1,{PLUGIN_ID_MAX_CHARS}}}")
_DOMAIN = re.compile(r"[a-z0-9.-]+")


class ManifestError(ValueError):
    """A manifest the hub does not take; the text names its file, or its plugin,
    and the cause.

    A `ValueError`, so `orbit-hub` stops with that one line, as for its configuration.
    """


@dataclass(frozen=True)
class Plugin:
    """One plugin of this hub.

    `mount(app, hub, identity, bounded_body, transport)` mounts its routes;
    `recording_tables` are deleted with a meeting, by device and recording.
    `assistant_tools` is the access group it offers the assistants, if any: the
    group's name and its `orbit-mcp` tools. Offering one needs the
    `assistant_tools` permission in the manifest.
    """

    manifest: dict[str, Any]
    mount: Callable[..., None]
    recording_tables: tuple[str, ...]
    assistant_tools: tuple[str, tuple[str, ...]] | None = None

    def __post_init__(self) -> None:
        if (
            self.assistant_tools is not None
            and "assistant_tools" not in self.manifest["permissions"]
        ):
            raise ManifestError(
                f"plugin {self.id!r} offers assistant tools without the permission assistant_tools"
            )

    @property
    def id(self) -> str:
        return self.manifest["id"]


def load() -> tuple[Plugin, ...]:
    """Every plugin of this hub, ordered by `id`, each manifest read now."""
    # Imported here: each entry is built on this module.
    from .vault.plugin import notes

    return tuple(sorted((notes(),), key=lambda plugin: plugin.id))


def recording_tables(plugins: tuple[Plugin, ...]) -> tuple[str, ...]:
    return tuple(table for plugin in plugins for table in plugin.recording_tables)


def read_manifest(path: Path) -> dict[str, Any]:
    """The manifest in `path`, as written, once it is inside the contract."""
    try:
        with path.open(encoding="utf-8") as file:
            manifest = json.load(file)
    except OSError as error:
        raise ManifestError(f"{path}: cannot be read ({error.strerror or error})") from error
    except ValueError as error:
        raise ManifestError(f"{path}: not JSON ({error})") from error
    cause = manifest_problem(manifest)
    if cause is not None:
        raise ManifestError(f"{path}: {cause}")
    return manifest


def manifest_problem(manifest) -> str | None:
    """The first rule of `contracts/plugin-v1.md` the manifest breaks, if any."""
    if not isinstance(manifest, dict):
        return "the manifest is not a JSON object"
    missing = [field for field in FIELDS if field not in manifest]
    if missing:
        return f"field {missing[0]!r} is missing"
    extra = sorted(set(manifest) - set(FIELDS))
    if extra:
        return f"field {extra[0]!r} is not one of {', '.join(FIELDS)}"
    for check in (_id, _version, _name, _api, _views, _logic, _permissions):
        cause = check(manifest)
        if cause is not None:
            return cause
    return None


def _id(manifest) -> str | None:
    plugin_id = manifest["id"]
    if not isinstance(plugin_id, str) or not _ID.fullmatch(plugin_id):
        return f"id {plugin_id!r} is not {_ID.pattern}"
    if plugin_id in RESERVED_IDS or plugin_id.startswith(RESERVED_PREFIXES):
        return f"id {plugin_id!r} is reserved: never hub or plugin, nor hub_* or plugin_*"
    return None


def _version(manifest) -> str | None:
    version = manifest["version"]
    return None if isinstance(version, str) else f"version {version!r} is not text"


def _name(manifest) -> str | None:
    name = manifest["name"]
    if (
        isinstance(name, dict)
        and set(name) == {"es", "en"}
        and all(isinstance(text, str) for text in name.values())
    ):
        return None
    return f"name {name!r} is not an object of two texts, es and en"


def _api(manifest) -> str | None:
    api = manifest["orbit_plugin_api"]
    if type(api) is int and api == API_VERSION:
        return None
    return f"orbit_plugin_api {api!r} is not the integer {API_VERSION}"


def _views(manifest) -> str | None:
    views = manifest["views"]
    if not isinstance(views, dict):
        return f"views {views!r} is not an object"
    for key in views:
        if key not in VIEWS:
            return f"views has {key!r}, which is not one of {', '.join(VIEWS)}"
    for flag in ("sidebar", "settings"):
        if flag in views and type(views[flag]) is not bool:
            return f"views.{flag} {views[flag]!r} is not a boolean"
    if "slots" in views and views["slots"] != []:
        return f"views.slots {views['slots']!r} is not an empty list: Orbit declares no slots yet"
    return None


def _logic(manifest) -> str | None:
    logic = manifest["logic"]
    if logic == "none":
        return None
    if (
        isinstance(logic, dict)
        and set(logic) == {"command"}
        and isinstance(logic["command"], list)
        and all(isinstance(part, str) for part in logic["command"])
    ):
        return None
    return f'logic {logic!r} is neither "none" nor an object of one field, command, a list of texts'


def _permissions(manifest) -> str | None:
    permissions = manifest["permissions"]
    if not isinstance(permissions, list):
        return f"permissions {permissions!r} is not a list"
    for permission in permissions:
        cause = _permission(permission, manifest["logic"])
        if cause is not None:
            return cause
    return None


def _permission(permission, logic) -> str | None:
    if not isinstance(permission, str):
        return f"permission {permission!r} is not text"
    if permission in PERMISSIONS:
        return None
    kind, colon, value = permission.partition(":")
    if kind == "events" and colon:
        if value in EVENTS:
            return None
        return f"permission {permission!r} names no event of the hub ({', '.join(sorted(EVENTS))})"
    if kind == "network" and colon:
        if not _DOMAIN.fullmatch(value):
            return f"permission {permission!r} names no domain: {value!r} is not {_DOMAIN.pattern}"
        if logic != "none":
            return None
        return f'permission {permission!r} requires logic, and logic is "none"'
    return f"permission {permission!r} is not in the vocabulary"


def mount(app, plugins: tuple[Plugin, ...], identity) -> None:
    """`GET /v1/plugins`: each plugin's manifest as its file says, by `id`."""
    listing = [plugin.manifest for plugin in plugins]

    @app.get("/v1/plugins")
    async def installed(request: Request):
        identity(request)
        return JSONResponse({"plugins": listing})
