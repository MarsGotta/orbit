"""Official assistant executables and their isolated account directories."""

import os
import re
import sys
import sysconfig
from collections.abc import Mapping
from dataclasses import dataclass
from importlib.util import find_spec
from pathlib import Path
from typing import Any

SERVICES = ("claude-code", "codex")
HOME_FOLDERS = {"claude-code": "claude-config", "codex": "codex-home"}
VARIABLE_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
LOCKED_VARIABLES = {"CLAUDE_CONFIG_DIR", "CODEX_HOME", "CODEX_CONFIG", "INITIAL_AGENT_MODE"}
# The same minimal environment used for stdio MCP: user configuration and
# executable lookup, never ambient provider credentials or debug switches.
SAFE_VARIABLES = ("HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER", "LANG", "TMPDIR")


@dataclass(frozen=True)
class ToolSettings:
    agent: str
    command: tuple[str, ...]
    orbit_mcp_config: Path
    env: tuple[str, ...] = ()


def bundled_command(service: str) -> tuple[str, ...]:
    if service == "claude-code":
        package = find_spec("claude_agent_sdk")
        root = (
            Path(package.origin).parent
            if package and package.origin
            else Path(sysconfig.get_path("purelib")) / "claude_agent_sdk"
        )
        return (str(root / "_bundled" / "claude"),)
    # publish.sh places the pinned official binary beside this release's venv.
    # Never search PATH: that could silently select an unvalidated protocol.
    return (str(Path(sys.prefix).parent / "bin" / "codex"),)


def default_tools(data_dir: Path) -> dict[str, ToolSettings]:
    return {
        service: ToolSettings(
            service, bundled_command(service), data_dir.resolve() / "assistant-mcp.toml"
        )
        for service in SERVICES
    }


def parse_tool(service: str, raw: Mapping[str, Any]) -> ToolSettings:
    if unknown := sorted(set(raw) - {"command", "env", "orbit_mcp_config"}):
        raise ValueError(f"unknown keys for {service}: {', '.join(unknown)}")
    command = raw.get("command")
    if not isinstance(command, list) or not command:
        raise ValueError(
            "`command` must be a list with the official executable's absolute path first"
        )
    if any(not isinstance(part, str) or not part for part in command):
        raise ValueError("`command` contains an empty or non-string item")
    if not os.path.isabs(command[0]):
        raise ValueError("`command` must contain an absolute executable path")
    env = raw.get("env", [])
    if not isinstance(env, list) or any(not isinstance(name, str) for name in env):
        raise ValueError("`env` must be a list of variable names")
    for index, name in enumerate(env):
        if not VARIABLE_NAME.fullmatch(name):
            raise ValueError(f"`env` item {name!r} is not a variable name")
        if name in LOCKED_VARIABLES:
            raise ValueError(f"`env` cannot pass {name}; Orbit sets it for the agent")
        if name in env[:index]:
            raise ValueError(f"`env` names {name} twice")
    path = raw.get("orbit_mcp_config")
    if not isinstance(path, str) or not os.path.isabs(path):
        raise ValueError("`orbit_mcp_config` must be an absolute path to the translator's TOML")
    return ToolSettings(service, tuple(command), Path(path), tuple(env))


def parse_assistant_tool(
    service: str, raw: Any, defaults: ToolSettings | None = None
) -> ToolSettings:
    where = f"[assistant_tools.{service}]"
    if service not in SERVICES:
        raise ValueError(f"{where}: unknown service; expected claude-code or codex")
    if not isinstance(raw, Mapping):
        raise ValueError(f"{where} must be a table")  # noqa: TRY004 - configuration refusal
    try:
        if defaults is not None:
            raw = {
                "command": list(defaults.command),
                "orbit_mcp_config": str(defaults.orbit_mcp_config),
                **raw,
            }
        return parse_tool(service, raw)
    except ValueError as exc:
        raise ValueError(f"{where}: {exc}") from None


def parse_jail(raw: Mapping[str, Any]) -> tuple[Path, ...]:
    """Optional host folders; never infer private infrastructure locations."""
    where = "[assistant_tools.jail]"
    if unknown := sorted(set(raw) - {"secret_roots"}):
        raise ValueError(f"{where}: unknown keys: {', '.join(unknown)}")
    roots = raw.get("secret_roots", [])
    if not isinstance(roots, list) or any(
        not isinstance(root, str) or not os.path.isabs(root) for root in roots
    ):
        raise ValueError(f"{where}: `secret_roots` must be a list of absolute folder paths")
    return tuple(dict.fromkeys(Path(root).resolve() for root in roots))


def environment(settings: ToolSettings, home: Path) -> dict[str, str]:
    variables = {
        name: os.environ[name] for name in (*SAFE_VARIABLES, *settings.env) if name in os.environ
    }
    variables["CLAUDE_CONFIG_DIR" if settings.agent == "claude-code" else "CODEX_HOME"] = str(home)
    return variables
