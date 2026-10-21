"""Validated local configuration for the Orbit MCP translator."""

from __future__ import annotations

import os
import re
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path

from orbit_hub.log import get_logger
from orbit_hub.private_auth import read_key
from orbit_hub.urls import protected_url

logger = get_logger("orbit_hub.orbit_mcp.config")

DEFAULT_CONFIG_PATH = "~/.config/orbit/mcp.toml"
KEY_ID = re.compile(r"^[A-Za-z0-9_-]{1,64}$")


class ConfigError(ValueError):
    """A safe startup message which never includes a secret path or value."""


def _field_errors(values: dict, required: set[str], allowed: set[str], prefix: str) -> list[str]:
    errors = []
    for names, cause in (
        (required - values.keys(), "missing"),
        (values.keys() - allowed, "unknown"),
    ):
        if names:
            fields = ", ".join(prefix + name for name in sorted(names))
            errors.append(f"{cause} fields ({len(names)}): {fields}")
    return errors


class _KeyFailure(Exception):
    def __init__(self, _status: int, _code: str):
        super().__init__(
            "The key is not valid: it must be a regular file owned by this user, not a "
            "symbolic link, with mode 0600 and a length of 32 bytes"
        )


@dataclass(frozen=True, repr=False)
class Credentials:
    key_id: str
    key: bytes


@dataclass(frozen=True, repr=False)
class Config:
    hub_url: str
    device_bridge: Credentials
    meetings: Credentials | None
    default_device: str
    devices: dict[str, str]
    source_path: Path | None = field(default=None, repr=False, compare=False)
    source_mtime_ns: int | None = field(default=None, repr=False, compare=False)

    @classmethod
    def from_env(cls) -> Config:
        configured = os.environ.get("ORBIT_MCP_CONFIG", DEFAULT_CONFIG_PATH)
        return cls.load(Path(configured).expanduser())

    @classmethod
    def load(cls, path: str | Path) -> Config:
        source_path = Path(path).expanduser()
        try:
            with source_path.open("rb") as handle:
                values = tomllib.load(handle)
            source_mtime_ns = source_path.stat().st_mtime_ns
        except (OSError, tomllib.TOMLDecodeError):
            raise ConfigError("Could not read a valid TOML configuration for Orbit MCP") from None
        try:
            return replace(
                cls._validated(values),
                source_path=source_path,
                source_mtime_ns=source_mtime_ns,
            )
        except ConfigError:
            raise
        except (KeyError, TypeError, ValueError):
            raise ConfigError("The Orbit MCP configuration has invalid fields") from None

    def reload_if_changed(self) -> Config:
        if self.source_path is None:
            return self
        try:
            current_mtime_ns = self.source_path.stat().st_mtime_ns
        except OSError:
            raise ConfigError("Could not read the Orbit MCP configuration again") from None
        if current_mtime_ns == self.source_mtime_ns:
            return self
        return type(self).load(self.source_path)

    @classmethod
    def _validated(cls, values: dict) -> Config:
        required = {"hub_url", "device_bridge", "default_device", "devices"}
        errors = _field_errors(values, required, required | {"meetings"}, "")
        credential_fields = {"key_id", "key_file"}
        for section in ("device_bridge", "meetings"):
            if isinstance(values.get(section), dict):
                errors.extend(
                    _field_errors(
                        values[section], credential_fields, credential_fields, section + "."
                    )
                )
        if errors:
            raise ConfigError("The Orbit MCP configuration has " + "; ".join(errors))
        hub_url = cls._hub_url(values["hub_url"])
        bridge = cls._credentials(values["device_bridge"])
        meetings = cls._credentials(values["meetings"]) if "meetings" in values else None
        devices = cls._devices(values["devices"])
        default_device = values["default_device"]
        if not cls._valid_device_name(default_device):
            raise ConfigError("default_device must name a valid device")
        return cls(
            hub_url=hub_url,
            device_bridge=bridge,
            meetings=meetings,
            default_device=default_device,
            devices=devices,
        )

    @staticmethod
    def _hub_url(value: object) -> str:
        if not isinstance(value, str) or len(value) > 2048:
            raise ConfigError("hub_url is not valid")
        try:
            protected_url(value, allow_insecure_localhost=True)
        except ValueError:
            raise ConfigError(
                "hub_url requires HTTPS except on loopback, without credentials or query"
            ) from None
        return value.rstrip("/")

    @staticmethod
    def _credentials(value: object) -> Credentials:
        if not isinstance(value, dict):
            raise ConfigError("The credentials configuration is incomplete")
        key_id = value["key_id"]
        key_file = value["key_file"]
        if not isinstance(key_id, str) or KEY_ID.fullmatch(key_id) is None:
            raise ConfigError("key_id is not valid")
        if not isinstance(key_file, str) or not key_file or "\x00" in key_file:
            raise ConfigError("key_file is not valid")
        try:
            key = read_key(Path(key_file).expanduser(), _KeyFailure)
        except _KeyFailure as exc:
            raise ConfigError(str(exc)) from None
        return Credentials(key_id=key_id, key=key)

    @staticmethod
    def _devices(value: object) -> dict[str, str]:
        if not isinstance(value, dict) or len(value) > 128:
            raise ConfigError("[devices] is not a valid table of devices")
        devices: dict[str, str] = {}
        for name, device_id in value.items():
            if (
                not Config._valid_device_name(name)
                or not isinstance(device_id, str)
                or not 1 <= len(device_id) <= 128
            ):
                raise ConfigError("[devices] contains an invalid name or identifier")
            devices[name] = device_id
        return devices

    @staticmethod
    def _valid_device_name(value: object) -> bool:
        return (
            isinstance(value, str)
            and 1 <= len(value) <= 80
            and not any(ord(character) < 32 or ord(character) == 127 for character in value)
        )
